//! Links under the mouse: OSC 8 hyperlinks, plain URLs and file paths
//! (optionally with `:line[:column]`) in the visible rows. Soft-wrapped rows
//! are joined, so a long URL that wraps is found as a whole.

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

/// Runs of characters that can make up a path: no blanks, quotes or brackets.
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[^\s"'`<>()\[\]{}|,;]+"#).expect("valid token pattern"));

static LINE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?):(\d+)(?::(\d+))?:?$").expect("valid suffix pattern"));

/// The link covering cell `(x, y)` of the viewport, if any.
pub fn link_at(frame: &Frame, x: u16, y: u16) -> Option<Link> {
    if let Some(h) = frame
        .hyperlinks
        .iter()
        .find(|h| h.row == y && (h.start..=h.end).contains(&x))
    {
        return Some(Link {
            target: LinkTarget::Url(h.uri.clone()),
            cells: vec![(h.row, h.start, h.end)],
        });
    }
    let line = LogicalLine::around(frame, y)?;
    let byte = line.byte_at(y, x)?;
    if let Some(m) = URL
        .find_iter(&line.text)
        .find(|m| m.range().contains(&byte))
    {
        let url = trim_url(m.as_str());
        if m.start() + url.len() > byte {
            return Some(Link {
                cells: line.cells(m.start(), m.start() + url.len()),
                target: LinkTarget::Url(url.to_owned()),
            });
        }
    }
    let m = TOKEN
        .find_iter(&line.text)
        .find(|m| m.range().contains(&byte))?;
    let token = m.as_str().trim_end_matches(['.', ',', ':', ';', '!', '?']);
    let (path, line_no, column) = match LINE_SUFFIX.captures(token) {
        Some(c) => (
            c.get(1)?.as_str(),
            c.get(2).and_then(|n| n.as_str().parse().ok()),
            c.get(3).and_then(|n| n.as_str().parse().ok()),
        ),
        None => (token, None, None),
    };
    if !looks_like_path(path, line_no.is_some()) || m.start() + token.len() <= byte {
        return None;
    }
    Some(Link {
        cells: line.cells(m.start(), m.start() + token.len()),
        target: LinkTarget::Path {
            path: path.to_owned(),
            line: line_no,
            column,
        },
    })
}

/// A path has a separator, or is a file name with an extension followed by
/// a line number (`main.rs:12`). Bare words are never paths.
fn looks_like_path(s: &str, has_line: bool) -> bool {
    if s.contains("://") || s.chars().all(|c| c == '.' || c == '/' || c == '~') {
        return false;
    }
    let has_extension = s
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.chars().all(char::is_alphanumeric));
    s.contains('/') || (has_line && has_extension)
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
            link_at(&f, 31, 0).is_none(),
            "the closing paren is not the link"
        );
        assert!(link_at(&f, 0, 0).is_none());
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
        assert!(link_at(&f, 1, 3).is_none());
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
        assert!(link_at(&f, 15, 1).is_none(), "bare words are not paths");
        assert!(link_at(&f, 0, 0).is_none());
        assert_eq!(link_at(&f, 3, 2).unwrap().cells, vec![(2, 0, 9)]);
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
