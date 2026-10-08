//! Links under the mouse: OSC 8 hyperlinks, plain URLs and file paths
//! (optionally with `:line[:column]`) in the visible rows. Soft-wrapped rows
//! are joined, so a long URL that wraps is found as a whole.
//!
//! Paths may contain spaces when parenthesized, quoted (`"My Notes/todo.md"`) or escaped
//! with a backslash (`My\ Notes/todo.md`); `file://` URLs are paths too.
//! Any word can name a file (`ls` prints bare names), so paths are only
//! candidates: the caller keeps the first that exists.

use std::sync::LazyLock;

use regex::Regex;

use crate::frame::{CellWidth, Frame};

/// An OSC 8 hyperlink: cells `start..=end` of viewport row `row`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub row: u16,
    pub start: u16,
    pub end: u16,
    pub uri: String,
}

/// What a link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Url(String),
    /// A path as printed, possibly relative or starting with `~`; the caller
    /// resolves it against the pane's directory and checks it exists.
    Path {
        path: String,
        line: Option<u32>,
        column: Option<u32>,
    },
}

/// A link and the cells it covers, as `(row, start, end)` with `end`
/// inclusive, one entry per viewport row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: LinkTarget,
    pub cells: Vec<(u16, u16, u16)>,
}

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:https?|file|ftp)://[^\s<>"'`]+"#).expect("valid URL pattern")
});

/// Runs of characters that can make up a path: no blanks, quotes or
/// brackets, except a backslash-escaped space.
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:\\ |[^\s"'`<>()\[\]{}|,;])+"#).expect("valid token pattern"));

/// A single- or double-quoted string on one logical line.
static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]+)"|'([^']+)'"#).expect("valid quote pattern"));

static LINE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?):(\d+)(?::(\d+))?:?$").expect("valid suffix pattern"));

/// Links covering cell `(x, y)` of the viewport, most specific first: an
/// OSC 8 hyperlink, a URL, a quoted or parenthesized path, then the plain token. Paths are
/// only candidates; the caller keeps the first that exists.
pub fn links_at(frame: &Frame, x: u16, y: u16) -> Vec<Link> {
    if let Some(h) = frame
        .hyperlinks
        .iter()
        .find(|h| h.row == y && (h.start..=h.end).contains(&x))
    {
        return vec![Link {
            target: url_target(&h.uri),
            cells: vec![(h.row, h.start, h.end)],
        }];
    }
    let Some(line) = LogicalLine::around(frame, y) else {
        return Vec::new();
    };
    let Some(byte) = line.byte_at(y, x) else {
        return Vec::new();
    };
    if let Some(m) = URL
        .find_iter(&line.text)
        .find(|m| m.range().contains(&byte))
    {
        let url = trim_url(m.as_str());
        if m.start() + url.len() > byte {
            return vec![Link {
                cells: line.cells(m.start(), m.start() + url.len()),
                target: url_target(url),
            }];
        }
    }
    let mut out = Vec::new();
    if let Some((range, inner)) = QUOTED.captures_iter(&line.text).find_map(|c| {
        let inner = c.get(1).or_else(|| c.get(2))?;
        c.get(0)?
            .range()
            .contains(&byte)
            .then_some((inner.range(), inner.as_str()))
    }) && inner.trim() == inner
        && !inner.contains(['"', '\''])
        && let Some(target) = path_target(inner)
    {
        out.push(Link {
            cells: line.cells(range.start, range.end),
            target,
        });
    }
    // Balanced outer bounds retain parentheses in filenames. Prefer the
    // complete bounded candidate over a shorter token that also exists.
    let mut stack = Vec::new();
    let mut bounded = Vec::new();
    for (offset, ch) in line.text.char_indices() {
        match ch {
            '(' => stack.push(offset + 1),
            ')' => {
                if let Some(start) = stack.pop()
                    && (start..offset).contains(&byte)
                {
                    bounded.push(start..offset);
                }
            }
            _ => {}
        }
    }
    bounded.sort_by_key(|range| std::cmp::Reverse(range.len()));
    for range in bounded {
        let inner = &line.text[range.clone()];
        if inner.trim() == inner
            && !inner.contains(['"', '\''])
            && let Some(target) = path_target(&inner.replace("\\ ", " "))
        {
            out.push(Link {
                cells: line.cells(range.start, range.end),
                target,
            });
        }
    }
    if let Some(m) = TOKEN
        .find_iter(&line.text)
        .find(|m| m.range().contains(&byte))
    {
        let token = m.as_str().trim_end_matches(['.', ',', ':', ';', '!', '?']);
        if m.start() + token.len() > byte
            && let Some(target) = path_target(&token.replace("\\ ", " "))
        {
            out.push(Link {
                cells: line.cells(m.start(), m.start() + token.len()),
                target,
            });
        }
    }
    out
}

