//! GitHub Copilot CLI: `copilot` CLI, `~/.copilot/session-state/<id>/
//! events.jsonl` transcripts, hooks loaded per launch from a local plugin
//! with `--plugin-dir`.

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

pub struct CopilotAdapter;

/// Copilot CLI hook events chda listens to. The payloads do not carry the
/// event name, so each gets its own command with the name as an argument.
const HOOK_EVENTS: [&str; 5] = [
    "sessionStart",
    "userPromptSubmitted",
    "notification",
    "agentStop",
    "sessionEnd",
];

/// The plugin directory under the data dir.
pub fn plugin_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks").join("copilot")
}

/// `~/.copilot`, or `$COPILOT_HOME`.
fn copilot_home() -> Option<PathBuf> {
    std::env::var_os("COPILOT_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".copilot")))
}

/// The plugin's `hooks.json`: one command per event calling
/// `chda hook copilot <event>`.
pub fn hooks_config(hook_bin: &Path) -> Value {
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        let command = format!("{} hook copilot {event}", shell_quote(hook_bin));
        hooks.insert(
            event.into(),
            json!([{ "type": "command", "bash": command, "timeoutSec": 10 }]),
        );
    }
    json!({ "version": 1, "hooks": hooks })
}

fn plugin_manifest() -> Value {
    json!({
        "name": "chda-status",
        "version": "1.0.0",
        "description": "Reports session status to chda",
        "hooks": "hooks.json"
    })
}

impl AgentAdapter for CopilotAdapter {
    fn id(&self) -> AgentId {
        AgentId::Copilot
    }

    fn display_name(&self) -> &str {
        "Copilot CLI"
    }

    fn short_label(&self) -> String {
        "CP".into()
    }

    fn is_installed(&self) -> bool {
        which("copilot").is_some()
    }

    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, _: &Path) -> Command {
        let mut cmd = Command::new("copilot");
        cmd.current_dir(cwd);
        if let Some(data_dir) = crate::hook::data_dir() {
            cmd.arg("--plugin-dir").arg(plugin_dir(&data_dir));
        }
        if let Some(id) = resume {
            // `--resume` takes an optional value; `=` keeps the id attached.
            cmd.arg(format!("--resume={}", id.0));
        }
        cmd
    }

    fn session_roots(&self) -> Vec<PathBuf> {
        copilot_home()
            .map(|h| h.join("session-state"))
            .into_iter()
            .collect()
    }

    /// The directory is in `session.start`'s `data.context.cwd`.
    fn session_cwd(&self, file: &Path) -> Option<PathBuf> {
        if file.file_name()? != "events.jsonl" {
            return None;
        }
        crate::session::matching_lines(file, &["\"session.start\""])
            .ok()?
            .take(1)
            .find_map(|line| {
                let v: Value = serde_json::from_str(&line).ok()?;
                let cwd = v.pointer("/data/context/cwd")?.as_str()?;
                Some(PathBuf::from(cwd))
            })
    }

    fn parse_session(&self, file: &Path) -> Option<AgentSession> {
        parse_events(file)
    }

    fn install_hooks(&self, data_dir: &Path, hook_bin: &Path) -> io::Result<HookInstallReport> {
        let dir = plugin_dir(data_dir);
        write_if_changed(
            &dir.join("plugin.json"),
            &serde_json::to_string_pretty(&plugin_manifest())?,
        )?;
        let hooks = dir.join("hooks.json");
        write_if_changed(
            &hooks,
            &serde_json::to_string_pretty(&hooks_config(hook_bin))?,
        )?;
        Ok(HookInstallReport {
            file: Some(hooks),
            note: None,
        })
    }
}

/// Parse a Copilot CLI session log: `session.start` for id, directory and
/// start time, then one `user.message` per prompt.
pub fn parse_events(file: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut started_at = None;
    let mut first = None;
    let mut count = 0;
    let lines = crate::session::matching_lines(file, &["\"session.start\"", "\"user.message\""]);
    for line in lines.ok()? {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(data) = v.get("data") else { continue };
        match v.get("type").and_then(Value::as_str) {
            Some("session.start") => {
                id = data
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                cwd = data
                    .pointer("/context/cwd")
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                started_at = data
                    .get("startTime")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_ms);
            }
            Some("user.message") => {
                let Some(text) = data.get("content").and_then(Value::as_str) else {
                    continue;
                };
                count += 1;
                if first.is_none() {
                    first = Some(text.to_owned());
                }
            }
            _ => {}
        }
    }
    let first = first?;
    let last_active_at = file_mtime_ms(file)?;
    let id = id.or_else(|| {
        let dir = file.parent()?.file_name()?;
        Some(dir.to_string_lossy().into_owned())
    })?;
    Some(AgentSession {
        id: SessionId(id),
        agent: AgentId::Copilot,
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
    use std::fs;

    #[test]
    fn parses_a_session_log_and_writes_the_plugin() {
        let dir = std::env::temp_dir().join(format!("chda-copilot-{}", std::process::id()));
        let session = dir.join("session-state").join("02c60a53");
        fs::create_dir_all(&session).unwrap();
        let file = session.join("events.jsonl");
        // Shape written by Copilot CLI 1.0.61.
        fs::write(&file, concat!(
            r#"{"type":"session.start","data":{"sessionId":"02c60a53","version":1,"producer":"copilot-agent","startTime":"2026-06-13T15:09:32.931Z","context":{"cwd":"/work/app","gitRoot":"/work/app","branch":"main"}},"id":"a","timestamp":"2026-06-13T15:09:32.931Z"}"#, "\n",
            r#"{"type":"system.message","data":{"role":"system","content":"You are the GitHub Copilot CLI"}}"#, "\n",
            r#"{"type":"user.message","data":{"content":"hello   there\nfriend","transformedContent":"<current_datetime>x</current_datetime> hello"}}"#, "\n",
            r#"{"type":"assistant.message","data":{"content":"Hello!"}}"#, "\n",
            r#"{"type":"user.message","data":{"content":"again"}}"#, "\n",
        )).unwrap();
        let adapter = CopilotAdapter;
        assert_eq!(adapter.session_cwd(&file), Some(PathBuf::from("/work/app")));
        let s = adapter.parse_session(&file).unwrap();
        assert_eq!(s.id, SessionId("02c60a53".into()));
        assert_eq!(s.snippet, "hello there friend");
        assert_eq!(s.message_count, 2);
        assert_eq!(
            s.started_at,
            parse_rfc3339_ms("2026-06-13T15:09:32.931Z").unwrap()
        );

        let report = adapter
            .install_hooks(&dir, Path::new("/opt/chda bin/chda"))
            .unwrap();
        let hooks: Value =
            serde_json::from_str(&fs::read_to_string(report.file.unwrap()).unwrap()).unwrap();
        assert_eq!(hooks["version"], 1);
        assert_eq!(
            hooks["hooks"]["agentStop"][0]["bash"],
            "'/opt/chda bin/chda' hook copilot agentStop"
        );
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(plugin_dir(&dir).join("plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["hooks"], "hooks.json");

        let cmd = adapter.launch_command(
            Path::new("/work/app"),
            Some(&SessionId("02c60a53".into())),
            Path::new("/usr/local/bin/chda"),
        );
        let argv = crate::command_argv(&cmd);
        assert_eq!(argv[0], "copilot");
        assert_eq!(argv[1], "--plugin-dir");
        assert_eq!(argv.last().unwrap(), "--resume=02c60a53");
        fs::remove_dir_all(&dir).unwrap();
    }
}
