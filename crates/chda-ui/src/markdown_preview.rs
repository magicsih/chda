//! Offline preview of Markdown files: the file is rendered to an HTML page
//! in the data directory and opened in the default browser, like the
//! diagram viewer. Mermaid blocks become diagrams through the bundled
//! script.
//!
//! The page loads nothing from the network, and the only script it runs is
//! chda's own: a Content Security Policy blocks remote resources, scripts in
//! the document (inline ones and event handler attributes) and other local
//! scripts. Raw HTML in the document is kept, so READMEs that center a logo
//! with `<p align="center">` look as they do on GitHub.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};

use crate::diagram;

/// Larger files are refused: they are not prose meant to be read.
const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Whether `path` names a Markdown file by its extension.
pub fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["md", "markdown", "mdown", "mkd", "mdx"]
            .iter()
            .any(|m| e.eq_ignore_ascii_case(m))
    })
}

/// Render `file` into a page in `dir` and return the page's path.
pub fn write_page(dir: &Path, file: &Path) -> io::Result<PathBuf> {
    let len = fs::metadata(file)?.len();
    if len > MAX_BYTES {
        return Err(io::Error::other(format!(
            "{} is larger than 5 MB",
            file.display()
        )));
    }
    let text = String::from_utf8_lossy(&fs::read(file)?).into_owned();
    let (body, has_mermaid) = render(&text);
    let script = diagram::install_script(dir)?;
    let mut hasher = DefaultHasher::new();
    file.hash(&mut hasher);
    text.hash(&mut hasher);
    let page = dir.join(format!("preview-{:016x}.html", hasher.finish()));
    let title = file
        .file_name()
        .map_or_else(|| file.to_string_lossy(), |n| n.to_string_lossy());
    let base = file.parent().unwrap_or(Path::new("/"));
    fs::write(
        &page,
        page_html(&title, base, &body, has_mermaid.then_some(&*script)),
    )?;
    diagram::prune(dir, &page);
    Ok(page)
}

/// HTML for Markdown text, GitHub flavored (tables, task lists,
/// strikethrough, footnotes, alerts); whether it has Mermaid blocks.
fn render(markdown: &str) -> (String, bool) {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let mut in_mermaid = false;
    let mut has_mermaid = false;
    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang)))
            if lang.trim().eq_ignore_ascii_case("mermaid") =>
        {
            in_mermaid = true;
            has_mermaid = true;
            Event::Html(r#"<pre class="mermaid">"#.into())
        }
        Event::End(TagEnd::CodeBlock) if in_mermaid => {
            in_mermaid = false;
            Event::Html("</pre>".into())
        }
        other => other,
    });
    let mut out = String::with_capacity(markdown.len() * 3 / 2);
    html::push_html(&mut out, events);
    (out, has_mermaid)
}

