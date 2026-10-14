//! Local, selection-scoped sharing previews. Conversation text is never logged.
use crate::{AgentId, SessionId};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharePreview {
    pub text: String,
    pub source: Option<String>,
}

pub fn sharing_preview(
    agent: AgentId,
    session: &SessionId,
    roots: &[PathBuf],
    selected: &str,
) -> SharePreview {
    let fallback = || SharePreview {
        text: selected.into(),
        source: None,
    };
    if selected.is_empty()
        || selected.len() > 1_048_576
        || session.0.is_empty()
        || session.0.len() > 256
        || session.0.contains(['/', '\\'])
        || !matches!(agent, AgentId::Claude | AgentId::Codex)
    {
        return fallback();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let answers = answers(agent, session, roots, deadline, None);
    let mut found: Option<(usize, Vec<std::ops::Range<usize>>)> = None;
    for (index, answer) in answers.iter().enumerate() {
        if std::time::Instant::now() >= deadline {
            return fallback();
        }
        let mut remember = |ranges: Vec<std::ops::Range<usize>>| {
            if ranges.is_empty() {
                return true;
            }
            if let Some(previous) = &found {
                return *previous == (index, ranges);
            }
            found = Some((index, ranges));
            true
        };
        for start in occurrences(&answer.text, selected) {
            if !remember(std::iter::once(start..start + selected.len()).collect()) {
                return fallback();
            }
        }
        if let Some(display) = projection(agent, answer.version.as_deref(), &answer.text) {
            for start in occurrences(&display.text, selected) {
                if !remember(display.source_ranges(start..start + selected.len())) {
                    return fallback();
                }
            }
        }
    }
    if std::time::Instant::now() >= deadline {
        return fallback();
    }
    let Some((index, ranges)) = found else {
        return fallback();
    };
    let answer = &answers[index];
    let text = ranges.into_iter().map(|r| &answer.text[r]).collect();
    SharePreview {
        text,
        source: Some(format!(
            "{} · conversation {} · answer {}{}",
            agent.as_str(),
            session.0,
            answer.id,
            answer
                .version
                .as_ref()
                .map(|v| format!(" · {v}"))
                .unwrap_or_default()
        )),
    }
}

struct Answer {
    id: String,
    text: String,
    version: Option<String>,
}

/// The last `count` assistant text blocks of the exact conversation
/// `session`, oldest first: what the agent answered, never what the user is
/// typing. `None` when the transcript cannot be read within two seconds.
pub fn recent_answers(
    agent: AgentId,
    session: &SessionId,
    roots: &[PathBuf],
    count: usize,
) -> Option<Vec<String>> {
    if session.0.is_empty()
        || session.0.len() > 256
        || session.0.contains(['/', '\\'])
        || !matches!(agent, AgentId::Claude | AgentId::Codex)
        || count == 0
    {
        return None;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let answers = answers(agent, session, roots, deadline, Some(count));
    (std::time::Instant::now() < deadline && !answers.is_empty())
        .then(|| answers.into_iter().map(|a| a.text).collect())
}

struct Span {
    display: std::ops::Range<usize>,
    source: Option<std::ops::Range<usize>>,
}

struct Projection {
    text: String,
    spans: Vec<Span>,
}

impl Projection {
    fn push(&mut self, text: &str, source: Option<std::ops::Range<usize>>) {
        let start = self.text.len();
        self.text.push_str(text);
        self.spans.push(Span {
            display: start..self.text.len(),
            source,
        });
    }

    fn source_ranges(&self, selected: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        for span in &self.spans {
            let start = span.display.start.max(selected.start);
            let end = span.display.end.min(selected.end);
            if start >= end {
                continue;
            }
            let Some(source) = &span.source else { continue };
            let mapped = if span.display.len() == source.len() {
                source.start + start - span.display.start..source.start + end - span.display.start
            } else {
                // Only a single displayed indentation space maps to a group
                // of original spaces. Selection cannot cut that display cell.
                debug_assert_eq!(span.display.len(), 1);
                source.clone()
            };
            if let Some(last) = ranges.last_mut()
                && last.end == mapped.start
            {
                last.end = mapped.end;
            } else {
                ranges.push(mapped);
            }
        }
        ranges
    }
}

fn projection(agent: AgentId, version: Option<&str>, original: &str) -> Option<Projection> {
    let (prefix, fences, nested_lists) = match (agent, version) {
        (AgentId::Codex, Some("0.161.0")) => ("• ", false, false),
        (AgentId::Claude, Some("2.1.294")) => ("⏺ ", false, false),
        (AgentId::Codex, Some("0.162.1")) => ("• ", true, false),
        (AgentId::Claude, Some("2.1.296")) => ("⏺ ", true, true),
        _ => return None,
    };
    let mut out = Projection {
        text: String::new(),
        spans: Vec::new(),
    };
    let mut offset = 0;
    let mut first = true;
    let mut fence: Option<(u8, usize)> = None;
    let mut list_indents: Vec<usize> = Vec::new();
    for line in original.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if fences && let Some(marker) = fence_marker(body) {
            if let Some((kind, length)) = fence {
                if marker.0 == kind && marker.1 >= length && body[marker.1..].trim().is_empty() {
                    fence = None;
                    offset += line.len();
                    continue;
                }
            } else {
                fence = Some(marker);
                list_indents.clear();
                offset += line.len();
                continue;
            }
        }
        if !body.is_empty() {
            out.push(if first { prefix } else { "  " }, None);
            first = false;
        }
        let indent = body.bytes().take_while(|&b| b == b' ').count();
        let mut displayed_indent = indent;
        if nested_lists
            && fence.is_none()
            && indent.is_multiple_of(2)
            && body[indent..].starts_with("- ")
        {
            if indent == 0 {
                list_indents.clear();
                list_indents.push(0);
            } else if let Some(&last) = list_indents.last() {
                if indent > last {
                    list_indents.push(indent);
                } else {
                    while list_indents.last().is_some_and(|&n| n > indent) {
                        list_indents.pop();
                    }
                }
                if list_indents.last() == Some(&indent) {
                    // Claude 2.1.296 uses two columns per nested hyphen-list
                    // level. The actual PTY fixtures confirm 2/4/8-space input.
                    displayed_indent = 2 * (list_indents.len() - 1);
                }
            }
        } else if !body.is_empty() {
            list_indents.clear();
        }
        if displayed_indent > 0 && displayed_indent != indent {
            for n in 0..displayed_indent {
                out.push(
                    " ",
                    Some(
                        offset + n * indent / displayed_indent
                            ..offset + (n + 1) * indent / displayed_indent,
                    ),
                );
            }
            out.push(&body[indent..], Some(offset + indent..offset + body.len()));
        } else {
            out.push(body, Some(offset..offset + body.len()));
        }
        if line.ends_with('\n') {
            out.push("\n", Some(offset + body.len()..offset + line.len()));
        }
        offset += line.len();
    }
    // An incomplete code block is not a confirmed omitted-fence layout.
    fence.is_none().then_some(out)
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let kind = *line.as_bytes().first()?;
    if !matches!(kind, b'`' | b'~') {
        return None;
    }
    let length = line.bytes().take_while(|&b| b == kind).count();
    (length >= 3).then_some((kind, length))
}

// Include overlapping occurrences; finding just one nonoverlapping occurrence
// does not establish a unique source selection.
fn occurrences<'a>(text: &'a str, selected: &'a str) -> impl Iterator<Item = usize> + 'a {
    let mut start = 0;
    std::iter::from_fn(move || {
        let found = start + text.get(start..)?.find(selected)?;
        start = found + text[found..].chars().next()?.len_utf8();
        Some(found)
    })
}

/// Assistant text blocks of the exact conversation. `last` keeps only that
/// many of the newest; without it the lookup must be complete, as uniqueness
/// checks need, and gives up past 2048 blocks.
fn answers(
    agent: AgentId,
    session: &SessionId,
    roots: &[PathBuf],
    deadline: std::time::Instant,
    last: Option<usize>,
) -> Vec<Answer> {
    use std::io::{BufRead, Read};
    let mut stack: Vec<_> = roots.iter().cloned().map(|p| (p, 0)).collect();
    let mut out = Vec::new();
    let mut visited = 0;
    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Vec::new(),
        };
        for entry in entries {
            let Ok(entry) = entry else { return Vec::new() };
            visited += 1;
            if visited > 20_000
                || (last.is_none() && out.len() >= 2048)
                || std::time::Instant::now() >= deadline
            {
                return Vec::new(); // Incomplete lookup cannot prove uniqueness.
            }
            let Ok(kind) = entry.file_type() else {
                return Vec::new();
            };
            if kind.is_dir() {
                if depth >= 6 {
                    return Vec::new();
                }
                stack.push((entry.path(), depth + 1));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            let candidate = if agent == AgentId::Claude {
                path.file_name()
                    .is_some_and(|n| n == format!("{}.jsonl", session.0).as_str())
            } else {
                path.extension().is_some_and(|e| e == "jsonl")
                    && path
                        .file_stem()
                        .is_some_and(|n| n.to_string_lossy().ends_with(&session.0))
            };
            if !candidate {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else {
                return Vec::new();
            };
            if !file
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= 33_554_432)
            {
                return Vec::new();
            }
            let reader = std::io::BufReader::new(file.take(33_554_433));
            let mut confirmed = agent == AgentId::Claude;
            let mut version = None;
            for (line_index, line) in reader.lines().enumerate() {
                if std::time::Instant::now() >= deadline {
                    return Vec::new();
                }
                let Ok(line) = line else { return Vec::new() };
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    return Vec::new(); // A partial transcript cannot prove uniqueness.
                };
                if agent == AgentId::Codex && value["type"] == "session_meta" {
                    if value["payload"]["id"].as_str() != Some(&session.0) {
                        return Vec::new();
                    }
                    confirmed = true;
                    version = value["payload"]["cli_version"].as_str().map(str::to_owned);
                    continue;
                }
                if !confirmed {
                    continue;
                }
                let (content, id, answer_version) = if agent == AgentId::Codex
                    && value["type"] == "response_item"
                    && value["payload"]["type"] == "message"
                    && value["payload"]["role"] == "assistant"
                {
                    (
                        &value["payload"]["content"],
                        value["payload"]["id"].as_str(),
                        version.clone(),
                    )
                } else if agent == AgentId::Claude
                    && value["type"] == "assistant"
                    && value["message"]["role"] == "assistant"
                    && value["sessionId"].as_str() == Some(&session.0)
                {
                    (
                        &value["message"]["content"],
                        value["uuid"].as_str(),
                        value["version"].as_str().map(str::to_owned),
                    )
                } else {
                    continue;
                };
                for (index, block) in content.as_array().into_iter().flatten().enumerate() {
                    if !matches!(block["type"].as_str(), Some("text" | "output_text")) {
                        continue;
                    }
                    let Some(text) = block["text"].as_str() else {
                        continue;
                    };
                    if text.len() > 1_048_576 || (last.is_none() && out.len() >= 2048) {
                        return Vec::new();
                    }
                    if last.is_some_and(|n| out.len() >= n) {
                        out.remove(0);
                    }
                    out.push(Answer {
                        id: format!(
                            "{}:{index}",
                            id.map(str::to_owned)
                                .unwrap_or_else(|| format!("line{}", line_index + 1))
                        ),
                        text: text.into(),
                        version: answer_version.clone(),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(agent: AgentId, id: &str, answers: &[&str]) -> PathBuf {
        let version = if agent == AgentId::Codex {
            "0.161.0"
        } else {
            "2.1.294"
        };
        fixture_version(agent, id, version, answers)
    }
    fn fixture_version(agent: AgentId, id: &str, version: &str, answers: &[&str]) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "chda-share-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut rows = Vec::new();
        let name = if agent == AgentId::Codex {
            rows.push(serde_json::json!({"type":"session_meta","payload":{"id":id,"cli_version":version}}));
            format!("rollout-{id}.jsonl")
        } else {
            format!("{id}.jsonl")
        };
        for (n, answer) in answers.iter().enumerate() {
            rows.push(if agent==AgentId::Codex {
                serde_json::json!({"type":"response_item","payload":{"type":"message","id":format!("a{n}"),"role":"assistant","content":[{"type":"output_text","text":answer}]}})
            } else {
                serde_json::json!({"type":"assistant","sessionId":id,"uuid":format!("a{n}"),"version":version,"message":{"role":"assistant","content":[{"type":"text","text":answer}]}})
            });
        }
        std::fs::write(
            root.join(name),
            rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
        )
        .unwrap();
        root
    }
    #[test]
    fn recent_answers_keep_the_newest_of_the_exact_conversation() {
        for agent in [AgentId::Claude, AgentId::Codex] {
            let many: Vec<String> = (0..3000).map(|n| format!("answer {n}")).collect();
            let refs: Vec<&str> = many.iter().map(String::as_str).collect();
            let root = fixture(agent, "s1", &refs);
            let session = SessionId("s1".into());
            assert_eq!(
                recent_answers(agent, &session, std::slice::from_ref(&root), 2),
                Some(vec!["answer 2998".to_owned(), "answer 2999".to_owned()]),
                "{agent:?} keeps the newest past the uniqueness cap"
            );
            assert_eq!(
                recent_answers(
                    agent,
                    &SessionId("other".into()),
                    std::slice::from_ref(&root),
                    2
                ),
                None
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn full_confirmed_gutter_matches_only_the_selected_conversation() {
        let root = fixture(
            AgentId::Codex,
            "exact",
            &["A shared draft.\n\n안녕 🧭\n| meaningful pipe"],
        );
        let selected = "• A shared draft.\n\n  안녕 🧭\n  | meaningful pipe";
        let preview = sharing_preview(
            AgentId::Codex,
            &SessionId("exact".into()),
            std::slice::from_ref(&root),
            selected,
        );
        assert_eq!(
            preview.text,
            "A shared draft.\n\n안녕 🧭\n| meaningful pipe"
        );
        assert!(preview.source.is_some());
        let wrong = sharing_preview(
            AgentId::Codex,
            &SessionId("other".into()),
            std::slice::from_ref(&root),
            selected,
        );
        assert_eq!(wrong.text, selected);
        assert!(wrong.source.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ambiguous_answers_keep_the_exact_selection_and_literal_content_stays_intact() {
        let root = fixture(
            AgentId::Codex,
            "exact",
            &[
                "Keep code:\n    a | b\n> quote\n| table | cell |",
                "Repeated draft",
                "Repeated draft",
            ],
        );
        for selection in [
            "    a | b",
            "> quote",
            "| table | cell |",
            "• Repeated draft",
        ] {
            assert_eq!(
                sharing_preview(
                    AgentId::Codex,
                    &SessionId("exact".into()),
                    std::slice::from_ref(&root),
                    selection
                )
                .text,
                selection
            );
        }
        let partial = sharing_preview(
            AgentId::Codex,
            &SessionId("exact".into()),
            std::slice::from_ref(&root),
            "• Keep code:",
        );
        assert_eq!(partial.text, "Keep code:");
        assert!(partial.source.is_some());
        assert!(
            sharing_preview(
                AgentId::Codex,
                &SessionId("exact".into()),
                std::slice::from_ref(&root),
                "• Repeated draft"
            )
            .source
            .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn actual_cli_selection_fixtures_restore_only_uniquely_selected_source() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/sharing-cli-renderings.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let agent = AgentId::parse(case["agent"].as_str().unwrap()).unwrap();
            let root = fixture_version(
                agent,
                "exact",
                case["version"].as_str().unwrap(),
                &[case["original"].as_str().unwrap()],
            );
            let preview = sharing_preview(
                agent,
                &SessionId("exact".into()),
                std::slice::from_ref(&root),
                case["selected"].as_str().unwrap(),
            );
            assert_eq!(
                preview.text,
                case["expected"].as_str().unwrap(),
                "{} {}",
                case["agent"],
                case["layout"]
            );
            assert_eq!(preview.source.is_some(), case["matched"].as_bool().unwrap());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn claude_source_match_preserves_partial_unicode_markdown_and_code() {
        let root = fixture(
            AgentId::Claude,
            "exact",
            &["안녕 🧭\n    a | b\n> quote\n| table | cell |"],
        );
        let preview = sharing_preview(
            AgentId::Claude,
            &SessionId("exact".into()),
            std::slice::from_ref(&root),
            "🧭\n    a | b",
        );
        assert_eq!(preview.text, "🧭\n    a | b");
        assert!(
            preview
                .source
                .as_ref()
                .unwrap()
                .contains("claude · conversation exact")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_projection_preserves_unicode_boundaries_and_never_adds_hidden_fences() {
        let body = "Before\n\n```rust\nfn greet() {\n    println!(\"안녕 🧭\");\n}\n```\n\nAfter";
        let root = fixture_version(AgentId::Claude, "exact", "2.1.296", &[body]);
        for (selected, expected) in [
            (
                "greet() {\n      println!(\"안녕 🧭\");",
                "greet() {\n    println!(\"안녕 🧭\");",
            ),
            ("🧭", "🧭"),
            (" println!", " println!"),
            ("⏺ Before", "Before"),
        ] {
            let result = sharing_preview(
                AgentId::Claude,
                &SessionId("exact".into()),
                std::slice::from_ref(&root),
                selected,
            );
            assert_eq!(result.text, expected);
            assert!(!result.text.contains("```"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_projected_ranges_and_overlapping_literal_ranges_remain_unchanged() {
        for body in ["Repeated\nRepeated", "aaa"] {
            let root = fixture_version(AgentId::Claude, "exact", "2.1.296", &[body]);
            let selected = if body == "aaa" { "aa" } else { "Repeated" };
            let result = sharing_preview(
                AgentId::Claude,
                &SessionId("exact".into()),
                std::slice::from_ref(&root),
                selected,
            );
            assert_eq!(result.text, selected);
            assert!(result.source.is_none());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn malformed_exact_transcript_cannot_prove_a_unique_partial_match() {
        let root = fixture_version(AgentId::Claude, "exact", "2.1.296", &["Draft"]);
        let path = root.join("exact.jsonl");
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, format!("{original}{{malformed assistant row\n")).unwrap();
        let result = sharing_preview(
            AgentId::Claude,
            &SessionId("exact".into()),
            std::slice::from_ref(&root),
            "⏺ Draft",
        );
        assert_eq!(result.text, "⏺ Draft");
        assert!(result.source.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unconfirmed_version_identity_or_incomplete_lookup_cannot_clean_gutters() {
        let root = fixture(AgentId::Codex, "exact", &["Draft"]);
        let path = root.join("rollout-exact.jsonl");
        let original = std::fs::read_to_string(&path).unwrap();
        for replacement in [
            original.replace("0.161.0", "unknown"),
            original.replace("\"id\":\"exact\"", "\"id\":\"other\""),
        ] {
            std::fs::write(&path, replacement).unwrap();
            let preview = sharing_preview(
                AgentId::Codex,
                &SessionId("exact".into()),
                std::slice::from_ref(&root),
                "• Draft",
            );
            assert_eq!(preview.text, "• Draft");
            assert!(preview.source.is_none());
        }
        std::fs::write(&path, original).unwrap();
        let deep = root.join("1/2/3/4/5/6/7");
        std::fs::create_dir_all(&deep).unwrap();
        let preview = sharing_preview(
            AgentId::Codex,
            &SessionId("exact".into()),
            std::slice::from_ref(&root),
            "• Draft",
        );
        assert_eq!(preview.text, "• Draft");
        assert!(preview.source.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
