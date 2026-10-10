//! The sidebar's Sessions list: one row per agent pane and one per other
//! tab. Rows keep their place whatever their status; only tabs and panes
//! opening, closing or gaining an agent change the list.

use std::path::PathBuf;

use crate::{AgentStatus, PaneId, Sidebar, Tab, TabContent, TabId, Workspace};

/// What a row stands for. Status changes never change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionKey {
    /// A pane running, or launched with, a coding agent.
    Pane(PaneId),
    /// A tab without such a pane.
    Tab(TabId),
}

/// A tab that shows something other than a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    GitTree,
    DiffReview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    /// An agent pane; split panes with agents get a row each.
    Agent,
    /// A terminal tab with no agent pane. Listed last and closable.
    Terminal,
    View(ViewKind),
}

/// One Sessions row. Rebuilt from the workspace, never saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRow {
    pub key: SessionKey,
    pub tab: TabId,
    pub kind: SessionKind,
    /// Agent id of an agent row, e.g. `claude`.
    pub agent: Option<String>,
    /// A runtime event named this pane, so a live process holds the agent.
    pub agent_live: bool,
    /// The status icon; `None` is the gray dot of a row without a live agent.
    pub status: Option<AgentStatus>,
    /// When the agent's current status began; zero when unknown.
    pub since: u64,
    /// Branch alias or branch name, as the label setting asks, else the tab
    /// title.
    pub title: String,
    pub branch: Option<String>,
    /// Repository name.
    pub repo: Option<String>,
    pub cwd: Option<PathBuf>,
    pub tab_title: String,
    /// The pane's 1-based place in its tab; zero for tab rows.
    pub pane_index: usize,
    /// Another agent row shares this tab, so the pane number tells them apart.
    pub shares_tab: bool,
    pub last_activity: u64,
    pub previous_activity: Option<u64>,
}

impl SessionRow {
    /// What the status icon means, in words, so color is not the only cue.
    pub fn status_label(&self) -> &'static str {
        match (self.status, self.kind) {
            (Some(AgentStatus::Working), _) => "Working",
            (Some(AgentStatus::WaitingInput), _) => "Waiting for input",
            (Some(AgentStatus::Review), _) => "Turn complete",
            (Some(AgentStatus::Idle), _) => "Idle",
            (None, SessionKind::Agent) => "No live agent",
            (None, SessionKind::Terminal) => "Terminal",
            (None, SessionKind::View(ViewKind::GitTree)) => "Git tree",
            (None, SessionKind::View(ViewKind::DiffReview)) => "Diff review",
        }
    }

    /// A live agent ready for another task: its pane can be closed from the row.
    pub fn live_idle(&self) -> bool {
        self.kind == SessionKind::Agent && self.agent_live && self.status == Some(AgentStatus::Idle)
    }

    /// Repository and branch, else the directory.
    pub fn location(&self) -> String {
        match (&self.repo, &self.branch) {
            (Some(repo), branch) => {
                format!("{repo} / {}", branch.as_deref().unwrap_or("detached HEAD"))
            }
            (None, _) => self
                .cwd
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Directory unavailable".into()),
        }
    }
}

/// Whether a pane gets an agent row.
fn agent_pane(ws: &Workspace, pane: PaneId) -> bool {
    ws.pane(pane)
        .is_some_and(|i| i.agent_live || i.agent_launch.is_some())
}