fn page_html(title: &str, base: &Path, body: &str, mermaid: Option<&Path>) -> String {
    // A fresh nonce per page, so script tags in the document cannot claim it.
    let nonce = format!("{:016x}", RandomState::new().hash_one(title));
    let base_url = file_url(base, true);
    // Browsers ignore `file:` URLs as script sources in a policy, so the
    // bundled script is allowed through the same nonce.
    let scripts = match mermaid {
        Some(script) => format!(
            r#"<script nonce="{nonce}" src="{url}"></script>
<script nonce="{nonce}">
  const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  mermaid.initialize({{ startOnLoad: true, securityLevel: "strict", theme: dark ? "dark" : "default" }});
</script>"#,
            url = file_url(script, false)
        ),
        None => String::new(),
    };
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src file: data:; style-src 'unsafe-inline'; font-src file: data:; script-src 'nonce-{nonce}'; base-uri file:; form-action 'none'">
<base href="{base_url}">
<title>{title}</title>
<style>
  :root {{ color-scheme: light dark; --line: #d0d7de; --muted: #59636e; --code: #f6f8fa; }}
  @media (prefers-color-scheme: dark) {{ :root {{ --line: #3d444d; --muted: #9198a1; --code: #151b23; }} }}
  body {{ margin: 0 auto; max-width: 860px; padding: 32px 24px 64px; font: 16px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; }}
  h1, h2 {{ border-bottom: 1px solid var(--line); padding-bottom: .3em; }}
  a {{ color: #0969da; }} @media (prefers-color-scheme: dark) {{ a {{ color: #4493f8; }} }}
  code, pre {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 85%; }}
  code {{ background: var(--code); padding: .2em .4em; border-radius: 6px; }}
  pre {{ background: var(--code); padding: 16px; border-radius: 6px; overflow: auto; }}
  pre code {{ background: none; padding: 0; }}
  pre.mermaid {{ background: none; display: flex; justify-content: center; }}
  table {{ border-collapse: collapse; display: block; overflow: auto; }}
  th, td {{ border: 1px solid var(--line); padding: 6px 13px; }}
  blockquote {{ margin: 0; padding: 0 1em; color: var(--muted); border-left: .25em solid var(--line); }}
  img {{ max-width: 100%; }}
  hr {{ border: 0; border-top: 1px solid var(--line); }}
  .file {{ color: var(--muted); font-size: 13px; margin-bottom: 24px; }}
</style>
</head>
<body>
<div class="file">{path}</div>
{body}
{scripts}
</body>
</html>
"#,
        title = diagram::escape(title),
        path = diagram::escape(&base.join(title).to_string_lossy()),
    )
}

/// A `file://` URL for `path`, percent-encoded; directories end in `/`.
fn file_url(path: &Path, dir: bool) -> String {
    let mut url = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                url.push(b as char)
            }
            _ => url.push_str(&format!("%{b:02X}")),
        }
    }
    if dir && !url.ends_with('/') {
        url.push('/');
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_files_by_extension() {
        assert!(is_markdown(Path::new("/w/README.md")));
        assert!(is_markdown(Path::new("notes.MARKDOWN")));
        assert!(!is_markdown(Path::new("/w/main.rs")));
        assert!(!is_markdown(Path::new("/w/md")));
    }

    #[test]
    fn github_flavored_markdown_and_mermaid_blocks() {
        let (html, mermaid) = render(
            "# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```rust\nfn main() {}\n```\n",
        );
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<table>"));
        assert!(html.contains(r#"type="checkbox""#));
        assert!(html.contains("<pre class=\"mermaid\">flowchart LR\n  A --&gt; B\n</pre>"));
        assert!(html.contains(r#"<code class="language-rust">"#));
        assert!(mermaid);
        assert!(!render("plain *text*").1);
    }

    #[test]
    fn page_runs_only_chdas_script_and_loads_nothing_remote() {
        let dir = std::env::temp_dir().join(format!("chda-md-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let docs = dir.join("My Docs");
        fs::create_dir_all(&docs).unwrap();
        let file = docs.join("README.md");
        fs::write(
            &file,
            "<p align=\"center\"><img src=\"logo.png\"></p>\n\n<script>alert(1)</script>\n\n```mermaid\ngraph TD\n  a-->b\n```\n",
        )
        .unwrap();
        let page = write_page(&dir.join("viewer"), &file).unwrap();
        let html = fs::read_to_string(&page).unwrap();
        // Relative images resolve next to the file.
        assert!(html.contains(r#"<base href="file://"#) && html.contains("/My%20Docs/\">"));
        // Raw HTML stays; the policy blocks its scripts and anything remote.
        assert!(html.contains(r#"<p align="center">"#));
        assert!(html.contains("default-src 'none'"));
        let policy = html
            .split("script-src")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        assert!(policy.starts_with(" 'nonce-") && !policy.contains("unsafe-inline"));
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "no network resources"
        );
        // The Mermaid script is referenced by its absolute URL.
        assert!(html.contains(&format!("/viewer/{}\"></script>", diagram::MERMAID_FILE)));
        // Only chda's own script tags carry the nonce.
        let nonce = policy
            .trim()
            .trim_start_matches("'nonce-")
            .trim_end_matches('\'');
        assert_eq!(html.matches(&format!("nonce=\"{nonce}\"")).count(), 2);
        // Rendering the same file again reuses the page.
        assert_eq!(write_page(&dir.join("viewer"), &file).unwrap(), page);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn large_files_are_refused() {
        let dir = std::env::temp_dir().join(format!("chda-md-big-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.md");
        fs::write(&file, vec![b'a'; (MAX_BYTES + 1) as usize]).unwrap();
        assert!(write_page(&dir.join("viewer"), &file).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