/// A path with an optional `:line[:column]` suffix.
fn path_target(text: &str) -> Option<LinkTarget> {
    let (path, line, column) = match LINE_SUFFIX.captures(text) {
        Some(c) => (
            c.get(1)?.as_str(),
            c.get(2).and_then(|n| n.as_str().parse().ok()),
            c.get(3).and_then(|n| n.as_str().parse().ok()),
        ),
        None => (text, None, None),
    };
    could_be_path(path).then(|| LinkTarget::Path {
        path: path.to_owned(),
        line,
        column,
    })
}

/// `file://` URLs are local paths (`file:///a%20b`, `file://host/a`);
/// everything else stays a URL.
fn url_target(url: &str) -> LinkTarget {
    file_url_path(url).map_or_else(
        || LinkTarget::Url(url.to_owned()),
        |path| LinkTarget::Path {
            path,
            line: None,
            column: None,
        },
    )
}

/// The decoded path of a `file://` URL. The host part (empty, `localhost`
/// or this machine's name, as `ls --hyperlink` writes) is dropped.
fn file_url_path(url: &str) -> Option<String> {
    let rest = url
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("file://"))
        .map(|_| &url[7..])?;
    let path = &rest[rest.find('/')?..];
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&h), Some(&l)) if hex(h).is_some() && hex(l).is_some() => {
                out.push((hex(h)? * 16 + hex(l)?) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Anything but URLs, command-line options and runs of `.`, `/` and `~`.
fn could_be_path(s: &str) -> bool {
    !s.contains("://")
        && !s.starts_with('-')
        && !s.chars().all(|c| c == '.' || c == '/' || c == '~')
}

/// Drop trailing punctuation that usually ends a sentence, and a closing
/// parenthesis or bracket that has no opening one in the URL.
fn trim_url(url: &str) -> &str {
    let mut url = url;
    loop {
        let Some(last) = url.chars().next_back() else {
            return url;
        };
        let unbalanced = |open: char, close: char| {
            last == close && url.matches(open).count() < url.matches(close).count()
        };
        if matches!(last, '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '"')
            || unbalanced('(', ')')
            || unbalanced('[', ']')
            || unbalanced('{', '}')
        {
            url = &url[..url.len() - last.len_utf8()];
        } else {
            return url;
        }
    }
}

/// Visible rows joined across soft wraps, with each character's cell.
struct LogicalLine {
    text: String,
    /// `(byte offset, row, first col, last col)` per character.
    chars: Vec<(usize, u16, u16, u16)>,
}

impl LogicalLine {
    fn around(frame: &Frame, y: u16) -> Option<Self> {
        let rows = frame.size.rows;
        if y >= rows {
            return None;
        }
        let wrapped = |r: u16| frame.rows.get(usize::from(r)).is_some_and(|r| r.wrapped);
        let mut first = y;
        while first > 0 && wrapped(first - 1) {
            first -= 1;
        }
        let mut last = y;
        while last + 1 < rows && wrapped(last) {
            last += 1;
        }
        let mut line = Self {
            text: String::new(),
            chars: Vec::new(),
        };
        for row in first..=last {
            for (x, cell) in frame.row_cells(row).iter().enumerate() {
                let x = x as u16;
                if matches!(cell.width, CellWidth::SpacerTail | CellWidth::SpacerHead) {
                    continue;
                }
                let end = if cell.width == CellWidth::Wide {
                    x + 1
                } else {
                    x
                };
                let text = if cell.has_text() {
                    frame.cell_text(cell)
                } else {
                    " "
                };
                line.chars.push((line.text.len(), row, x, end));
                line.text.push_str(text);
            }
        }
        Some(line)
    }

    fn byte_at(&self, row: u16, col: u16) -> Option<usize> {
        self.chars
            .iter()
            .find(|c| c.1 == row && (c.2..=c.3).contains(&col))
            .map(|c| c.0)
    }

    /// Cells covered by bytes `start..end`, merged per row.
    fn cells(&self, start: usize, end: usize) -> Vec<(u16, u16, u16)> {
        let mut out: Vec<(u16, u16, u16)> = Vec::new();
        for &(at, row, from, to) in &self.chars {
            if at < start || at >= end {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.0 == row => last.2 = to,
                _ => out.push((row, from, to)),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Cell, Row, Size};

    fn link_at(frame: &Frame, x: u16, y: u16) -> Option<Link> {
        links_at(frame, x, y).into_iter().next()
    }

    fn is_url(link: Option<Link>) -> bool {
        matches!(
            link,
            Some(Link {
                target: LinkTarget::Url(_),
                ..
            })
        )
    }

    /// A frame whose rows hold `lines`, `cols` wide; rows listed in `wrapped`
    /// continue on the next row.
    fn frame(cols: u16, lines: &[&str], wrapped: &[u16]) -> Frame {
        let mut f = Frame {
            size: Size {
                cols,
                rows: lines.len() as u16,
            },
            ..Default::default()
        };
        for (y, line) in lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            for x in 0..cols {
                let mut cell = Cell::default();
                if let Some(c) = chars.get(usize::from(x)) {
                    let (start, len) = f.push_cell_text(&c.to_string());
                    Frame::set_cell_text(&mut cell, start, len);
                }
                f.cells.push(cell);
            }
            f.rows.push(Row {
                wrapped: wrapped.contains(&(y as u16)),
                ..Default::default()
            });
        }
        f
    }

    #[test]
    fn finds_urls_and_trims_punctuation() {
        let f = frame(40, &["see (https://example.com/a_(b)) now."], &[]);
        let link = link_at(&f, 10, 0).unwrap();
        assert_eq!(
            link.target,
            LinkTarget::Url("https://example.com/a_(b)".into())
        );
        assert_eq!(link.cells, vec![(0, 5, 29)]);
        assert!(
            !is_url(link_at(&f, 31, 0)),
            "the closing paren is not the link"
        );
        assert!(!is_url(link_at(&f, 0, 0)));
    }

    #[test]
    fn joins_soft_wrapped_rows() {
        let f = frame(
            10,
            &["PR: https:", "//github.c", "om/x/y/1", "next"],
            &[0, 1],
        );
        let link = link_at(&f, 3, 2).unwrap();
        assert_eq!(
            link.target,
            LinkTarget::Url("https://github.com/x/y/1".into())
        );
        assert_eq!(link.cells, vec![(0, 4, 9), (1, 0, 9), (2, 0, 7)]);
        assert!(!is_url(link_at(&f, 1, 3)));
    }

    #[test]
    fn finds_paths_with_line_numbers() {
        let f = frame(
            60,
            &[
                "error at src/main.rs:12:5: oops",
                "lib.rs:7 and plain words",
                "~/notes.md.",
            ],
            &[],
        );
        assert_eq!(
            link_at(&f, 12, 0).unwrap().target,
            LinkTarget::Path {
                path: "src/main.rs".into(),
                line: Some(12),
                column: Some(5)
            }
        );
        assert_eq!(link_at(&f, 12, 0).unwrap().cells, vec![(0, 9, 24)]);
        assert_eq!(
            link_at(&f, 2, 1).unwrap().target,
            LinkTarget::Path {
                path: "lib.rs".into(),
                line: Some(7),
                column: None
            }
        );
        assert!(
            matches!(link_at(&f, 15, 1).unwrap().target, LinkTarget::Path { path, .. } if path == "plain"),
            "bare words are candidates; the caller checks they exist"
        );
        assert!(link_at(&f, 2, 1).unwrap().cells == vec![(1, 0, 7)]);
        assert_eq!(link_at(&f, 3, 2).unwrap().cells, vec![(2, 0, 9)]);
    }

    #[test]
    fn parenthesized_paths_cover_spaces_unicode_and_suffixes() {
        for path in [
            "/tmp/Notes Folder/workflow.html",
            "작업 문서/설계안.md:12:5",
            "/tmp/My Notes/file (draft).md",
        ] {
            let text = format!("Plan ({path}) next");
            let f = frame(100, &[&text], &[]);
            let expected = path_target(path).unwrap();
            for x in 6..6 + path.chars().count() as u16 {
                let links = links_at(&f, x, 0);
                assert_eq!(
                    links.first().map(|l| &l.target),
                    Some(&expected),
                    "{path} at {x}"
                );
                assert_eq!(
                    links[0].cells,
                    vec![(0, 6, 5 + path.chars().count() as u16)]
                );
            }
        }
    }

    #[test]
    fn parenthesized_path_crosses_only_soft_wraps() {
        let f = frame(
            16,
            &[
                "Plan (/tmp/Notes",
                " Folder/file.md)",
                "Other (a b.md)",
                "Broken (/tmp/A",
                "B/file.md)",
            ],
            &[0],
        );
        for (x, y) in [(8, 0), (1, 1), (10, 1)] {
            assert_eq!(
                links_at(&f, x, y)[0].target,
                path_target("/tmp/Notes Folder/file.md").unwrap()
            );
        }
        assert_eq!(links_at(&f, 9, 2)[0].target, path_target("a b.md").unwrap());
        assert_eq!(
            links_at(&f, 10, 3)[0].target,
            path_target("/tmp/A").unwrap()
        );
        assert_eq!(
            links_at(&f, 3, 4)[0].target,
            path_target("B/file.md").unwrap()
        );
    }

    #[test]
    fn parenthesized_adjacent_paths_and_urls_stay_distinct() {
        let f = frame(100, &["(a b.md) (c d.md) (https://example.com/x)"], &[]);
        assert_eq!(links_at(&f, 2, 0)[0].target, path_target("a b.md").unwrap());
        assert_eq!(
            links_at(&f, 11, 0)[0].target,
            path_target("c d.md").unwrap()
        );
        assert_eq!(
            links_at(&f, 25, 0)[0].target,
            LinkTarget::Url("https://example.com/x".into())
        );
    }

    #[test]
    fn finds_quoted_and_escaped_paths_with_spaces() {
        let f = frame(
            60,
            &[
                r#"open "/tmp/My Notes/todo.md" now"#,
                "cp 'a b/c' x; \"My File.txt\"",
                r"ls My\ Notes/todo.md:3",
                "it's in src/x.rs",
            ],
            &[],
        );
        let quoted = links_at(&f, 12, 0);
        assert_eq!(
            quoted[0].target,
            LinkTarget::Path {
                path: "/tmp/My Notes/todo.md".into(),
                line: None,
                column: None
            }
        );
        assert_eq!(quoted[0].cells, vec![(0, 6, 26)]);
        assert_eq!(
            quoted[1].target,
            LinkTarget::Path {
                path: "/tmp/My".into(),
                line: None,
                column: None
            },
            "the plain token is the fallback"
        );
        assert!(matches!(
            link_at(&f, 5, 1).unwrap().target,
            LinkTarget::Path { path, .. } if path == "a b/c"
        ));
        assert!(matches!(
            link_at(&f, 18, 1).unwrap().target,
            LinkTarget::Path { path, .. } if path == "My File.txt"
        ));
        assert_eq!(
            link_at(&f, 6, 2).unwrap().target,
            LinkTarget::Path {
                path: "My Notes/todo.md".into(),
                line: Some(3),
                column: None
            }
        );
        assert_eq!(link_at(&f, 6, 2).unwrap().cells, vec![(2, 3, 21)]);
        assert!(matches!(
            link_at(&f, 10, 3).unwrap().target,
            LinkTarget::Path { path, .. } if path == "src/x.rs"
        ));
    }

    #[test]
    fn file_urls_are_paths() {
        let f = frame(60, &["see file:///tmp/My%20Notes/a.md."], &[]);
        assert_eq!(
            link_at(&f, 8, 0).unwrap().target,
            LinkTarget::Path {
                path: "/tmp/My Notes/a.md".into(),
                line: None,
                column: None
            }
        );
        let mut f = frame(20, &["build"], &[]);
        f.hyperlinks.push(Hyperlink {
            row: 0,
            start: 0,
            end: 4,
            uri: "file://mac.local/Users/me/build".into(),
        });
        assert!(matches!(
            link_at(&f, 2, 0).unwrap().target,
            LinkTarget::Path { path, .. } if path == "/Users/me/build"
        ));
    }

    #[test]
    fn osc8_hyperlinks_win() {
        let mut f = frame(20, &["click here please"], &[]);
        f.hyperlinks.push(Hyperlink {
            row: 0,
            start: 6,
            end: 9,
            uri: "https://chda.dev".into(),
        });
        let link = link_at(&f, 7, 0).unwrap();
        assert_eq!(link.target, LinkTarget::Url("https://chda.dev".into()));
        assert_eq!(link.cells, vec![(0, 6, 9)]);
    }
}

#[cfg(test)]
mod terminal_wrap_regression {
    use super::*;
    use crate::{Size, Terminal};
    #[test]
    fn wrapped_paths_survive_resize_and_scrollback_without_joining_hard_newlines() {
        let path = "/tmp/prefix/long-directory/another-directory/file.rs:12:5";
        let expected = LinkTarget::Path {
            path: "/tmp/prefix/long-directory/another-directory/file.rs".into(),
            line: Some(12),
            column: Some(5),
        };
        let mut terminal = Terminal::new(Size { cols: 30, rows: 8 }, 1_000_000).unwrap();
        terminal.feed(path.as_bytes());
        let frame = terminal.frame().unwrap();
        assert_eq!(links_at(&frame, 4, 1)[0].target, expected);
        assert_eq!(links_at(&frame, 4, 1)[0].cells.len(), 2);
        terminal.resize(Size { cols: 20, rows: 8 }, 8, 16).unwrap();
        let frame = terminal.frame().unwrap();
        for row in 0..3 {
            assert_eq!(links_at(&frame, 4, row)[0].target, expected);
        }
        terminal.feed(b"\r\nnext\r\nnext\r\nnext\r\nnext\r\nnext\r\nnext\r\nnext");
        terminal.scroll(-100);
        let frame = terminal.frame().unwrap();
        for row in 0..3 {
            assert_eq!(links_at(&frame, 4, row)[0].target, expected);
        }
        let mut separate = Terminal::new(Size { cols: 40, rows: 8 }, 1_000_000).unwrap();
        separate.feed(b"/tmp/one.rs:1\r\n/tmp/two.rs:2");
        let frame = separate.frame().unwrap();
        assert_eq!(
            links_at(&frame, 4, 0)[0].target,
            LinkTarget::Path {
                path: "/tmp/one.rs".into(),
                line: Some(1),
                column: None
            }
        );
        assert_eq!(
            links_at(&frame, 4, 1)[0].target,
            LinkTarget::Path {
                path: "/tmp/two.rs".into(),
                line: Some(2),
                column: None
            }
        );
    }

    #[test]
    fn real_terminal_frames_preserve_complete_wrapped_paths() {
        let path = "/tmp/prefix/long-directory/another-directory/file.rs:12:5";
        let mut terminal = Terminal::new(Size { cols: 20, rows: 8 }, 1_000_000).unwrap();
        terminal.feed(path.as_bytes());
        let frame = terminal.frame().unwrap();
        assert!(frame.rows[0].wrapped, "{:?}", frame.rows);
        for row in 0..3 {
            let links = links_at(&frame, 4, row);
            assert!(links.iter().any(|l| matches!(&l.target, LinkTarget::Path { path, line: Some(12), column: Some(5) } if path == "/tmp/prefix/long-directory/another-directory/file.rs")), "row {row}: {links:?}");
        }
    }
}
