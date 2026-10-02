//! `chda hook <agent>`: the command the agents call. It turns the agent's
//! payload into a [`HookEvent`], hands it to the running app over the
//! local socket, and falls back to an append-only file when no app is up.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AgentId;
use crate::ipc;

/// What happened, reduced to what the status board needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    SessionStart,
    PromptSubmitted,
    /// The agent is waiting for the user (permission or idle prompt).
    WaitingInput,
    /// The agent finished a turn.
    Stopped,
    SessionEnd,
}

/// One status event. No prompt text is kept (design: privacy).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookEvent {
    pub agent: String,
    pub session_id: String,
    pub cwd: PathBuf,
    pub kind: HookKind,
    /// Milliseconds since the Unix epoch, taken when the hook ran.
    pub timestamp: u64,
    /// The chda pane the agent runs in, from [`PANE_ENV`]; absent when the
    /// agent was started outside chda.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<u64>,
}

/// Environment variable chda sets in every pane's shell. Agents inherit it
/// and so do their hooks, which report it back with each event.
pub const PANE_ENV: &str = "CHDA_PANE_ID";

impl HookEvent {
    pub fn agent_id(&self) -> Option<AgentId> {
        AgentId::parse(&self.agent)
    }
}

/// Per-user data directory, shared with the rest of chda.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(dir).join("chda"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/chda"))
    } else {
        Some(home.join(".local/share/chda"))
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Map a Claude Code hook payload (stdin JSON) to an event.
pub fn claude_event(payload: &Value) -> Option<HookEvent> {
    let kind = match payload.get("hook_event_name")?.as_str()? {
        "SessionStart" => HookKind::SessionStart,
        "UserPromptSubmit" => HookKind::PromptSubmitted,
        "PermissionRequest" => HookKind::WaitingInput,
        "Notification" => {
            // Only prompts for the user count as waiting; other notifications
            // (e.g. auth) do not change the status.
            let kind = payload
                .get("notification_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            if kind.is_empty() || kind.contains("permission") || kind.contains("idle") {
                HookKind::WaitingInput
            } else {
                return None;
            }
        }
        "Stop" => HookKind::Stopped,
        "SessionEnd" => HookKind::SessionEnd,
        _ => return None,
    };
    Some(HookEvent {
        agent: "claude".into(),
        session_id: payload.get("session_id")?.as_str()?.to_owned(),
        cwd: PathBuf::from(payload.get("cwd")?.as_str()?),
        kind,
        timestamp: now_ms(),
        pane: None,
    })
}

/// Map a Gemini CLI hook payload (stdin JSON) to an event.
pub fn gemini_event(payload: &Value) -> Option<HookEvent> {
    let kind = match payload.get("hook_event_name")?.as_str()? {
        "SessionStart" => HookKind::SessionStart,
        "BeforeAgent" => HookKind::PromptSubmitted,
        // `ToolPermission` is the only notification Gemini CLI documents.
        "Notification" => match payload.get("notification_type").and_then(Value::as_str) {
            None | Some("ToolPermission") => HookKind::WaitingInput,
            Some(_) => return None,
        },
        "AfterAgent" => HookKind::Stopped,
        "SessionEnd" => HookKind::SessionEnd,
        _ => return None,
    };
    Some(HookEvent {
        agent: "gemini".into(),
        session_id: payload.get("session_id")?.as_str()?.to_owned(),
        cwd: PathBuf::from(payload.get("cwd")?.as_str()?),
        kind,
        timestamp: now_ms(),
        pane: None,
    })
}

/// Map a GitHub Copilot CLI hook payload (stdin JSON) to an event. Its
/// payloads do not name the event, so chda registers one command per event
/// and passes the name as an argument.
pub fn copilot_event(event: &str, payload: &Value) -> Option<HookEvent> {
    let kind = match event {
        "sessionStart" => HookKind::SessionStart,
        "userPromptSubmitted" => HookKind::PromptSubmitted,
        "notification" => match payload.get("notification_type").and_then(Value::as_str) {
            Some("permission_prompt" | "elicitation_dialog") => HookKind::WaitingInput,
            _ => return None,
        },
        "agentStop" => HookKind::Stopped,
        "sessionEnd" => HookKind::SessionEnd,
        _ => return None,
    };
    Some(HookEvent {
        agent: "copilot".into(),
        session_id: payload.get("sessionId")?.as_str()?.to_owned(),
        cwd: PathBuf::from(payload.get("cwd")?.as_str()?),
        kind,
        timestamp: now_ms(),
        pane: None,
    })
}

/// Map the payload of chda's OpenCode plugin (stdin JSON) to an event. The
/// plugin already reduces OpenCode's bus events to a [`HookKind`].
pub fn opencode_event(payload: &Value) -> Option<HookEvent> {
    Some(HookEvent {
        agent: "opencode".into(),
        session_id: payload.get("session_id")?.as_str()?.to_owned(),
        cwd: PathBuf::from(payload.get("cwd")?.as_str()?),
        kind: serde_json::from_value(payload.get("kind")?.clone()).ok()?,
        timestamp: now_ms(),
        pane: None,
    })
}

/// Map a Codex `notify` payload (last argv item, JSON) to an event.
pub fn codex_event(payload: &Value, cwd: &Path) -> Option<HookEvent> {
    let kind = match payload.get("type")?.as_str()? {
        "agent-turn-complete" => HookKind::Stopped,
        _ => return None,
    };
    let session_id = payload
        .get("thread-id")
        .or_else(|| payload.get("thread_id"))
        .or_else(|| payload.get("session-id"))
        .and_then(Value::as_str)?
        .to_owned();
    Some(HookEvent {
        agent: "codex".into(),
        session_id,
        cwd: payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf()),
        kind,
        timestamp: now_ms(),
        pane: None,
    })
}

