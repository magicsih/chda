//! Gemini CLI: `gemini` CLI, `~/.gemini/tmp/<project>/chats` transcripts,
//! hooks passed per launch as the system defaults settings file.
//!
//! Gemini CLI reads hooks from its settings layers and concatenates them
//! (`hooks.*` merge with `CONCAT` in its settings schema). The lowest layer,
//! system defaults, can be pointed elsewhere with
//! `GEMINI_CLI_SYSTEM_DEFAULTS_PATH`, so chda writes a copy of the machine's
//! system defaults with its hooks added and sets that variable for the
//! agents it starts. The user's own settings files are never touched.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::session::file_mtime_ms;
use crate::session::{parse_rfc3339_ms, snippet};
use crate::{
    AgentAdapter, AgentId, AgentSession, HookInstallReport, SessionId, shell_quote, which,
    write_if_changed,
};

pub struct GeminiAdapter;

const DEFAULTS_ENV: &str = "GEMINI_CLI_SYSTEM_DEFAULTS_PATH";

const HOOK_EVENTS: [&str; 5] = [
    "SessionStart",
    "BeforeAgent",
    "Notification",
    "AfterAgent",
    "SessionEnd",
];

/// Where the per-launch system defaults file lives under the data dir.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks").join("gemini-system-defaults.json")
}

/// `~/.gemini`, or `$GEMINI_CLI_HOME/.gemini`.
fn gemini_dir() -> Option<PathBuf> {
    std::env::var_os("GEMINI_CLI_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(".gemini"))
}

/// The system defaults file Gemini CLI reads when chda does not redirect it.
fn machine_defaults_path() -> PathBuf {
    if let Some(path) = std::env::var_os(DEFAULTS_ENV) {
        return PathBuf::from(path);
    }
    let settings = std::env::var_os("GEMINI_CLI_SYSTEM_SETTINGS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(target_os = "macos") {
                "/Library/Application Support/GeminiCli/settings.json"
            } else if cfg!(windows) {
                "C:\\ProgramData\\gemini-cli\\settings.json"
            } else {
                "/etc/gemini-cli/settings.json"
            })
        });
    settings.with_file_name("system-defaults.json")
}

/// `base` (the machine's system defaults, or `{}`) with a hook for each
/// status event that calls `chda hook gemini`.
pub fn hooks_settings(base: Value, hook_bin: &Path) -> Value {
    let mut settings = match base {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let command = format!("{} hook gemini", shell_quote(hook_bin));
    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    if let Some(hooks) = hooks {
        for event in HOOK_EVENTS {
            let entry = json!({ "hooks": [{
                "type": "command", "name": "chda", "command": command, "timeout": 10_000
            }] });
            match hooks.get_mut(event).and_then(Value::as_array_mut) {
                Some(list) => list.push(entry),
                None => {
                    hooks.insert(event.into(), json!([entry]));
                }
            }
        }
    }
    Value::Object(settings)
}

impl AgentAdapter for GeminiAdapter {
    fn id(&self) -> AgentId {
        AgentId::Gemini
    }

    fn display_name(&self) -> &str {
        "Gemini CLI"
    }

    fn short_label(&self) -> String {
        "GM".into()
    }

    fn is_installed(&self, path: &std::ffi::OsStr) -> bool {
        which("gemini", path).is_some()
    }

    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, _: &Path) -> Command {
        let mut cmd = Command::new("gemini");
        cmd.current_dir(cwd);
        if let Some(data_dir) = crate::hook::data_dir() {
            cmd.env(DEFAULTS_ENV, settings_path(&data_dir));
        }
        if let Some(id) = resume {
            cmd.arg("--resume").arg(&id.0);
        }
        cmd
    }

    fn session_roots(&self) -> Vec<PathBuf> {
        gemini_dir().map(|d| d.join("tmp")).into_iter().collect()
    }

    fn has_session(&self, id: &SessionId) -> bool {
        !id.0.is_empty() && self.session_roots().iter().any(|root| has_chat(root, id))
    }

    /// Gemini CLI keeps one folder per project directory and records that
    /// directory in its `.project_root` file.
    fn session_dirs(&self, worktrees: &[PathBuf]) -> Vec<PathBuf> {
        self.session_roots()
            .into_iter()
            .flat_map(|root| fs::read_dir(root).into_iter().flatten().flatten())
            .map(|entry| entry.path())
            .filter(|dir| {
                project_root(dir).is_some_and(|root| worktrees.iter().any(|w| root.starts_with(w)))
            })
            .map(|dir| dir.join("chats"))
            .collect()
    }

    fn session_cwd(&self, file: &Path) -> Option<PathBuf> {
        project_root(file.parent()?.parent()?)
    }

    fn parse_session(&self, file: &Path) -> Option<AgentSession> {
        parse_chat(file, &self.session_cwd(file)?)
    }

    fn install_hooks(&self, data_dir: &Path, hook_bin: &Path) -> io::Result<HookInstallReport> {
        let base = fs::read(machine_defaults_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}));
        let path = settings_path(data_dir);
        write_if_changed(
            &path,
            &serde_json::to_string_pretty(&hooks_settings(base, hook_bin))?,
        )?;
        Ok(HookInstallReport {
            file: Some(path),
            note: None,
        })
    }
}

