//! Requests other chda processes (`chda mcp`) make of the running app, and
//! its replies. They travel over the hook socket next to hook events.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::HookEvent;

/// Something the app should do. `cwd` is the caller's working directory;
/// read-only requests answer only for the repository it is in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Request {
    /// Create a worktree in the repository containing `cwd`. Without a
    /// branch a random name is picked; with `base` the new branch starts
    /// there instead of at HEAD. `open` opens a tab in it, running `agent`
    /// when given (an agent id), else the configured default action.
    CreateWorktree {
        cwd: PathBuf,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        base: Option<String>,
        #[serde(default)]
        note: Option<String>,
        #[serde(default)]
        open: bool,
        #[serde(default)]
        agent: Option<String>,
    },
    /// Open a tab in `path`, running `agent` (an agent id) or a shell.
    OpenTab {
        path: PathBuf,
        #[serde(default)]
        agent: Option<String>,
    },
    /// The worktrees of the repository containing `cwd`, or of every
    /// repository in the sidebar when `cwd` is in none.
    ListWorktrees { cwd: PathBuf },
    /// Agent sessions and terminals in the repository containing `cwd`.
    ListSessions { cwd: PathBuf },
    /// Recent text of the pane chda knows as `pane`, at most `lines` lines,
    /// never what the user is still typing.
    ReadOutput {
        cwd: PathBuf,
        pane: u64,
        #[serde(default)]
        lines: Option<usize>,
    },
    /// Answer once the agent in `pane` reaches one of `states`, the pane
    /// closes or `timeout_ms` passes.
    WaitForStatus {
        cwd: PathBuf,
        pane: u64,
        states: Vec<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
}

/// Agent states a caller can wait for, as `list_sessions` reports them.
pub const AGENT_STATES: [&str; 5] = [
    "working",
    "waiting_for_input",
    "turn_complete",
    "idle",
    "ended",
];

/// Longest wait one request may ask for, below the socket's reply timeout.
pub const MAX_WAIT_MS: u64 = 55_000;

/// The app's answer to a [`Request`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    /// One line for the user or the model.
    pub message: String,
    /// Structured result, e.g. the new worktree's path.
    #[serde(default)]
    pub data: Value,
}

impl Reply {
    pub fn ok(message: impl Into<String>, data: Value) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            data: Value::Null,
        }
    }
}

/// What the app receives on the socket.
#[derive(Debug)]
pub enum Incoming {
    Quota(crate::quota::QuotaSnapshot),
    Event(HookEvent),
    /// A request and where its reply goes; the caller waits for it.
    Request(Request, Sender<Reply>),
}

/// How a request is framed on the socket, so it cannot be mistaken for a
/// hook event.
#[derive(Serialize, Deserialize)]
pub(crate) struct Envelope {
    pub request: Request,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_have_a_stable_wire_form() {
        let r: Request = serde_json::from_str(
            r#"{"action":"create_worktree","cwd":"/w","branch":"feat/x","open":true}"#,
        )
        .unwrap();
        assert_eq!(
            r,
            Request::CreateWorktree {
                cwd: "/w".into(),
                branch: Some("feat/x".into()),
                base: None,
                note: None,
                open: true,
                agent: None,
            }
        );
        let line = serde_json::to_string(&Envelope {
            request: Request::ListWorktrees { cwd: "/w".into() },
        })
        .unwrap();
        assert_eq!(
            line,
            r#"{"request":{"action":"list_worktrees","cwd":"/w"}}"#
        );
        assert!(serde_json::from_str::<HookEvent>(&line).is_err());
        let wait: Request = serde_json::from_str(
            r#"{"action":"wait_for_status","cwd":"/w","pane":7,"states":["turn_complete"]}"#,
        )
        .unwrap();
        assert_eq!(
            wait,
            Request::WaitForStatus {
                cwd: "/w".into(),
                pane: 7,
                states: vec!["turn_complete".into()],
                timeout_ms: None,
            }
        );
        assert_eq!(
            serde_json::to_string(&Request::ReadOutput {
                cwd: "/w".into(),
                pane: 7,
                lines: Some(20),
            })
            .unwrap(),
            r#"{"action":"read_output","cwd":"/w","pane":7,"lines":20}"#
        );
    }
}