/// Entry point for `chda hook <agent> [args...]`. Never fails loudly: a
/// hook that errors must not disturb the agent.
pub fn hook_main(args: &[String]) -> i32 {
    let Some(agent) = args.first() else {
        eprintln!("usage: chda hook <claude|codex|gemini|copilot|opencode>");
        return 2;
    };
    let stdin_json = || {
        let mut input = String::new();
        let _ = io::stdin().read_to_string(&mut input);
        serde_json::from_str::<Value>(&input).ok()
    };
    let event = match agent.as_str() {
        "claude" => stdin_json().and_then(|v| claude_event(&v)),
        "gemini" => stdin_json().and_then(|v| gemini_event(&v)),
        "copilot" => {
            let name = args.get(1).map(String::as_str).unwrap_or("");
            stdin_json().and_then(|v| copilot_event(name, &v))
        }
        "opencode" => stdin_json().and_then(|v| opencode_event(&v)),
        "codex" => {
            let json = args.last().filter(|a| a.starts_with('{'));
            let cwd = std::env::current_dir().unwrap_or_default();
            let event = json
                .and_then(|j| serde_json::from_str::<Value>(j).ok())
                .and_then(|v| codex_event(&v, &cwd));
            run_chained_notify(args);
            event
        }
        _ => None,
    };
    if let Some(mut event) = event {
        event.pane = std::env::var(PANE_ENV).ok().and_then(|v| v.parse().ok());
        deliver(&event);
    }
    // Claude Code and Gemini CLI read stdout as JSON; an empty object means
    // "carry on".
    if agent == "claude" || agent == "gemini" {
        println!("{{}}");
    }
    0
}

/// `--then <program> [args...]`: run the user's own Codex notify program with
/// the same payload so chda does not replace it.
fn run_chained_notify(args: &[String]) {
    let Some(pos) = args.iter().position(|a| a == "--then") else {
        return;
    };
    let payload = args.last().cloned().unwrap_or_default();
    let rest = &args[pos + 1..];
    let Some((program, extra)) = rest.split_first() else {
        return;
    };
    let extra: Vec<&String> = extra.iter().filter(|a| **a != payload).collect();
    let _ = Command::new(program).args(extra).arg(&payload).status();
}

