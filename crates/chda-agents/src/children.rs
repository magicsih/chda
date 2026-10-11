//! Provider-confirmed child identities and status, without conversation content.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildState {
    Unknown,
    Working,
    WaitingInput,
    TurnComplete,
    Ended,
}
impl ChildState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Working => "Working",
            Self::WaitingInput => "Waiting for input",
            Self::TurnComplete => "Turn complete",
            Self::Ended => "Ended",
        }
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Working | Self::WaitingInput)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildEvent {
    pub id: String,
    pub label: Option<String>,
    pub state: ChildState,
    #[serde(default)]
    pub started: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildActivity {
    pub agent: String,
    pub parent: String,
    pub child: ChildEvent,
    pub observed_at: u64,
}

/// App memory and live handoff only: never added to a cold session save.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildBoard {
    entries: Vec<ChildActivity>,
}
impl ChildBoard {
    pub fn children(&self, agent: &str, parent: &str) -> Vec<ChildActivity> {
        self.entries
            .iter()
            .filter(|e| e.agent == agent && e.parent == parent)
            .cloned()
            .collect()
    }
    pub fn apply(&mut self, event: ChildActivity) -> bool {
        fn identity(s: &str) -> bool {
            !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
        }
        if !matches!(event.agent.as_str(), "claude" | "codex")
            || !identity(&event.parent)
            || !identity(&event.child.id)
            || event.parent == event.child.id
            || event.observed_at == 0
            || event
                .child
                .label
                .as_ref()
                .is_some_and(|s| s.len() > 160 || s.chars().any(char::is_control))
        {
            return false;
        }
        if let Some(old) = self.entries.iter_mut().find(|e| {
            e.agent == event.agent && e.parent == event.parent && e.child.id == event.child.id
        }) {
            fn rank(state: ChildState) -> u8 {
                match state {
                    ChildState::Unknown => 0,
                    ChildState::Working => 1,
                    ChildState::WaitingInput => 2,
                    ChildState::TurnComplete => 3,
                    ChildState::Ended => 4,
                }
            }
            if event.observed_at < old.observed_at
                || (event.observed_at == old.observed_at
                    && rank(event.child.state) <= rank(old.child.state))
                || (old.child.state == ChildState::Ended && !event.child.started)
            {
                return false;
            }
            let changed =
                old.child.state != event.child.state || old.child.label != event.child.label;
            *old = event;
            return changed;
        }
        const LIMIT: usize = 1024;
        if self.entries.len() == LIMIT {
            let i = self
                .entries
                .iter()
                .position(|e| !e.child.state.active())
                .unwrap_or(0);
            self.entries.remove(i);
        }
        self.entries.push(event);
        true
    }
}

/// Metadata-only read from the daemon used by the installed interactive CLI.
/// `proxy` connects to an existing server; it does not start a quota server or a conversation.
pub fn read_codex_children(
    executable: &std::path::Path,
    env: &[(String, String)],
    parents: &[String],
    at: u64,
) -> std::io::Result<Vec<ChildActivity>> {
    use serde_json::json;
    let mut command = std::process::Command::new(executable);
    command
        .args(["app-server", "proxy"])
        .envs(env.iter().cloned());
    let parents = parents.to_vec();
    crate::ipc::with_local_rpc(
        &mut command,
        std::time::Duration::from_secs(5),
        move |rpc| {
            let mut out = Vec::new();
            for parent in parents.iter().take(64) {
                let mut cursor = None;
                for _ in 0..8 {
                    let page = rpc.request("thread/list", json!({"parentThreadId":parent,"sourceKinds":["subAgentThreadSpawn"],"useStateDbOnly":true,"limit":100,"cursor":cursor}))?;
                    for thread in page
                        .get("data")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(event) = codex_thread(thread, parent, at) {
                            out.push(event);
                        }
                    }
                    cursor = page
                        .get("nextCursor")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    if cursor.is_none() {
                        break;
                    }
                }
            }
            Ok(out)
        },
    )
}

