//! Read-only coordination for agents through `chda mcp` (#249): the sessions
//! of the caller's repository, a pane's recent text, and waiting for an
//! agent's status. Nothing here changes focus, rows or seen state.
use super::*;
use chda_core::agents::control::{AGENT_STATES, MAX_WAIT_MS};
use chda_core::{SessionKey, SessionKind};
use serde_json::json;

const DEFAULT_WAIT_MS: u64 = 30_000;
const DEFAULT_LINES: usize = 100;
const MAX_LINES: usize = 500;
const MAX_BYTES: usize = 64 * 1024;
/// Transcript answers read for one `read_output` call.
const ANSWERS: usize = 20;

/// A `wait_for_status` call, kept by the window that owns its pane.
pub(crate) struct StatusWait {
    id: u64,
    pane: PaneId,
    states: Vec<String>,
    started: u64,
    reply: mpsc::Sender<Reply>,
}

/// What `read_output` can return for a pane.
enum Readable {
    /// The exact conversation's latest answers.
    Transcript {
        agent: AgentId,
        session: SessionId,
        roots: Vec<PathBuf>,
    },
    /// A shell's screen up to its current prompt.
    Screen(String),
}

impl WorkspaceView {
    /// The repository the caller's directory belongs to.
    fn caller_repo(&self, cwd: &Path, cx: &App) -> Result<PathBuf, Reply> {
        self.sidebar
            .read(cx)
            .model
            .worktree_for_path(cwd)
            .map(|(repo, _)| repo.path.clone())
            .ok_or_else(|| {
                Reply::err(format!(
                    "{} is not in a repository chda shows; add it to the sidebar first",
                    cwd.display()
                ))
            })
    }

    /// The other windows' views; this one is borrowed while it handles a
    /// request.
    fn other_views(&self) -> Vec<Entity<Self>> {
        self.env
            .windows
            .borrow()
            .entries
            .iter()
            .filter(|e| e.view != self.self_weak)
            .filter_map(|e| e.view.upgrade())
            .collect()
    }

