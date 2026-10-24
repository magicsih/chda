//! Scrollback search. Matching runs on the plain text of the whole screen,
//! one line per row, so line `n` is screen row `n` and positions map back to
//! cells exactly. A match does not continue across a soft wrap.

use regex::{Regex, RegexBuilder};

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    /// Treat `text` as a regular expression instead of a literal.
    pub regex: bool,
}

impl SearchQuery {
    pub(crate) fn compile(&self) -> Result<Regex, regex::Error> {
        let pattern = if self.regex {
            self.text.clone()
        } else {
            regex::escape(&self.text)
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .build()
    }
}

/// Outcome of the latest search run, for the search bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchStatus {
    pub total: usize,
    /// Position of the current match counted from the newest one (1-based).
    pub current: Option<usize>,
    /// The regular expression did not compile.
    pub error: Option<String>,
}

/// One match: cells `start..=end` of screen row `row`, where row 0 is the
/// oldest scrollback row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub row: u64,
    pub start: u16,
    pub end: u16,
}

/// How a cell is highlighted by the search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchMark {
    #[default]
    None,
    Match,
    Current,
}

/// Find every match in `text`, the screen as plain text with one line per
/// row and trailing blanks trimmed. `width` gives a character's cell width
/// (0 for combining marks, 2 for wide glyphs). Zero-length matches are
/// skipped.
pub fn find_matches(text: &str, width: impl Fn(char) -> u8, re: &Regex) -> Vec<Match> {
    let mut out = Vec::new();
    for (row, line) in text.split('\n').enumerate() {
        let row = row as u64;
        let mut found = re.find_iter(line).filter(|m| !m.is_empty()).peekable();
        if found.peek().is_none() {
            continue;
        }
        if line.is_ascii() {
            // One cell per byte.
            for m in found {
                out.push(Match {
                    row,
                    start: m.start() as u16,
                    end: (m.end() - 1) as u16,
                });
            }
            continue;
        }
        // Column where each character starts; zero-width characters share
        // the previous character's cell.
        let mut starts: Vec<(usize, u16, u16)> = Vec::with_capacity(line.len());
        let mut col = 0u16;
        for (i, c) in line.char_indices() {
            let w = u16::from(width(c));
            if w == 0 {
                let prev = starts.last().map(|&(_, c, w)| (c, w)).unwrap_or((0, 1));
                starts.push((i, prev.0, prev.1));
            } else {
                starts.push((i, col, w));
                col = col.saturating_add(w);
            }
        }
        let cell = |byte: usize| {
            let i = starts.partition_point(|s| s.0 <= byte).saturating_sub(1);
            starts.get(i).map(|&(_, c, w)| (c, w)).unwrap_or((0, 1))
        };
        for m in found {
            let last = line[..m.end()]
                .char_indices()
                .next_back()
                .map_or(m.start(), |(i, _)| i);
            let (end_col, end_w) = cell(last);
            out.push(Match {
                row,
                start: cell(m.start()).0,
                end: end_col + end_w.max(1) - 1,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn width(c: char) -> u8 {
        match c {
            '\u{0301}' => 0,
            '中' | '文' => 2,
            _ => 1,
        }
    }

    fn query(text: &str) -> Regex {
        SearchQuery {
            text: text.into(),
            ..Default::default()
        }
        .compile()
        .unwrap()
    }

    fn m(row: u64, start: u16, end: u16) -> Match {
        Match { row, start, end }
    }

    #[test]
    fn lines_are_rows_and_bytes_are_cells() {
        let text = "one\nan error here\n\nerror error";
        assert_eq!(
            find_matches(text, width, &query("error")),
            vec![m(1, 3, 7), m(3, 0, 4), m(3, 6, 10)]
        );
    }

    #[test]
    fn wide_and_combining_characters_map_to_cells() {
        // 中 takes cells 0-1, 文 2-3, then "e" + combining accent in cell 4.
        let text = "中文e\u{0301}x";
        assert_eq!(find_matches(text, width, &query("文")), vec![m(0, 2, 3)]);
        assert_eq!(find_matches(text, width, &query("x")), vec![m(0, 5, 5)]);
        assert_eq!(
            find_matches(text, width, &query("e\u{0301}")),
            vec![m(0, 4, 4)]
        );
    }

    #[test]
    fn case_and_regex_options() {
        assert_eq!(find_matches("ERROR error", width, &query("Error")).len(), 2);
        let sensitive = SearchQuery {
            text: "Error".into(),
            case_sensitive: true,
            regex: false,
        }
        .compile()
        .unwrap();
        assert!(find_matches("ERROR error", width, &sensitive).is_empty());
        assert_eq!(find_matches("abc a.c", width, &query("a.c")).len(), 1);
        let re = SearchQuery {
            text: r"e\d+".into(),
            case_sensitive: false,
            regex: true,
        }
        .compile()
        .unwrap();
        let found = find_matches("e1 x e22 E3 e", width, &re);
        assert_eq!(found, vec![m(0, 0, 1), m(0, 5, 7), m(0, 9, 10)]);
        assert!(
            SearchQuery {
                text: "(".into(),
                regex: true,
                ..Default::default()
            }
            .compile()
            .is_err()
        );
        let empty = SearchQuery {
            text: "x*".into(),
            regex: true,
            ..Default::default()
        }
        .compile()
        .unwrap();
        assert!(find_matches("ab", width, &empty).is_empty());
    }
}
