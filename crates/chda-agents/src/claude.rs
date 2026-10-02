//! Claude Code: `claude` CLI, `~/.claude/projects` transcripts, hooks
//! injected per launch with `--settings`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::session::{file_mtime_ms, parse_rfc3339_ms, snippet};
use crate::{
    AgentAdapter, AgentId, AgentSession, HookInstallReport, SessionId, shell_quote, which,
    write_if_changed,
};

pub struct ClaudeAdapter;

const HOOK_EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "Stop",
    "Notification",
    "PermissionRequest",
    "SessionEnd",
];

/// Where the per-launch settings file lives under the data dir.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks").join("claude-settings.json")
}

/// The settings JSON that makes Claude Code call `chda hook claude`.
pub fn hooks_settings(hook_bin: &Path) -> Value {
    let command = format!("{} hook claude", shell_quote(hook_bin));
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        hooks.insert(
            event.into(),
            json!([{ "hooks": [{ "type": "command", "command": command, "timeout": 10 }] }]),
        );
    }
    json!({ "hooks": hooks })
}

impl AgentAdapter for ClaudeAdapter {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }

    fn short_label(&self) -> String {
        "CC".into()
    }

    fn is_installed(&self) -> bool {
        which("claude").is_some()
    }

    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, hook_bin: &Path) -> Command {
        let mut cmd = Command::new("claude");
        cmd.current_dir(cwd);
        if let Some(data_dir) = crate::hook::data_dir() {
            cmd.arg("--settings").arg(settings_path(&data_dir));
        }
        let _ = hook_bin;
        if let Some(id) = resume {
            cmd.arg("--resume").arg(&id.0);
        }
        cmd
    }

    fn session_roots(&self) -> Vec<PathBuf> {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".claude").join("projects"))
            .into_iter()
            .collect()
    }

    /// Claude Code files transcripts under `projects/<directory with every
    /// non-alphanumeric character replaced by '-'>`, so only folders that
    /// match a worktree (or a directory inside one) need reading.
    fn session_dirs(&self, worktrees: &[PathBuf]) -> Vec<PathBuf> {
        let prefixes: Vec<String> = worktrees.iter().map(|w| project_dir_name(w)).collect();
        self.session_roots()
            .into_iter()
            .flat_map(|root| fs::read_dir(root).into_iter().flatten().flatten())
            .map(|entry| entry.path())
            .filter(|dir| {
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
                name.is_some_and(|name| {
                    prefixes.iter().any(|p| {
                        name == *p
                            || name
                                .strip_prefix(p.as_str())
                                .is_some_and(|r| r.starts_with('-'))
                    })
                })
            })
            .collect()
    }

    fn parse_session(&self, file: &Path) -> Option<AgentSession> {
        parse_transcript(file)
    }

    fn install_hooks(&self, data_dir: &Path, hook_bin: &Path) -> io::Result<HookInstallReport> {
        let path = settings_path(data_dir);
        write_if_changed(
            &path,
            &serde_json::to_string_pretty(&hooks_settings(hook_bin))?,
        )?;
        Ok(HookInstallReport {
            file: Some(path),
            note: None,
        })
    }
}

/// The folder name Claude Code uses for a directory's transcripts.
fn project_dir_name(dir: &Path) -> String {
    dir.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Text of a user message, if it is a real prompt rather than a tool result.
fn user_text(message: &Value) -> Option<String> {
    match message.get("content")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let text: Vec<&str> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            (!text.is_empty()).then(|| text.join(" "))
        }
        _ => None,
    }
}

/// Parse a Claude Code transcript. The format is internal to Claude Code;
/// this reads only `type == "user"` lines and tolerates anything else.
pub fn parse_transcript(file: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut started_at = None;
    let mut first = None;
    let mut count = 0;
    for line in crate::session::matching_lines(file, &["\"type\":\"user\""]).ok()? {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("user")
            || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(message) = v.get("message") else {
            continue;
        };
        let Some(prompt) = user_text(message) else {
            continue;
        };
        // Skip synthetic prompts Claude Code inserts itself.
        if prompt.starts_with('<') && prompt.contains("</") && count == 0 {
            continue;
        }
        count += 1;
        if first.is_none() {
            first = Some(prompt);
            id = v
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            cwd = v.get("cwd").and_then(Value::as_str).map(PathBuf::from);
            started_at = v
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);
        }
    }
    let first = first?;
    let id = id.or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
    let last_active_at = file_mtime_ms(file)?;
    Some(AgentSession {
        id: SessionId(id),
        agent: AgentId::Claude,
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
    fn parses_a_transcript_and_builds_hook_settings() {
        let dir = std::env::temp_dir().join(format!("chda-claude-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc.jsonl");
        fs::write(&file, concat!(
            r#"{"type":"last-prompt","sessionId":"abc"}"#, "\n",
            r#"{"type":"user","isSidechain":false,"sessionId":"abc","cwd":"/tmp/proj","timestamp":"2026-09-09T12:42:15.371Z","message":{"role":"user","content":"다국어 지원은   아직\n반영 안 됐나?"}}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}"#, "\n",
            r#"{"type":"user","sessionId":"abc","cwd":"/tmp/proj","message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}}"#, "\n",
            r#"{"type":"user","sessionId":"abc","cwd":"/tmp/proj","message":{"role":"user","content":[{"type":"text","text":"second"}]}}"#, "\n",
            "not json\n",
        )).unwrap();
        let s = parse_transcript(&file).unwrap();
        assert_eq!(s.id, SessionId("abc".into()));
        assert_eq!(s.cwd, PathBuf::from("/tmp/proj"));
        assert_eq!(s.started_at, 1_788_957_735_371);
        assert_eq!(s.snippet, "다국어 지원은 아직 반영 안 됐나?");
        assert_eq!(s.message_count, 2);

        let settings = hooks_settings(Path::new("/opt/chda bin/chda"));
        let stop = &settings["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(stop["command"], "'/opt/chda bin/chda' hook claude");
        assert_eq!(stop["timeout"], 10);
        assert!(settings["hooks"].get("PermissionRequest").is_some());

        let report = ClaudeAdapter
            .install_hooks(&dir, Path::new("/usr/local/bin/chda"))
            .unwrap();
        let written = fs::read_to_string(report.file.unwrap()).unwrap();
        assert!(written.contains("/usr/local/bin/chda hook claude"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn project_folders_follow_claude_code_naming() {
        assert_eq!(
            project_dir_name(Path::new("/Users/me/.agent")),
            "-Users-me--agent"
        );
        assert_eq!(
            project_dir_name(Path::new("/src/app.worktrees/feat_x")),
            "-src-app-worktrees-feat-x"
        );
    }
}
