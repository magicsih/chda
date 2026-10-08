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
    let answers = answers(agent, session, roots);
    let mut matches = Vec::new();
    for answer in &answers {
        if answer.text.match_indices(selected).count() == 1 {
            matches.push((selected.to_owned(), answer));
        } else if let Some(body) = confirmed_gutter(agent, answer.version.as_deref(), selected)
            && body == answer.text
        {
            // Require the whole selected answer to match. A partial draft
            // must never grow into the remaining answer or lose indentation.
            matches.push((answer.text.clone(), answer));
        }
    }
    if matches.len() != 1 {
        return fallback();
    }
    let (text, answer) = matches.pop().unwrap();
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

fn confirmed_gutter(agent: AgentId, version: Option<&str>, selected: &str) -> Option<String> {
    let prefix = match (agent, version) {
        (AgentId::Codex, Some("0.161.0")) => "• ",
        (AgentId::Claude, Some("2.1.294")) => "⏺ ",
        _ => return None,
    };
    let mut lines = selected.split('\n');
    let first = lines.next()?.strip_prefix(prefix)?;
    let mut body = first.to_owned();
    for line in lines {
        body.push('\n');
        body.push_str(if line.is_empty() {
            ""
        } else {
            line.strip_prefix("  ")?
        });
    }
    Some(body)
}

fn answers(agent: AgentId, session: &SessionId, roots: &[PathBuf]) -> Vec<Answer> {
    use std::io::{BufRead, Read};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
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
            if visited > 20_000 || out.len() >= 2048 || std::time::Instant::now() >= deadline {
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
                    continue;
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
                    if text.len() > 1_048_576 || out.len() >= 2048 {
                        return Vec::new();
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
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "chda-share-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut rows = Vec::new();
        let name = if agent == AgentId::Codex {
            rows.push(serde_json::json!({"type":"session_meta","payload":{"id":id,"cli_version":"0.161.0"}}));
            format!("rollout-{id}.jsonl")
        } else {
            format!("{id}.jsonl")
        };
        for (n, answer) in answers.iter().enumerate() {
            rows.push(if agent==AgentId::Codex {
                serde_json::json!({"type":"response_item","payload":{"type":"message","id":format!("a{n}"),"role":"assistant","content":[{"type":"output_text","text":answer}]}})
            } else {
                serde_json::json!({"type":"assistant","sessionId":id,"uuid":format!("a{n}"),"version":"2.1.294","message":{"role":"assistant","content":[{"type":"text","text":answer}]}})
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
    fn ambiguous_or_partial_answers_keep_the_exact_selection() {
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
            "• Keep code:",
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
    fn actual_cli_selection_fixtures_clean_only_exact_full_answers() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/sharing-cli-renderings.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let agent = AgentId::parse(case["agent"].as_str().unwrap()).unwrap();
            let root = fixture(agent, "exact", &[case["original"].as_str().unwrap()]);
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