/// Send to the running app, or append to the fallback log.
pub fn deliver(event: &HookEvent) {
    let Some(dir) = data_dir() else {
        return;
    };
    if ipc::send(&ipc::socket_path(&dir), event).is_ok() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(line) = serde_json::to_string(event)
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("events.jsonl"))
    {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_agent_payloads_to_events() {
        let e = claude_event(&json!({
            "session_id": "s1", "cwd": "/w", "hook_event_name": "Notification",
            "notification_type": "permission_prompt"
        }))
        .unwrap();
        assert_eq!(e.kind, HookKind::WaitingInput);
        assert_eq!(e.agent_id(), Some(AgentId::Claude));
        assert!(
            claude_event(&json!({
                "session_id": "s1", "cwd": "/w", "hook_event_name": "Notification",
                "notification_type": "auth_success"
            }))
            .is_none()
        );
        assert_eq!(
            claude_event(&json!({"session_id": "s", "cwd": "/w", "hook_event_name": "Stop"}))
                .unwrap()
                .kind,
            HookKind::Stopped
        );
        let c = codex_event(
            &json!({"type": "agent-turn-complete", "thread-id": "t1"}),
            Path::new("/work"),
        )
        .unwrap();
        assert_eq!(
            (c.kind, c.session_id.as_str(), c.cwd.as_path()),
            (HookKind::Stopped, "t1", Path::new("/work"))
        );
        // Events from older hooks have no pane and still parse.
        let old: HookEvent = serde_json::from_str(
            r#"{"agent":"claude","session_id":"s","cwd":"/w","kind":"stopped","timestamp":1}"#,
        )
        .unwrap();
        assert_eq!(old.pane, None);

        let g = gemini_event(&json!({
            "session_id": "g1", "cwd": "/w", "hook_event_name": "Notification",
            "notification_type": "ToolPermission", "timestamp": "2026-10-02T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(
            (g.kind, g.agent_id()),
            (HookKind::WaitingInput, Some(AgentId::Gemini))
        );
        assert_eq!(
            gemini_event(&json!({"session_id": "g", "cwd": "/w", "hook_event_name": "AfterAgent"}))
                .unwrap()
                .kind,
            HookKind::Stopped
        );
        assert!(
            gemini_event(&json!({"session_id": "g", "cwd": "/w", "hook_event_name": "BeforeTool"}))
                .is_none()
        );

        // Payload as Copilot CLI 1.0.91 sends it.
        let start = json!({
            "sessionId": "72b6", "timestamp": 1_790_943_851_694_u64, "cwd": "/w",
            "source": "new", "initialPrompt": "hi"
        });
        let p = copilot_event("sessionStart", &start).unwrap();
        assert_eq!(
            (p.kind, p.session_id.as_str(), p.agent_id()),
            (HookKind::SessionStart, "72b6", Some(AgentId::Copilot))
        );
        let note = |t: &str| {
            copilot_event(
                "notification",
                &json!({"sessionId": "s", "cwd": "/w", "notification_type": t}),
            )
            .map(|e| e.kind)
        };
        assert_eq!(note("permission_prompt"), Some(HookKind::WaitingInput));
        assert_eq!(note("shell_completed"), None);
        assert_eq!(copilot_event("preToolUse", &start), None);

        let o =
            opencode_event(&json!({"kind": "waiting_input", "session_id": "ses_1", "cwd": "/w"}))
                .unwrap();
        assert_eq!(
            (o.kind, o.agent_id()),
            (HookKind::WaitingInput, Some(AgentId::OpenCode))
        );
        assert!(opencode_event(&json!({"kind": "nope", "session_id": "s", "cwd": "/w"})).is_none());
        let line = serde_json::to_string(&HookEvent {
            pane: Some(7),
            ..old
        })
        .unwrap();
        assert!(line.contains(r#""pane":7"#), "{line}");
    }
}
