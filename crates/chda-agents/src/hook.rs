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
}

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

fn now_ms() -> u64 {
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
    })
}

/// Entry point for `chda hook <agent> [args...]`. Never fails loudly: a
/// hook that errors must not disturb the agent.
pub fn hook_main(args: &[String]) -> i32 {
    let Some(agent) = args.first() else {
        eprintln!("usage: chda hook <claude|codex>");
        return 2;
    };
    let event = match agent.as_str() {
        "claude" => {
            let mut input = String::new();
            let _ = io::stdin().read_to_string(&mut input);
            serde_json::from_str::<Value>(&input)
                .ok()
                .and_then(|v| claude_event(&v))
        }
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
    if let Some(event) = event {
        deliver(&event);
    }
    // Claude Code reads stdout as JSON; an empty object means "carry on".
    if agent == "claude" {
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
    }
}
