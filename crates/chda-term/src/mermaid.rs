//! Mermaid diagram sources in terminal output.
//!
//! Agents print diagrams either as fenced Markdown (```` ```mermaid ````) or,
//! when their UI renders the Markdown, as the bare source: a line starting
//! with a diagram keyword (`flowchart LR`, `sequenceDiagram`, ...), up to the
//! next blank line. Both may be indented and the first line may carry the
//! agent's bullet (`⏺`, `●`, `•`).

use crate::frame::Frame;

/// A diagram found in the viewport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagram {
    /// First and last viewport rows of the block, fences included.
    pub first_row: u16,
    pub last_row: u16,
    /// The Mermaid source, without fences and common indentation.
    pub source: String,
}

/// Keywords that open a diagram when they stand alone on a line.
const HEADERS: &[&str] = &[
    "sequenceDiagram",
    "classDiagram",
    "classDiagram-v2",
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "journey",
    "gantt",
    "pie",
    "quadrantChart",
    "requirementDiagram",
    "gitGraph",
    "C4Context",
    "C4Container",
    "C4Component",
    "C4Dynamic",
    "C4Deployment",
    "mindmap",
    "timeline",
    "zenuml",
    "sankey",
    "sankey-beta",
    "xychart",
    "xychart-beta",
    "block",
    "block-beta",
    "packet",
    "packet-beta",
    "kanban",
    "architecture",
    "architecture-beta",
    "radar-beta",
    "treemap",
    "treemap-beta",
];

/// Leading marks agents put before the first line of an answer.
const BULLETS: &[char] = &['⏺', '●', '•', '◦', '▸', '│', '┃', '>'];

/// Strip indentation and an agent bullet; returns the column the content
/// starts at (in characters) and the content.
fn content(line: &str) -> (usize, &str) {
    let trimmed = line.trim_start();
    let mut col = line.len() - trimmed.len();
    let mut rest = trimmed;
    if let Some(c) = rest.chars().next()
        && BULLETS.contains(&c)
    {
        let after = &rest[c.len_utf8()..];
        let inner = after.trim_start();
        if inner.len() < after.len() {
            col += 1 + (after.len() - inner.len());
            rest = inner;
        }
    }
    (col, rest.trim_end())
}

/// Whether a line opens an unfenced diagram.
fn is_header(text: &str) -> bool {
    let mut words = text.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let rest: Vec<&str> = words.collect();
    match first {
        // `graph TD`, `flowchart LR`, or the keyword alone.
        "graph" | "flowchart" | "flowchart-elk" => match rest.as_slice() {
            [] => true,
            [dir] => matches!(*dir, "TB" | "TD" | "BT" | "RL" | "LR" | "TD;" | "LR;"),
            _ => false,
        },
        "pie" => rest.is_empty() || matches!(rest[0], "title" | "showData"),
        "gantt" | "journey" | "timeline" | "mindmap" | "kanban" => {
            rest.is_empty() || rest[0] == "title"
        }
        _ => rest.is_empty() && HEADERS.contains(&first),
    }
}

fn is_fence_open(text: &str) -> bool {
    ["```", "~~~"].iter().any(|f| {
        text.strip_prefix(f)
            .is_some_and(|rest| rest.trim().eq_ignore_ascii_case("mermaid"))
    })
}

fn is_fence_close(text: &str) -> bool {
    text == "```" || text == "~~~"
}

/// Remove `indent` characters of leading whitespace (or the bullet on the
/// first line) from each line.
fn dedent(lines: &[&str], first_col: usize) -> String {
    let body_indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let text = if i == 0 {
            content(line).1
        } else {
            let strip = body_indent
                .min(first_col)
                .min(line.len() - line.trim_start().len());
            line[strip..].trim_end()
        };
        out.push_str(text);
        out.push('\n');
    }
    out
}

