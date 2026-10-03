//! Offline viewer for Mermaid diagrams: an HTML page next to a bundled
//! copy of Mermaid, opened in the default browser. Nothing leaves the
//! machine; diagrams may contain private code.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

/// Mermaid 12.1.0 (MIT), `dist/mermaid.min.js` from npm. License in
/// `assets/mermaid/LICENSE`.
const MERMAID_JS: &[u8] = include_bytes!("../assets/mermaid/mermaid.min.js");
pub(crate) const MERMAID_FILE: &str = "mermaid-12.1.0.min.js";

/// Pages kept in the directory; older ones are removed.
const KEEP_PAGES: usize = 50;

/// Write a page that renders `source` into `dir` (with the Mermaid script
/// beside it) and return its path.
pub fn write_page(dir: &Path, source: &str) -> io::Result<PathBuf> {
    install_script(dir)?;
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let page = dir.join(format!("diagram-{:016x}.html", hasher.finish()));
    fs::write(&page, html(source))?;
    prune(dir, &page);
    Ok(page)
}

/// Put the bundled Mermaid script into `dir` unless it is already there;
/// returns its path. Diagram pages and Markdown previews share it.
pub(crate) fn install_script(dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let script = dir.join(MERMAID_FILE);
    if fs::metadata(&script).map(|m| m.len()).ok() != Some(MERMAID_JS.len() as u64) {
        let tmp = dir.join(format!("{MERMAID_FILE}.tmp"));
        fs::write(&tmp, MERMAID_JS)?;
        fs::rename(&tmp, &script)?;
    }
    Ok(script)
}

fn html(source: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Diagram</title>
<style>
  :root {{ color-scheme: light dark; }}
  body {{ margin: 0; padding: 24px; font-family: -apple-system, system-ui, sans-serif; }}
  .mermaid {{ display: flex; justify-content: center; }}
  details {{ margin-top: 24px; opacity: 0.8; }}
  pre.source {{ white-space: pre-wrap; font-family: ui-monospace, monospace; font-size: 12px; }}
</style>
</head>
<body>
<pre class="mermaid">{escaped}</pre>
<details><summary>Source</summary><pre class="source">{escaped}</pre></details>
<script src="{MERMAID_FILE}"></script>
<script>
  const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  mermaid.initialize({{ startOnLoad: true, securityLevel: "strict", theme: dark ? "dark" : "default" }});
</script>
</body>
</html>
"#,
        escaped = escape(source)
    )
}

pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Remove all but the newest pages.
pub(crate) fn prune(dir: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut pages: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p != keep && p.extension().is_some_and(|x| x == "html"))
        .filter_map(|p| Some((fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    pages.sort();
    let excess = (pages.len() + 1).saturating_sub(KEEP_PAGES);
    for (_, p) in pages.into_iter().take(excess) {
        let _ = fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_embeds_escaped_source_and_the_local_script() {
        let dir = std::env::temp_dir().join(format!("chda-diagram-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let page = write_page(&dir, "flowchart LR\n  A -->|x < y| B\n").unwrap();
        let html = fs::read_to_string(&page).unwrap();
        assert!(html.contains("A --&gt;|x &lt; y| B"));
        assert!(html.contains(&format!(r#"<script src="{MERMAID_FILE}">"#)));
        assert!(!html.contains("http"), "no network resources");
        assert_eq!(
            fs::metadata(dir.join(MERMAID_FILE)).unwrap().len(),
            MERMAID_JS.len() as u64
        );
        // The same source maps to the same page.
        assert_eq!(
            write_page(&dir, "flowchart LR\n  A -->|x < y| B\n").unwrap(),
            page
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