/// Build the Sessions rows: agent and view rows in tab order, agent rows in
/// pane order within their tab, then terminal rows in tab order.
pub fn session_rows(
    ws: &Workspace,
    sidebar: &Sidebar,
    label: chda_config::ActiveLabel,
) -> Vec<SessionRow> {
    let repo_name = |path: Option<PathBuf>| {
        let path = path?;
        sidebar
            .repos
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.name.clone())
    };
    let title = |cwd: Option<&PathBuf>, branch: Option<&String>, fallback: String| {
        let alias = cwd
            .and_then(|cwd| sidebar.worktree_for_path(cwd))
            .and_then(|(_, w)| w.note_title());
        match label {
            chda_config::ActiveLabel::Alias => alias
                .map(str::to_owned)
                .or_else(|| branch.cloned())
                .unwrap_or(fallback),
            chda_config::ActiveLabel::Branch => branch.cloned().unwrap_or(fallback),
        }
    };
    let mut rows = Vec::new();
    let mut terminals = Vec::new();
    for tab in ws.tabs() {
        let tab_title = ws.tab_title(tab);
        let panes = tab.panes();
        let agents: Vec<(usize, PaneId)> = panes
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, p)| agent_pane(ws, *p))
            .collect();
        for (index, pane) in &agents {
            let Some(info) = ws.pane(*pane) else {
                continue;
            };
            let status = match (&info.agent, info.agent_live) {
                (Some(a), true) => Some(a.status),
                (Some(a), false) => (a.status != AgentStatus::Idle).then_some(a.status),
                (None, true) => Some(AgentStatus::Idle),
                (None, false) => None,
            };
            let agent = info
                .agent
                .as_ref()
                .map(|a| a.agent.clone())
                .or_else(|| {
                    info.agent_launch
                        .as_ref()
                        .map(|l| l.agent.as_str().to_owned())
                })
                .or_else(|| info.agent_session.as_ref().map(|s| s.agent.clone()));
            rows.push(SessionRow {
                key: SessionKey::Pane(*pane),
                tab: tab.id,
                kind: SessionKind::Agent,
                agent,
                agent_live: info.agent_live,
                status,
                since: info.agent.as_ref().map_or(0, |a| a.since),
                title: title(info.cwd.as_ref(), info.branch.as_ref(), tab_title.clone()),
                branch: info.branch.clone(),
                repo: repo_name(info.repo.clone()),
                cwd: info.cwd.clone(),
                tab_title: tab_title.clone(),
                pane_index: index + 1,
                shares_tab: agents.len() > 1,
                last_activity: info.last_activity,
                previous_activity: info.previous_activity,
            });
        }
        if !agents.is_empty() {
            continue;
        }
        let kind = match &tab.content {
            TabContent::Terminal(_) => SessionKind::Terminal,
            TabContent::GitGraph { .. } => SessionKind::View(ViewKind::GitTree),
            TabContent::DiffReview { .. } => SessionKind::View(ViewKind::DiffReview),
        };
        let focused = tab.focused_pane().and_then(|p| ws.pane(p));
        let branch = focused.and_then(|i| i.branch.clone());
        let cwd = focused
            .and_then(|i| i.cwd.clone())
            .or_else(|| tab.directory().map(PathBuf::from));
        let (last_activity, previous_activity) = tab_activity(ws, tab);
        let row = SessionRow {
            key: SessionKey::Tab(tab.id),
            tab: tab.id,
            kind,
            agent: None,
            agent_live: false,
            // Hook reports that named no pane can still show attention.
            status: ws
                .tab_agent(tab)
                .map(|a| a.status)
                .filter(|s| *s != AgentStatus::Idle),
            since: 0,
            title: title(
                focused.and_then(|i| i.cwd.as_ref()),
                branch.as_ref(),
                tab_title.clone(),
            ),
            branch,
            repo: repo_name(ws.tab_repo(tab)),
            cwd,
            tab_title,
            pane_index: 0,
            shares_tab: false,
            last_activity,
            previous_activity,
        };
        if kind == SessionKind::Terminal {
            terminals.push(row);
        } else {
            rows.push(row);
        }
    }
    rows.extend(terminals);
    rows
}

/// Latest output in any of a tab's panes, and the latest work before a restart.
fn tab_activity(ws: &Workspace, tab: &Tab) -> (u64, Option<u64>) {
    let panes = tab.panes();
    let infos = || panes.iter().filter_map(|p| ws.pane(*p));
    (
        infos().map(|i| i.last_activity).max().unwrap_or(0),
        infos().filter_map(|i| i.previous_activity).max(),
    )
}

/// Refresh activity times in place: output never adds, removes or moves a
/// row. Returns whether any time changed.
pub fn update_session_activity(rows: &mut [SessionRow], ws: &Workspace) -> bool {
    let mut changed = false;
    for row in rows {
        let at = match row.key {
            SessionKey::Pane(pane) => ws.pane(pane).map(|i| i.last_activity),
            SessionKey::Tab(id) => ws
                .tabs()
                .iter()
                .find(|t| t.id == id)
                .map(|t| tab_activity(ws, t).0),
        };
        if let Some(at) = at
            && at != row.last_activity
        {
            row.last_activity = at;
            changed = true;
        }
    }
    changed
}

