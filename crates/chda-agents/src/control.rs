//! Requests other chda processes (`chda mcp`) make of the running app, and
//! its replies. They travel over the hook socket next to hook events.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::HookEvent;

/// Something the app should do. `pane` is the chda pane the caller runs in
/// (from `CHDA_PANE_ID`), when it runs in one.
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
}

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
    }
}