/// Diagrams in a list of logical lines, as `(first line, last line,
/// source)`. A fenced block needs its closing fence; an unfenced one ends
/// before the next blank line.
fn find(lines: &[&str]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let (col, text) = content(lines[i]);
        if is_fence_open(text) {
            let close = (i + 1..lines.len()).find(|&j| is_fence_close(content(lines[j]).1));
            if let Some(close) = close {
                let body = &lines[i + 1..close];
                if body.iter().any(|l| !l.trim().is_empty()) {
                    let indent = body
                        .iter()
                        .filter(|l| !l.trim().is_empty())
                        .map(|l| l.len() - l.trim_start().len())
                        .min()
                        .unwrap_or(0)
                        .min(col);
                    let source: String = body
                        .iter()
                        .map(|l| {
                            let strip = indent.min(l.len() - l.trim_start().len());
                            format!("{}\n", l[strip..].trim_end())
                        })
                        .collect();
                    out.push((i, close, source));
                }
                i = close + 1;
                continue;
            }
            // Still printing, or its end is out of view.
            break;
        } else if is_header(text) {
            let end = (i + 1..lines.len())
                .find(|&j| lines[j].trim().is_empty() || is_fence_close(content(lines[j]).1))
                .unwrap_or(lines.len());
            if end > i + 1 {
                out.push((i, end - 1, dedent(&lines[i..end], col)));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Diagrams whose whole block is in the viewport. Soft-wrapped rows are
/// joined into one line first.
pub fn diagrams(frame: &Frame) -> Vec<Diagram> {
    // Logical lines and the viewport rows each one spans.
    let mut lines: Vec<(u16, u16, String)> = Vec::new();
    let mut joining = false;
    for y in 0..frame.size.rows {
        let text = row_text_untrimmed(frame, y);
        match (joining, lines.last_mut()) {
            (true, Some(last)) => {
                last.1 = y;
                last.2.push_str(&text);
            }
            _ => lines.push((y, y, text)),
        }
        joining = frame.rows.get(usize::from(y)).is_some_and(|r| r.wrapped);
    }
    for l in &mut lines {
        l.2.truncate(l.2.trim_end().len());
    }
    let texts: Vec<&str> = lines.iter().map(|l| l.2.as_str()).collect();
    find(&texts)
        .into_iter()
        .map(|(first, last, source)| Diagram {
            first_row: lines[first].0,
            last_row: lines[last].1,
            source,
        })
        .collect()
}

/// The last diagram in plain text, one line per row (the scrollback).
pub fn last_diagram(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    find(&lines).pop().map(|(_, _, source)| source)
}

/// A row's text with trailing blanks kept, so soft-wrapped rows join at
/// the right column.
fn row_text_untrimmed(frame: &Frame, y: u16) -> String {
    let mut s = String::new();
    for cell in frame.row_cells(y) {
        match cell.width {
            crate::frame::CellWidth::SpacerTail | crate::frame::CellWidth::SpacerHead => {}
            _ if cell.has_text() => s.push_str(frame.cell_text(cell)),
            _ => s.push(' '),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_blocks_lose_fences_and_indentation() {
        let text = "Here it is:\n\n  ```mermaid\n  flowchart LR\n    A --> B\n\n    B --> C\n  ```\n\ndone";
        assert_eq!(
            last_diagram(text).as_deref(),
            Some("flowchart LR\n  A --> B\n\n  B --> C\n")
        );
    }

    #[test]
    fn unfenced_blocks_end_at_a_blank_line() {
        // Claude Code shows rendered Markdown: a bullet, then the source.
        let text =
            "⏺ sequenceDiagram\n    Alice->>Bob: Hi\n    Bob-->>Alice: Hello\n\n  Want more?";
        assert_eq!(
            last_diagram(text).as_deref(),
            Some("sequenceDiagram\n  Alice->>Bob: Hi\n  Bob-->>Alice: Hello\n")
        );
        let text = "  graph TD\n    a-->b\n";
        assert_eq!(last_diagram(text).as_deref(), Some("graph TD\n  a-->b\n"));
    }

    #[test]
    fn prose_is_not_a_diagram() {
        for text in [
            "the graph of commits\nshows a merge",
            "pie chart below\nfoo",
            "flowchart LR",
            "```mermaid\nflowchart LR\n  a-->b",
            "timeline of events\nmore",
        ] {
            assert_eq!(last_diagram(text), None, "{text}");
        }
    }

    #[test]
    fn the_last_of_several_wins() {
        let text = "pie title Pets\n  \"Dogs\" : 3\n\nerDiagram\n  A ||--o{ B : has\n";
        assert_eq!(
            last_diagram(text).as_deref(),
            Some("erDiagram\n  A ||--o{ B : has\n")
        );
    }

    #[test]
    fn viewport_rows_are_reported() {
        let mut t = crate::Terminal::new(crate::Size { cols: 30, rows: 8 }, 10_000).unwrap();
        t.feed(b"intro\r\n```mermaid\r\nflowchart LR\r\n  A --> B\r\n```\r\nafter");
        let d = diagrams(&t.frame().unwrap());
        assert_eq!(
            d,
            [Diagram {
                first_row: 1,
                last_row: 4,
                source: "flowchart LR\n  A --> B\n".into()
            }]
        );
    }
}