    /// A pane's agent state as callers see it; `None` when the pane is gone.
    fn pane_state(&self, pane: PaneId) -> Option<&'static str> {
        let info = self.ws.pane(pane)?;
        Some(match (&info.agent, info.agent_live) {
            (Some(agent), true) => match agent.status {
                AgentStatus::Working => "working",
                AgentStatus::WaitingInput => "waiting_for_input",
                AgentStatus::Review => "turn_complete",
                AgentStatus::Idle => "idle",
            },
            _ if info.agent.is_some()
                || info.agent_session.is_some()
                || info.agent_launch.is_some() =>
            {
                "ended"
            }
            _ => "terminal",
        })
    }

    /// This window's Sessions rows in `repo`, for `list_sessions`.
    fn session_entries(&self, repo: &Path, now: u64, cx: &App) -> Vec<serde_json::Value> {
        let model = &self.sidebar.read(cx).model;
        let registry = self.env.windows.borrow();
        model
            .sessions
            .iter()
            .filter(|row| matches!(row.kind, SessionKind::Agent | SessionKind::Terminal))
            .filter(|row| {
                row.cwd
                    .as_deref()
                    .and_then(|cwd| model.worktree_for_path(cwd))
                    .is_some_and(|(r, _)| r.path == repo)
            })
            .map(|row| {
                let pane = match row.key {
                    SessionKey::Pane(pane) => Some(pane),
                    SessionKey::Tab(tab) => self
                        .ws
                        .tabs()
                        .iter()
                        .find(|t| t.id == tab)
                        .and_then(|t| t.panes().first().copied()),
                };
                let state = match (row.kind, pane) {
                    (SessionKind::Agent, Some(pane)) => self.pane_state(pane),
                    _ => Some("terminal"),
                };
                let children = pane
                    .and_then(|p| self.ws.pane(p)?.agent_session.clone())
                    .map(|s| registry.children.children(&s.agent, &s.session))
                    .filter(|children| !children.is_empty())
                    .map(|children| {
                        let counts = crate::sidebar_view::ChildCounts::of(&children);
                        json!({
                            "working": counts.working,
                            "waiting_for_input": counts.waiting,
                            "unknown": counts.unknown,
                            "total": counts.total,
                        })
                    });
                json!({
                    "pane": pane.map(PaneId::raw),
                    "kind": if row.kind == SessionKind::Agent { "agent" } else { "terminal" },
                    "agent": row.agent,
                    "provider": row.agent.as_deref().map(|a| self.agent_name(a)),
                    "status": state,
                    "worktree": row.cwd,
                    "repository": row.repo,
                    "branch": row.branch,
                    "label": row.title,
                    "tab": row.tab_title,
                    "pane_number": row.shares_tab.then_some(row.pane_index),
                    "seconds_since_activity": (row.last_activity > 0)
                        .then(|| now.saturating_sub(row.last_activity) / 1000),
                    "children": children,
                })
            })
            .collect()
    }

    pub(super) fn list_sessions(&self, cwd: &Path, cx: &App) -> Reply {
        let repo = match self.caller_repo(cwd, cx) {
            Ok(repo) => repo,
            Err(reply) => return reply,
        };
        let now = now_ms();
        let mut sessions = self.session_entries(&repo, now, cx);
        for view in self.other_views() {
            sessions.extend(view.read(cx).session_entries(&repo, now, cx));
        }
        let count = sessions.len();
        Reply::ok(
            format!(
                "{count} session{} in {}",
                if count == 1 { "" } else { "s" },
                repo.display()
            ),
            json!({ "repository": repo, "sessions": sessions }),
        )
    }

    /// What this window can read from the pane with id `raw`; `None` when
    /// the pane lives in another window.
    fn readable(&self, raw: u64, repo: &Path, cx: &App) -> Option<Result<Readable, String>> {
        let pane = self.ws.pane_by_raw(raw)?;
        let info = self.ws.pane(pane)?;
        if info.repo.as_deref() != Some(repo) {
            return Some(Err(format!("pane {raw} is not in your repository")));
        }
        if info.agent_live {
            let session = info.agent_session.as_ref().and_then(|s| {
                let agent = AgentId::parse(&s.agent)?;
                matches!(agent, AgentId::Claude | AgentId::Codex)
                    .then(|| (agent, SessionId(s.session.clone())))
            });
            return Some(match session {
                Some((agent, session)) => Ok(Readable::Transcript {
                    roots: self
                        .adapters
                        .iter()
                        .find(|a| a.id() == agent)
                        .map(|a| a.session_roots())
                        .unwrap_or_default(),
                    agent,
                    session,
                }),
                None => Err(format!(
                    "pane {raw} runs an agent without a transcript chda can read; its screen may hold unfinished input"
                )),
            });
        }
        let (view, _) = self.panes.get(&pane)?;
        let frame = view.read(cx).frame();
        if frame.alternate_screen {
            return Some(Err(format!(
                "pane {raw} shows a full-screen program; its screen may hold unfinished input"
            )));
        }
        // At a prompt, the prompt rows hold what the user is typing.
        let mut end = frame.size.rows;
        if frame.at_prompt()
            && let Some(cursor) = frame.cursor
        {
            end = cursor.y;
            while end > 0
                && frame.rows[usize::from(end)].semantic_prompt
                    == chda_term::SemanticPrompt::Continuation
            {
                end -= 1;
            }
        }
        let text = (0..end)
            .map(|y| frame.row_text(y))
            .collect::<Vec<_>>()
            .join("\n");
        Some(Ok(Readable::Screen(text)))
    }

    pub(super) fn read_output(
        &self,
        cwd: &Path,
        raw: u64,
        lines: Option<usize>,
        reply: mpsc::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        let repo = match self.caller_repo(cwd, cx) {
            Ok(repo) => repo,
            Err(error) => {
                let _ = reply.send(error);
                return;
            }
        };
        let lines = lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
        let readable = self.readable(raw, &repo, cx).or_else(|| {
            self.other_views()
                .into_iter()
                .find_map(|view| view.read(cx).readable(raw, &repo, cx))
        });
        let finish = move |source: &str, text: String| {
            let text = tail(&text, lines);
            Reply::ok(
                format!("{} lines from pane {raw} ({source})", text.lines().count()),
                json!({ "pane": raw, "source": source, "text": text }),
            )
        };
        match readable {
            None => {
                let _ = reply.send(Reply::err(format!("chda has no pane {raw}")));
            }
            Some(Err(why)) => {
                let _ = reply.send(Reply::err(why));
            }
            Some(Ok(Readable::Screen(text))) => {
                let _ = reply.send(finish("screen", text));
            }
            Some(Ok(Readable::Transcript {
                agent,
                session,
                roots,
            })) => {
                // Transcripts are files; read them off the UI thread.
                cx.background_spawn(async move {
                    let answer =
                        match chda_core::agents::recent_answers(agent, &session, &roots, ANSWERS) {
                            Some(answers) => finish("transcript", answers.join("\n\n")),
                            None => Reply::err(format!(
                                "the transcript of pane {raw}'s conversation could not be read"
                            )),
                        };
                    let _ = reply.send(answer);
                })
                .detach();
            }
        }
    }

    pub(super) fn wait_for_status(
        &mut self,
        cwd: &Path,
        raw: u64,
        states: Vec<String>,
        timeout_ms: Option<u64>,
        reply: mpsc::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        let repo = match self.caller_repo(cwd, cx) {
            Ok(repo) => repo,
            Err(error) => {
                let _ = reply.send(error);
                return;
            }
        };
        if states.is_empty() || states.iter().any(|s| !AGENT_STATES.contains(&s.as_str())) {
            let _ = reply.send(Reply::err(format!(
                "`states` must list some of {}",
                AGENT_STATES.join(", ")
            )));
            return;
        }
        let timeout = timeout_ms.unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS);
        if self.ws.pane_by_raw(raw).is_some() {
            self.start_status_wait(raw, &repo, states, timeout, reply, cx);
            return;
        }
        let owner = self
            .other_views()
            .into_iter()
            .find(|view| view.read(cx).ws.pane_by_raw(raw).is_some());
        match owner {
            Some(view) => view.update(cx, |v, cx| {
                v.start_status_wait(raw, &repo, states, timeout, reply, cx)
            }),
            None => {
                let _ = reply.send(Reply::err(format!("chda has no pane {raw}")));
            }
        }
    }

    fn start_status_wait(
        &mut self,
        raw: u64,
        repo: &Path,
        states: Vec<String>,
        timeout: u64,
        reply: mpsc::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.ws.pane_by_raw(raw) else {
            let _ = reply.send(Reply::err(format!("chda has no pane {raw}")));
            return;
        };
        if self.ws.pane(pane).and_then(|i| i.repo.as_deref()) != Some(repo) {
            let _ = reply.send(Reply::err(format!("pane {raw} is not in your repository")));
            return;
        }
        let state = self.pane_state(pane).unwrap_or("closed");
        if states.iter().any(|s| s == state) {
            let _ = reply.send(reached(raw, state, 0));
            return;
        }
        self.next_status_wait += 1;
        let id = self.next_status_wait;
        self.status_waits.push(StatusWait {
            id,
            pane,
            states,
            started: now_ms(),
            reply,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(timeout))
                .await;
            let _ = this.update(cx, |view, _| view.expire_status_wait(id));
        })
        .detach();
    }

    /// Answer the waits whose pane reached a state they wait for or closed.
    pub(super) fn resolve_status_waits(&mut self) {
        if self.status_waits.is_empty() {
            return;
        }
        let now = now_ms();
        let waits = std::mem::take(&mut self.status_waits);
        for wait in waits {
            let raw = wait.pane.raw();
            let waited = now.saturating_sub(wait.started);
            match self.pane_state(wait.pane) {
                None => {
                    let _ = wait.reply.send(Reply::ok(
                        format!("Pane {raw} closed"),
                        json!({ "pane": raw, "state": "closed", "waited_ms": waited }),
                    ));
                }
                Some(state) if wait.states.iter().any(|s| s == state) => {
                    let _ = wait.reply.send(reached(raw, state, waited));
                }
                Some(_) => self.status_waits.push(wait),
            }
        }
    }

    fn expire_status_wait(&mut self, id: u64) {
        let Some(index) = self.status_waits.iter().position(|w| w.id == id) else {
            return;
        };
        let wait = self.status_waits.remove(index);
        let raw = wait.pane.raw();
        let state = self.pane_state(wait.pane).unwrap_or("closed");
        let waited = now_ms().saturating_sub(wait.started);
        let _ = wait.reply.send(Reply::ok(
            format!(
                "Still {state} after {}s; call again to keep waiting",
                waited / 1000
            ),
            json!({ "pane": raw, "state": state, "timed_out": true, "waited_ms": waited }),
        ));
    }
}

fn reached(raw: u64, state: &str, waited: u64) -> Reply {
    Reply::ok(
        format!("Pane {raw} is {state}"),
        json!({ "pane": raw, "state": state, "timed_out": false, "waited_ms": waited }),
    )
}

/// The last `lines` lines of `text`, without trailing blank lines, at most
/// [`MAX_BYTES`] long.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.trim_end().lines().collect();
    let mut out = all[all.len().saturating_sub(lines)..].join("\n");
    if out.len() > MAX_BYTES {
        let mut cut = out.len() - MAX_BYTES;
        while !out.is_char_boundary(cut) {
            cut += 1;
        }
        out = out[cut..].to_owned();
    }
    out
}