fn codex_thread(thread: &serde_json::Value, parent: &str, at: u64) -> Option<ChildActivity> {
    use serde_json::Value;
    let source = thread.pointer("/source/subagent/thread_spawn");
    let confirmed_parent = thread
        .get("parentThreadId")
        .and_then(Value::as_str)
        .or_else(|| source?.get("parent_thread_id")?.as_str())?;
    if confirmed_parent != parent {
        return None;
    }
    if source
        .and_then(|s| s.get("parent_thread_id"))
        .and_then(Value::as_str)
        .is_some_and(|s| s != confirmed_parent)
    {
        return None;
    }
    let id = thread.get("id")?.as_str()?;
    let state = match thread.pointer("/status/type").and_then(Value::as_str) {
        Some("active") => {
            if thread
                .pointer("/status/activeFlags")
                .and_then(Value::as_array)
                .is_some_and(|flags| {
                    flags.iter().any(|f| {
                        matches!(f.as_str(), Some("waitingOnApproval" | "waitingOnUserInput"))
                    })
                })
            {
                ChildState::WaitingInput
            } else {
                ChildState::Working
            }
        }
        Some("idle") => ChildState::TurnComplete,
        // NotLoaded/SystemError do not prove a process exit.
        _ => ChildState::Unknown,
    };
    Some(ChildActivity {
        agent: "codex".into(),
        parent: parent.into(),
        child: ChildEvent {
            id: id.into(),
            label: thread
                .get("agentNickname")
                .and_then(Value::as_str)
                .or_else(|| thread.get("agentRole").and_then(Value::as_str))
                .map(str::to_owned),
            state,
            started: false,
        },
        observed_at: at,
    })
}

/// What Codex's saved transcripts say about the thread that sent a `notify`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CodexNotifySender {
    /// A provider-confirmed child, reported under its exact parent.
    Child { parent: String, child: ChildEvent },
    /// A conversation of its own, saved under `sessions`.
    TopLevel,
    /// No transcript exists for the thread: Codex saves every conversation
    /// before its first turn completes, so this is an ephemeral helper, such
    /// as the title request on a new conversation's first turn.
    NotSaved,
    /// The transcripts could not be searched to the end or read.
    Incomplete,
}