/// The row that holds the focus: the focused pane's own row, else the
/// tab's first row.
pub fn focused_session(
    rows: &[SessionRow],
    tab: TabId,
    pane: Option<PaneId>,
) -> Option<SessionKey> {
    pane.and_then(|p| {
        rows.iter()
            .find(|r| r.key == SessionKey::Pane(p))
            .map(|r| r.key)
    })
    .or_else(|| rows.iter().find(|r| r.tab == tab).map(|r| r.key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Axis, PaneAgent};
    use chda_config::ActiveLabel;
    use std::path::Path;

    fn agent(ws: &mut Workspace, pane: PaneId, id: &str, status: AgentStatus, live: bool) {
        let info = ws.pane_mut(pane).unwrap();
        info.agent = Some(PaneAgent {
            agent: id.into(),
            status,
            since: 7,
            seen: false,
        });
        info.agent_live = live;
    }

    fn keys(rows: &[SessionRow]) -> Vec<SessionKey> {
        rows.iter().map(|r| r.key).collect()
    }

    /// Terminal tab, agent tab with two agent splits and a shell split,
    /// Git tree, another terminal and a diff review.
    fn fixture() -> (Workspace, Vec<TabId>, Vec<PaneId>) {
        let mut ws = Workspace::new();
        let (shell, shell_pane) = ws.new_tab();
        let (agents, first) = ws.new_tab();
        let second = ws.split(Axis::Horizontal).unwrap();
        let plain = ws.split(Axis::Vertical).unwrap();
        let graph = ws.open_graph("/src/app".into());
        let (other, other_pane) = ws.new_tab();
        let review = ws.open_review("/src/app".into(), "/src/app".into(), "main".into());
        agent(&mut ws, first, "claude", AgentStatus::Working, true);
        agent(&mut ws, second, "codex", AgentStatus::Idle, true);
        (
            ws,
            vec![shell, agents, graph, other, review],
            vec![shell_pane, first, second, plain, other_pane],
        )
    }

    #[test]
    fn agents_and_views_follow_tab_and_pane_order_then_terminals() {
        let (ws, tabs, panes) = fixture();
        let rows = session_rows(&ws, &Sidebar::new(), ActiveLabel::Branch);
        assert_eq!(
            keys(&rows),
            vec![
                SessionKey::Pane(panes[1]),
                SessionKey::Pane(panes[2]),
                SessionKey::Tab(tabs[2]),
                SessionKey::Tab(tabs[4]),
                SessionKey::Tab(tabs[0]),
                SessionKey::Tab(tabs[3]),
            ]
        );
        let kinds: Vec<SessionKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SessionKind::Agent,
                SessionKind::Agent,
                SessionKind::View(ViewKind::GitTree),
                SessionKind::View(ViewKind::DiffReview),
                SessionKind::Terminal,
                SessionKind::Terminal,
            ]
        );
        assert_eq!((rows[0].pane_index, rows[1].pane_index), (1, 2));
        assert!(rows[0].shares_tab && rows[1].shares_tab);
        assert_eq!(rows[1].agent.as_deref(), Some("codex"));
        assert!(
            rows.iter()
                .skip(2)
                .all(|r| r.pane_index == 0 && !r.shares_tab)
        );
    }

    #[test]
    fn status_changes_keep_identity_and_place_without_duplicates() {
        let (mut ws, tabs, panes) = fixture();
        let sidebar = Sidebar::new();
        let before = keys(&session_rows(&ws, &sidebar, ActiveLabel::Branch));
        for status in [
            AgentStatus::Idle,
            AgentStatus::WaitingInput,
            AgentStatus::Review,
            AgentStatus::Working,
        ] {
            agent(&mut ws, panes[1], "claude", status, true);
            agent(&mut ws, panes[2], "codex", status, true);
            let rows = session_rows(&ws, &sidebar, ActiveLabel::Branch);
            assert_eq!(keys(&rows), before, "{status:?}");
            assert_eq!(
                rows.iter()
                    .filter(|r| r.key == SessionKey::Pane(panes[2]))
                    .count(),
                1
            );
            assert_eq!(rows[1].status, Some(status));
            assert_eq!(rows[1].live_idle(), status == AgentStatus::Idle);
        }
        // An agent that has not reported yet still has its launched pane's row;
        // after it exits, the row stays with the gray dot.
        assert!(ws.activate_tab_id(tabs[1]));
        let launched = ws
            .split(Axis::Horizontal)
            .expect("a split in the agents' tab");
        ws.pane_mut(launched).unwrap().agent_launch = Some(chda_agents::AgentLaunchContext {
            agent: chda_agents::AgentId::Claude,
            executable: "claude".into(),
            cwd: "/src/app".into(),
            options: Vec::new(),
            policy: Default::default(),
            session: None,
            managed: true,
            reported_account_scope: None,
        });
        let rows = session_rows(&ws, &sidebar, ActiveLabel::Branch);
        let row = rows
            .iter()
            .find(|r| r.key == SessionKey::Pane(launched))
            .unwrap();
        assert_eq!((row.status, row.status_label()), (None, "No live agent"));
        assert!(!row.live_idle());
        // The last agent leaving a tab turns it into a terminal row at the end.
        for pane in [panes[1], panes[2]] {
            ws.end_agent(pane);
        }
        ws.pane_mut(launched).unwrap().agent_launch = None;
        let rows = session_rows(&ws, &sidebar, ActiveLabel::Branch);
        let last = rows.last().unwrap();
        assert_eq!(last.kind, SessionKind::Terminal);
        assert!(rows.iter().all(|r| r.kind != SessionKind::Agent));
    }

    #[test]
    fn worktree_only_reports_keep_a_terminal_row_with_their_status() {
        let mut ws = Workspace::new();
        let (tab, pane) = ws.new_tab();
        agent(&mut ws, pane, "claude", AgentStatus::Review, false);
        let rows = session_rows(&ws, &Sidebar::new(), ActiveLabel::Branch);
        assert_eq!(keys(&rows), vec![SessionKey::Tab(tab)]);
        assert_eq!(rows[0].kind, SessionKind::Terminal);
        assert_eq!(rows[0].status_label(), "Turn complete");
        agent(&mut ws, pane, "claude", AgentStatus::Idle, false);
        let rows = session_rows(&ws, &Sidebar::new(), ActiveLabel::Branch);
        assert_eq!((rows[0].status, rows[0].status_label()), (None, "Terminal"));
    }

    #[test]
    fn labels_follow_each_row_and_say_what_the_status_means() {
        let mut sidebar = Sidebar::new();
        sidebar.add_repo("/src/app".into());
        sidebar.set_worktrees(
            Path::new("/src/app"),
            vec![
                crate::WorktreeEntry {
                    path: "/src/app".into(),
                    branch: Some("main".into()),
                    is_main: true,
                    note: Some("Fix login\nmore".into()),
                    ..Default::default()
                },
                crate::WorktreeEntry {
                    path: "/src/app.wt/feat".into(),
                    branch: Some("feat".into()),
                    ..Default::default()
                },
            ],
        );
        let (mut ws, tabs, panes) = fixture();
        for (pane, cwd, branch) in [
            (panes[1], "/src/app", "main"),
            (panes[2], "/src/app.wt/feat", "feat"),
        ] {
            let info = ws.pane_mut(pane).unwrap();
            info.cwd = Some(cwd.into());
            info.branch = Some(branch.into());
            info.repo = Some("/src/app".into());
        }
        let rows = session_rows(&ws, &sidebar, ActiveLabel::Alias);
        assert_eq!(rows[0].title, "Fix login");
        assert_eq!(rows[1].title, "feat", "each agent row uses its own pane");
        assert_eq!(rows[1].location(), "app / feat");
        assert_eq!(rows[1].repo.as_deref(), Some("app"));
        let rows = session_rows(&ws, &sidebar, ActiveLabel::Branch);
        assert_eq!(rows[0].title, "main");
        let labels: Vec<&str> = rows.iter().map(SessionRow::status_label).collect();
        assert_eq!(
            labels,
            [
                "Working",
                "Idle",
                "Git tree",
                "Diff review",
                "Terminal",
                "Terminal"
            ]
        );
        agent(&mut ws, panes[1], "claude", AgentStatus::WaitingInput, true);
        let rows = session_rows(&ws, &sidebar, ActiveLabel::Branch);
        assert_eq!(rows[0].status_label(), "Waiting for input");
        assert_eq!(rows[2].title, "Git tree · app");
        assert_eq!(rows[2].repo.as_deref(), Some("app"));
        assert_eq!(rows[4].location(), "Directory unavailable");
        assert_eq!(
            focused_session(&rows, tabs[1], Some(panes[2])),
            Some(SessionKey::Pane(panes[2]))
        );
        assert_eq!(
            focused_session(&rows, tabs[1], Some(panes[3])),
            Some(SessionKey::Pane(panes[1])),
            "a shell split falls back to its tab's first row"
        );
        assert_eq!(
            focused_session(&rows, tabs[2], None),
            Some(SessionKey::Tab(tabs[2]))
        );
    }

    #[test]
    fn activity_updates_in_place() {
        let (mut ws, tabs, panes) = fixture();
        let mut rows = session_rows(&ws, &Sidebar::new(), ActiveLabel::Branch);
        let order = keys(&rows);
        assert!(!update_session_activity(&mut rows, &ws));
        ws.pane_mut(panes[2]).unwrap().last_activity = 50;
        ws.pane_mut(panes[0]).unwrap().last_activity = 40;
        ws.pane_mut(panes[3]).unwrap().last_activity = 60;
        assert!(update_session_activity(&mut rows, &ws));
        assert_eq!(keys(&rows), order);
        let at =
            |rows: &[SessionRow], key| rows.iter().find(|r| r.key == key).unwrap().last_activity;
        assert_eq!(at(&rows, SessionKey::Pane(panes[2])), 50);
        assert_eq!(
            at(&rows, SessionKey::Pane(panes[1])),
            0,
            "a shell split's output is not the agent's"
        );
        assert_eq!(at(&rows, SessionKey::Tab(tabs[0])), 40);
        assert_eq!(rows, {
            let mut fresh = session_rows(&ws, &Sidebar::new(), ActiveLabel::Branch);
            update_session_activity(&mut fresh, &ws);
            fresh
        });
    }
}