fn project_root(project_dir: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(project_dir.join(".project_root")).ok()?;
    let root = text.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn has_chat(root: &Path, id: &SessionId) -> bool {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .any(|project| {
            fs::read_dir(project.path().join("chats"))
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| {
                    let path = entry.path();
                    matches!(
                        path.extension().and_then(|s| s.to_str()),
                        Some("json" | "jsonl")
                    ) && crate::session::head_metadata_field(&path, "sessionId").as_deref()
                        == Some(id.0.as_str())
                })
        })
}

/// Text of a user message, if it is a prompt rather than a tool response or
/// the context block Gemini CLI inserts at the start of a session.
fn prompt_text(message: &Value) -> Option<String> {
    if message.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let text = match message.get("content")? {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let trimmed = text.trim_start();
    (!trimmed.is_empty() && !trimmed.starts_with("<session_context>")).then_some(text)
}

/// Parse a Gemini CLI chat log. The first line holds the session's metadata;
/// messages follow as lines of their own, or inside `{"$set": {"messages":
/// [...]}}` updates, and the same message can be written more than once.
pub fn parse_chat(file: &Path, cwd: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut started_at = None;
    let mut subagent = false;
    let mut seen = HashSet::new();
    let mut first = None;
    let mut count = 0;
    let lines = crate::session::matching_lines(file, &["\"sessionId\"", "\"type\":\"user\""]);
    for line in lines.ok()? {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(session) = v.get("sessionId").and_then(Value::as_str) {
            id = Some(session.to_owned());
            started_at = v
                .get("startTime")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);
            subagent = v.get("kind").and_then(Value::as_str) == Some("subagent");
            continue;
        }
        let messages = match v.pointer("/$set/messages").and_then(Value::as_array) {
            Some(list) => list.iter().collect(),
            None => vec![&v],
        };
        for message in messages {
            let key = message.get("id").and_then(Value::as_str).map(str::to_owned);
            let Some(text) = prompt_text(message) else {
                continue;
            };
            if key.is_some_and(|k| !seen.insert(k)) {
                continue;
            }
            count += 1;
            if first.is_none() {
                first = Some(text);
            }
        }
    }
    if subagent {
        return None;
    }
    let first = first?;
    let last_active_at = file_mtime_ms(file)?;
    Some(AgentSession {
        id: SessionId(id?),
        agent: AgentId::Gemini,
        cwd: cwd.to_path_buf(),
        started_at: started_at.unwrap_or(last_active_at),
        last_active_at,
        snippet: snippet(&first, 120),
        message_count: count,
        usage: Default::default(),
        file: file.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_confirms_full_metadata_id_for_legacy_json_and_jsonl() {
        let dir = std::env::temp_dir().join(format!("chda-gemini-resume-{}", std::process::id()));
        let chats = dir.join("project/chats");
        fs::create_dir_all(&chats).unwrap();
        for extension in ["json", "jsonl"] {
            let file = chats.join(format!("session-time-short.{extension}"));
            fs::write(
                &file,
                r#"{"sessionId":"full-identity","messages":[{"content":"not retained"}]}"#,
            )
            .unwrap();
            assert!(has_chat(&dir, &SessionId("full-identity".into())));
            assert!(!has_chat(&dir, &SessionId("identity".into())));
            assert!(!has_chat(&dir, &SessionId("short".into())));
            fs::remove_file(file).unwrap();
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_a_chat_log_and_adds_hooks_to_the_system_defaults() {
        let dir = std::env::temp_dir().join(format!("chda-gemini-{}", std::process::id()));
        let chats = dir.join("app").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(dir.join("app").join(".project_root"), "/work/app\n").unwrap();
        let file = chats.join("session-2026-09-12T14-12-aea97e3b.jsonl");
        // Shape written by Gemini CLI 0.59.
        fs::write(&file, concat!(
            r#"{"sessionId":"aea97e3b-9e68","projectHash":"0016","startTime":"2026-09-12T14:12:18.287Z","lastUpdated":"2026-09-12T14:12:18.287Z","kind":"main"}"#, "\n",
            r#"{"$set":{"messages":[{"id":"d049","timestamp":"2026-09-12T14:12:18.287Z","type":"user","content":[{"text":"<session_context>\nThis is the Gemini CLI."}]}]}}"#, "\n",
            r#"{"id":"7433","timestamp":"2026-09-12T14:12:31.345Z","type":"user","content":[{"text":"이 폴더의   변경사항\n있어?"}]}"#, "\n",
            r#"{"$set":{"lastUpdated":"2026-09-12T14:12:31.345Z"}}"#, "\n",
            r#"{"id":"f992","timestamp":"2026-09-12T14:12:39.076Z","type":"gemini","content":""}"#, "\n",
            r#"{"id":"8827","timestamp":"2026-09-12T14:14:31.354Z","type":"user","content":[{"functionResponse":{"id":"x","name":"run_shell_command"}}]}"#, "\n",
            r#"{"id":"7433","timestamp":"2026-09-12T14:12:31.345Z","type":"user","content":[{"text":"이 폴더의   변경사항\n있어?"}]}"#, "\n",
            r#"{"id":"9001","timestamp":"2026-09-12T14:20:00.000Z","type":"user","content":[{"text":"second"}]}"#, "\n",
        )).unwrap();
        let adapter = GeminiAdapter;
        assert_eq!(adapter.session_cwd(&file), Some(PathBuf::from("/work/app")));
        let s = adapter.parse_session(&file).unwrap();
        assert_eq!(s.id, SessionId("aea97e3b-9e68".into()));
        assert_eq!(s.cwd, PathBuf::from("/work/app"));
        assert_eq!(s.snippet, "이 폴더의 변경사항 있어?");
        assert_eq!(s.message_count, 2);
        assert_eq!(
            s.started_at,
            parse_rfc3339_ms("2026-09-12T14:12:18.287Z").unwrap()
        );

        let base = json!({
            "general": { "vimMode": true },
            "hooks": { "AfterAgent": [{ "hooks": [{ "type": "command", "command": "mine" }] }] }
        });
        let settings = hooks_settings(base, Path::new("/opt/chda bin/chda"));
        assert_eq!(
            settings["general"]["vimMode"], true,
            "machine defaults kept"
        );
        let after = settings["hooks"]["AfterAgent"].as_array().unwrap();
        assert_eq!(after[0]["hooks"][0]["command"], "mine");
        assert_eq!(
            after[1]["hooks"][0]["command"],
            "'/opt/chda bin/chda' hook gemini"
        );
        assert_eq!(
            settings["hooks"]["BeforeAgent"][0]["hooks"][0]["timeout"],
            10_000
        );

        let cmd = adapter.launch_command(
            Path::new("/work/app"),
            Some(&SessionId("aea97e3b-9e68".into())),
            Path::new("/usr/local/bin/chda"),
        );
        let argv = crate::command_argv(&cmd);
        assert!(argv[1].starts_with(DEFAULTS_ENV), "{argv:?}");
        assert_eq!(
            argv[argv.len() - 3..],
            ["gemini", "--resume", "aea97e3b-9e68"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
