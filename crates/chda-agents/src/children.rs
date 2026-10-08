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

/// Identify a notify sender only from its exact rollout metadata. Bounded
/// directory traversal does not follow symlinks or read conversation bodies.
pub(crate) fn codex_notify_child(root: &std::path::Path, id: &str) -> Option<(String, ChildEvent)> {
    use std::io::{BufRead, Read};
    if id.is_empty() || id.len() > 256 || id.contains(['/', '\\']) {
        return None;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    let mut stack = vec![(root.join("sessions"), 0)];
    let mut visited = 0;
    while let Some((dir, depth)) = stack.pop() {
        for entry in std::fs::read_dir(dir).ok().into_iter().flatten().flatten() {
            visited += 1;
            if visited > 20_000 || std::time::Instant::now() >= deadline {
                return None;
            }
            let kind = entry.file_type().ok()?;
            if kind.is_dir() && depth < 6 {
                stack.push((entry.path(), depth + 1));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "jsonl")
                || path
                    .file_stem()
                    .is_none_or(|s| !s.to_string_lossy().ends_with(id))
            {
                continue;
            }
            let reader = std::io::BufReader::new(std::fs::File::open(&path).ok()?.take(524_288));
            for line in reader.lines().take(50).filter_map(Result::ok) {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if value["type"] != "session_meta" || value["payload"]["id"].as_str() != Some(id) {
                    continue;
                }
                let source = value.pointer("/payload/source/subagent/thread_spawn")?;
                return Some((
                    source.get("parent_thread_id")?.as_str()?.into(),
                    ChildEvent {
                        id: id.into(),
                        label: source
                            .get("agent_nickname")
                            .and_then(serde_json::Value::as_str)
                            .or_else(|| {
                                source.get("agent_role").and_then(serde_json::Value::as_str)
                            })
                            .map(str::to_owned),
                        state: ChildState::TurnComplete,
                        started: false,
                    },
                ));
            }
        }
    }
    None
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
        let dir = root.join("sessions/fixture");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout-fixture-full-child-id.jsonl");
        std::fs::write(file,concat!(r#"{"type":"session_meta","payload":{"id":"full-child-id","source":{"subagent":{"thread_spawn":{"parent_thread_id":"exact-parent","agent_nickname":null,"agent_role":"reviewer"}}}}}"#, "\n", r#"{"type":"response_item","payload":{"content":"private answer"}}"#)).unwrap();
        let (parent, child) = codex_notify_child(&root, "full-child-id").unwrap();
        assert_eq!(parent, "exact-parent");
        assert_eq!(child.id, "full-child-id");
        assert_eq!(child.label.as_deref(), Some("reviewer"));
        assert_eq!(child.state, ChildState::TurnComplete);
        assert!(codex_notify_child(&root, "child-id").is_none());
        assert!(
            !serde_json::to_string(&child)
                .unwrap()
                .contains("private answer")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