/// Classify a notify sender only from its exact rollout metadata. Newest
/// dated directories are searched first; bounded traversal does not follow
/// symlinks or read conversation bodies. Compressed (cold) transcripts are not
/// read: a thread that reports a turn has a plain one.
pub(crate) fn codex_notify_sender(root: &std::path::Path, id: &str) -> CodexNotifySender {
    use std::io::{BufRead, Read};
    if id.is_empty() || id.len() > 256 || id.contains(['/', '\\']) {
        return CodexNotifySender::Incomplete;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    let mut stack = vec![(root.join("sessions"), 0)];
    let mut visited = 0;
    let mut unreadable = false;
    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if depth == 0 && e.kind() == std::io::ErrorKind::NotFound => {
                return CodexNotifySender::NotSaved;
            }
            Err(_) => return CodexNotifySender::Incomplete,
        };
        let Ok(mut entries) = entries.collect::<Result<Vec<_>, _>>() else {
            return CodexNotifySender::Incomplete;
        };
        // Ascending here, so the newest (largest) name is popped first.
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            visited += 1;
            if visited > 20_000 || std::time::Instant::now() >= deadline {
                return CodexNotifySender::Incomplete;
            }
            let Ok(kind) = entry.file_type() else {
                return CodexNotifySender::Incomplete;
            };
            if kind.is_dir() && depth < 6 {
                stack.push((entry.path(), depth + 1));
                continue;
            }
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".jsonl")) else {
                continue;
            };
            // `rollout-<time>-<thread>.jsonl`, or `-<thread>_<rollout>` after a revert.
            if !kind.is_file()
                || !(stem.ends_with(&format!("-{id}")) || stem.contains(&format!("-{id}_")))
            {
                continue;
            }
            let Ok(file) = std::fs::File::open(entry.path()) else {
                unreadable = true;
                continue;
            };
            let reader = std::io::BufReader::new(file.take(524_288));
            let meta = reader
                .lines()
                .take(50)
                .map_while(Result::ok)
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
                .find(|value| value["type"] == "session_meta");
            let Some(meta) = meta else {
                unreadable = true;
                continue;
            };
            if meta["payload"]["id"].as_str() != Some(id) {
                continue;
            }
            let Some(source) = meta.pointer("/payload/source/subagent/thread_spawn") else {
                return CodexNotifySender::TopLevel;
            };
            let Some(parent) = source.get("parent_thread_id").and_then(|p| p.as_str()) else {
                return CodexNotifySender::Incomplete;
            };
            return CodexNotifySender::Child {
                parent: parent.into(),
                child: ChildEvent {
                    id: id.into(),
                    label: source
                        .get("agent_nickname")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| source.get("agent_role").and_then(serde_json::Value::as_str))
                        .map(str::to_owned),
                    state: ChildState::TurnComplete,
                    started: false,
                },
            };
        }
    }
    if unreadable {
        CodexNotifySender::Incomplete
    } else {
        CodexNotifySender::NotSaved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(parent: &str, id: &str, state: ChildState, at: u64) -> ChildActivity {
        ChildActivity {
            agent: "claude".into(),
            parent: parent.into(),
            child: ChildEvent {
                id: id.into(),
                label: Some("Explore".into()),
                state,
                started: false,
            },
            observed_at: at,
        }
    }
    #[test]
    fn exact_parents_late_events_and_ended_children_keep_stable_identity() {
        let mut board = ChildBoard::default();
        assert!(board.apply(event("p1", "c1", ChildState::Working, 10)));
        assert!(board.apply(event("p2", "c1", ChildState::WaitingInput, 11)));
        assert_eq!(
            board.children("claude", "p1")[0].child.state,
            ChildState::Working
        );
        assert!(board.apply(event("p1", "c1", ChildState::TurnComplete, 12)));
        assert!(!board.apply(event("p1", "c1", ChildState::Working, 10)));
        assert!(!board.apply(event("p1", "c1", ChildState::TurnComplete, 12)));
        assert!(board.apply(event("p1", "c1", ChildState::Ended, 14)));
        assert!(!board.apply(event("p1", "c1", ChildState::Working, 15)));
        assert!(!board.children("claude", "p1")[0].child.state.active());
        assert_eq!(
            board.children("claude", "p2")[0].child.state,
            ChildState::WaitingInput
        );
        let mut resumed = event("p1", "c1", ChildState::Working, 16);
        resumed.child.started = true;
        assert!(board.apply(resumed));
        assert_eq!(board.children("claude", "p1").len(), 1);
        assert!(ChildBoard::default().children("claude", "p1").is_empty());
    }
    #[test]
    fn codex_metadata_confirms_the_parent_and_only_current_runtime_status() {
        use serde_json::json;
        let mut thread = json!({"id":"child","parentThreadId":"parent","status":{"type":"active","activeFlags":[]},"preview":"private prompt","agentNickname":"reviewer"});
        assert!(codex_thread(&thread, "other", 10).is_none());
        let event = codex_thread(&thread, "parent", 10).unwrap();
        assert_eq!(event.child.state, ChildState::Working);
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains("private prompt")
        );
        thread["status"]["activeFlags"] = json!(["waitingOnApproval"]);
        assert_eq!(
            codex_thread(&thread, "parent", 11).unwrap().child.state,
            ChildState::WaitingInput
        );
        thread["status"] = json!({"type":"idle"});
        assert_eq!(
            codex_thread(&thread, "parent", 12).unwrap().child.state,
            ChildState::TurnComplete
        );
        thread["status"] = json!({"type":"notLoaded"});
        assert_eq!(
            codex_thread(&thread, "parent", 13).unwrap().child.state,
            ChildState::Unknown
        );
    }
    #[test]
    fn claude_hooks_keep_child_work_and_turn_completion_out_of_parent_state_and_logs() {
        use serde_json::json;
        for (name, expected) in [
            ("SubagentStart", ChildState::Working),
            ("PreToolUse", ChildState::Working),
            ("PermissionRequest", ChildState::WaitingInput),
            ("SubagentStop", ChildState::TurnComplete),
            ("SessionEnd", ChildState::Ended),
        ] {
            let event = crate::hook::claude_event(&json!({"hook_event_name":name,"session_id":"parent","cwd":"/fixture","agent_id":"child","agent_type":"Explore","tool_input":{"prompt":"secret prompt"},"last_assistant_message":"private answer"})).unwrap();
            assert_eq!(event.session_id, "parent");
            assert_eq!(event.child.as_ref().unwrap().state, expected);
            let encoded = serde_json::to_string(&event).unwrap();
            assert!(!encoded.contains("secret prompt") && !encoded.contains("private answer"));
        }
        assert!(
            crate::hook::claude_event(
                &json!({"hook_event_name":"PreToolUse","session_id":"parent","cwd":"/fixture"})
            )
            .is_none()
        );
    }
    #[test]
    fn codex_notify_uses_full_identity_and_provider_parent_metadata() {
        let root = std::env::temp_dir().join(format!("chda-child-metadata-{}", std::process::id()));
        let dir = root.join("sessions/2026/10/11");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout-2026-10-11T09-00-00-full-child-id.jsonl");
        std::fs::write(file,concat!(r#"{"type":"session_meta","payload":{"id":"full-child-id","source":{"subagent":{"thread_spawn":{"parent_thread_id":"exact-parent","agent_nickname":null,"agent_role":"reviewer"}}}}}"#, "\n", r#"{"type":"response_item","payload":{"content":"private answer"}}"#)).unwrap();
        let CodexNotifySender::Child { parent, child } =
            codex_notify_sender(&root, "full-child-id")
        else {
            panic!("a child rollout identifies its parent");
        };
        assert_eq!(parent, "exact-parent");
        assert_eq!(child.id, "full-child-id");
        assert_eq!(child.label.as_deref(), Some("reviewer"));
        assert_eq!(child.state, ChildState::TurnComplete);
        assert!(
            !serde_json::to_string(&child)
                .unwrap()
                .contains("private answer")
        );
        // A suffix of another thread's identity is not that thread.
        assert_eq!(
            codex_notify_sender(&root, "child-id"),
            CodexNotifySender::NotSaved
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_notify_senders_without_a_saved_transcript_are_told_apart() {
        let root = std::env::temp_dir().join(format!("chda-notify-sender-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Before Codex saved anything, no thread has a transcript.
        assert_eq!(
            codex_notify_sender(&root, "title-helper"),
            CodexNotifySender::NotSaved
        );
        let day = root.join("sessions/2026/10/11");
        std::fs::create_dir_all(&day).unwrap();
        let meta = |id: &str| {
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"source\":\"cli\"}}}}\n"
            )
        };
        std::fs::write(
            day.join("rollout-2026-10-11T09-00-00-conversation.jsonl"),
            meta("conversation"),
        )
        .unwrap();
        // A reverted conversation continues in a rollout named after both ids.
        std::fs::write(
            day.join("rollout-2026-10-11T10-00-00-reverted_new-rollout.jsonl"),
            meta("reverted"),
        )
        .unwrap();
        assert_eq!(
            codex_notify_sender(&root, "conversation"),
            CodexNotifySender::TopLevel
        );
        assert_eq!(
            codex_notify_sender(&root, "reverted"),
            CodexNotifySender::TopLevel
        );
        // Codex's title request on a new conversation's first turn is
        // ephemeral: it notifies, but nothing is saved for it.
        assert_eq!(
            codex_notify_sender(&root, "title-helper"),
            CodexNotifySender::NotSaved
        );
        // A matching file whose metadata cannot be read proves nothing.
        std::fs::write(
            day.join("rollout-2026-10-11T11-00-00-partial.jsonl"),
            "{\"type\":\"response_item\"",
        )
        .unwrap();
        assert_eq!(
            codex_notify_sender(&root, "partial"),
            CodexNotifySender::Incomplete
        );
        assert_eq!(
            codex_notify_sender(&root, "../conversation"),
            CodexNotifySender::Incomplete
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
