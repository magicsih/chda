//! Codex: `codex` CLI, `~/.codex/sessions` rollouts, status through the
//! `notify` hook passed per launch with `-c`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::session::{file_mtime_ms, parse_rfc3339_ms, snippet};
use crate::{AgentAdapter, AgentId, AgentSession, HookInstallReport, SessionId, which};

pub struct CodexAdapter;

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))
}

/// The user's own `notify` program from `~/.codex/config.toml`, so chda can
/// run it after its own hook instead of replacing it.
pub fn existing_notify(config_toml: &str) -> Option<Vec<String>> {
    let line = config_toml
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("notify") && l[6..].trim_start().starts_with('='))?;
    let value = line.split_once('=')?.1.trim();
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    let items: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    (!items.is_empty()).then_some(items)
}

/// `-c notify=[...]` value: chda's hook, then the user's program if any.
pub fn notify_override(hook_bin: &Path, existing: Option<&[String]>) -> String {
    let mut argv: Vec<String> = vec![
        hook_bin.to_string_lossy().into_owned(),
        "hook".into(),
        "codex".into(),
    ];
    if let Some(existing) = existing {
        argv.push("--then".into());
        argv.extend(existing.iter().cloned());
    }
    let quoted: Vec<String> = argv
        .iter()
        .map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("notify=[{}]", quoted.join(","))
}

impl AgentAdapter for CodexAdapter {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn display_name(&self) -> &str {
        "Codex"
    }

    fn short_label(&self) -> String {
        "CX".into()
    }

    fn is_installed(&self) -> bool {
        which("codex").is_some()
    }

    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, hook_bin: &Path) -> Command {
        let mut cmd = Command::new("codex");
        cmd.current_dir(cwd);
        let existing = codex_home()
            .and_then(|h| fs::read_to_string(h.join("config.toml")).ok())
            .and_then(|t| existing_notify(&t));
        cmd.arg("-c")
            .arg(notify_override(hook_bin, existing.as_deref()));
        if let Some(id) = resume {
            cmd.arg("resume").arg(&id.0);
        }
        cmd
    }

    fn session_roots(&self) -> Vec<PathBuf> {
        codex_home()
            .map(|h| h.join("sessions"))
            .into_iter()
            .collect()
    }

    fn parse_session(&self, file: &Path) -> Option<AgentSession> {
        parse_rollout(file)
    }

    fn install_hooks(&self, _: &Path, _: &Path) -> io::Result<HookInstallReport> {
        // Nothing to write: the notify override travels on the command line.
        Ok(HookInstallReport {
            file: None,
            note: Some(
                "status comes from Codex's notify hook, which only reports turn completion".into(),
            ),
        })
    }
}

fn user_message_text(item: &Value) -> Option<String> {
    match item.get("content")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let text: Vec<&str> = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            (!text.is_empty()).then(|| text.join(" "))
        }
        _ => None,
    }
}

/// Parse a Codex rollout: `session_meta` for id and cwd, then
/// `event_msg`/`item_completed` with a `UserMessage` item per prompt.
pub fn parse_rollout(file: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut started_at = None;
    let mut first = None;
    let mut count = 0;
    for line in crate::session::matching_lines(file, &["session_meta", "UserMessage"]).ok()? {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let payload = v.get("payload");
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                let p = payload?;
                id = p
                    .get("id")
                    .or_else(|| p.get("session_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                cwd = p.get("cwd").and_then(Value::as_str).map(PathBuf::from);
                started_at = p
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_ms);
            }
            Some("event_msg") => {
                let Some(p) = payload else { continue };
                if p.get("type").and_then(Value::as_str) != Some("item_completed") {
                    continue;
                }
                let Some(item) = p.get("item") else { continue };
                if item.get("type").and_then(Value::as_str) != Some("UserMessage") {
                    continue;
                }
                if let Some(text) = user_message_text(item) {
                    count += 1;
                    if first.is_none() {
                        first = Some(text);
                    }
                }
            }
            _ => {}
        }
    }
    let first = first?;
    let last_active_at = file_mtime_ms(file)?;
    Some(AgentSession {
        id: SessionId(id?),
        agent: AgentId::Codex,
        cwd: cwd?,
        started_at: started_at.unwrap_or(last_active_at),
        last_active_at,
        snippet: snippet(&first, 120),
        message_count: count,
        file: file.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_rollout_and_chains_notify() {
        let dir = std::env::temp_dir().join(format!("chda-codex-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout.jsonl");
        fs::write(&file, concat!(
            r#"{"timestamp":"2026-05-22T08:22:31.613Z","type":"session_meta","payload":{"id":"019e","session_id":"019e","timestamp":"2026-05-22T08:22:31.540Z","cwd":"/work/app"}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"task_started"}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"이 작업 진행해줘.  둘째 줄"}]}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":"x"}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":"again"}}}"#, "\n",
        )).unwrap();
        let s = parse_rollout(&file).unwrap();
        assert_eq!(s.id, SessionId("019e".into()));
        assert_eq!(s.cwd, PathBuf::from("/work/app"));
        assert_eq!(s.snippet, "이 작업 진행해줘. 둘째 줄");
        assert_eq!(s.message_count, 2);
        assert_eq!(
            s.started_at,
            parse_rfc3339_ms("2026-05-22T08:22:31.540Z").unwrap()
        );

        let cfg = "model = \"x\"\nnotify = [\"/Apps/Client.app/Contents/MacOS/Client\", \"turn-ended\"]\n";
        let existing = existing_notify(cfg).unwrap();
        assert_eq!(
            existing,
            vec!["/Apps/Client.app/Contents/MacOS/Client", "turn-ended"]
        );
        assert_eq!(
            notify_override(Path::new("/usr/local/bin/chda"), Some(&existing)),
            r#"notify=["/usr/local/bin/chda","hook","codex","--then","/Apps/Client.app/Contents/MacOS/Client","turn-ended"]"#
        );
        assert_eq!(existing_notify("model = \"x\"\n"), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
