//! Sidebar model: repositories, their worktrees, and the agents running in
//! them. Pure data; refreshers and the UI feed it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Agent activity in one worktree, as the status board shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentStatus {
    #[default]
    Idle,
    Working,
    /// The agent asked the user something (permission, idle prompt).
    WaitingInput,
    /// The agent finished and the output has not been looked at yet.
    Review,
}

/// Counts shown as git badges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GitBadges {
    pub changed: usize,
    pub staged: usize,
    pub untracked: usize,
    pub conflicted: usize,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
}

impl GitBadges {
    pub fn dirty_count(&self) -> usize {
        self.changed + self.staged + self.untracked + self.conflicted
    }
}

/// Pull request state for a worktree's branch, from `gh`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub checks: CheckState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    /// No checks reported.
    None,
    Pending,
    Success,
    Failure,
}

/// A past agent session that can be resumed in this worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    pub agent: String,
    pub id: String,
    pub started_at: u64,
    pub snippet: String,
    pub message_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
    pub badges: GitBadges,
    pub pr: Option<PrInfo>,
    /// The branch is fully contained in the default branch.
    pub merged: bool,
    /// Agent status per agent id.
    pub agents: BTreeMap<String, AgentStatus>,
    /// Newest first.
    pub sessions: Vec<SessionEntry>,
    /// Milliseconds since the epoch of the last agent event or session.
    pub last_activity: u64,
    /// Panes whose cwd is inside this worktree.
    pub panes: Vec<crate::PaneId>,
}

impl WorktreeEntry {
    /// Merged, clean and not the main worktree: safe to remove.
    pub fn safe_to_delete(&self) -> bool {
        !self.is_main
            && self.merged
            && self.badges.dirty_count() == 0
            && self.badges.ahead.unwrap_or(0) == 0
    }

    /// The most urgent agent status across agents.
    pub fn status(&self) -> AgentStatus {
        self.agents
            .values()
            .copied()
            .max_by_key(|s| match s {
                AgentStatus::Idle => 0,
                AgentStatus::Review => 1,
                AgentStatus::Working => 2,
                AgentStatus::WaitingInput => 3,
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoEntry {
    /// Main worktree path; doubles as the identifier.
    pub path: PathBuf,
    pub name: String,
    /// Main worktree first, then by last activity.
    pub worktrees: Vec<WorktreeEntry>,
    pub collapsed: bool,
    /// Last refresh failed with this message.
    pub error: Option<String>,
}

/// Sort order for worktrees inside a repository.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Activity,
    Name,
}

/// A tab as the sidebar's activity list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveTab {
    pub tab: crate::TabId,
    pub title: String,
    /// Repository name, or `None` outside any repository.
    pub repo: Option<String>,
    pub last_activity: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Sidebar {
    pub repos: Vec<RepoEntry>,
    pub sort: SortOrder,
    pub visible: bool,
    /// Open tabs, most recently active first.
    pub active_tabs: Vec<ActiveTab>,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            visible: true,
            ..Default::default()
        }
    }

    pub fn add_repo(&mut self, path: PathBuf) -> bool {
        if self.repos.iter().any(|r| r.path == path) {
            return false;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.repos.push(RepoEntry {
            path,
            name,
            ..Default::default()
        });
        true
    }

    pub fn remove_repo(&mut self, path: &Path) -> bool {
        let before = self.repos.len();
        self.repos.retain(|r| r.path != path);
        self.repos.len() != before
    }

    pub fn repo_mut(&mut self, path: &Path) -> Option<&mut RepoEntry> {
        self.repos.iter_mut().find(|r| r.path == path)
    }

    /// Replace a repository's worktree list, keeping agent and session state
    /// for worktrees that still exist.
    pub fn set_worktrees(&mut self, repo: &Path, worktrees: Vec<WorktreeEntry>) {
        let sort = self.sort;
        let Some(entry) = self.repo_mut(repo) else {
            return;
        };
        let old = std::mem::take(&mut entry.worktrees);
        entry.worktrees = worktrees
            .into_iter()
            .map(|mut w| {
                if let Some(prev) = old.iter().find(|o| o.path == w.path) {
                    w.agents = prev.agents.clone();
                    w.sessions = prev.sessions.clone();
                    w.last_activity = prev.last_activity;
                    w.panes = prev.panes.clone();
                    w.pr = prev.pr.clone();
                }
                w
            })
            .collect();
        entry.error = None;
        sort_worktrees(&mut entry.worktrees, sort);
    }

    /// The worktree whose path contains `cwd`, longest match wins.
    pub fn worktree_for_path(&self, cwd: &Path) -> Option<(&RepoEntry, &WorktreeEntry)> {
        self.repos
            .iter()
            .flat_map(|r| r.worktrees.iter().map(move |w| (r, w)))
            .filter(|(_, w)| cwd.starts_with(&w.path))
            .max_by_key(|(_, w)| w.path.as_os_str().len())
    }

    fn worktree_mut_for_path(&mut self, cwd: &Path) -> Option<&mut WorktreeEntry> {
        let best = self
            .repos
            .iter()
            .enumerate()
            .flat_map(|(ri, r)| {
                r.worktrees
                    .iter()
                    .enumerate()
                    .map(move |(wi, w)| (ri, wi, w))
            })
            .filter(|(_, _, w)| cwd.starts_with(&w.path))
            .max_by_key(|(_, _, w)| w.path.as_os_str().len())
            .map(|(ri, wi, _)| (ri, wi));
        let (ri, wi) = best?;
        Some(&mut self.repos[ri].worktrees[wi])
    }

    /// Apply an agent event for the worktree containing `cwd`. Returns the
    /// worktree path and new status when something changed.
    pub fn apply_agent_event(
        &mut self,
        agent: &str,
        cwd: &Path,
        event: AgentEvent,
        timestamp: u64,
    ) -> Option<(PathBuf, AgentStatus)> {
        let w = self.worktree_mut_for_path(cwd)?;
        let current = w.agents.get(agent).copied().unwrap_or_default();
        let next = match (current, event) {
            (_, AgentEvent::SessionStart) => AgentStatus::Working,
            (_, AgentEvent::PromptSubmitted) => AgentStatus::Working,
            (_, AgentEvent::WaitingInput) => AgentStatus::WaitingInput,
            (_, AgentEvent::Stopped) => AgentStatus::Review,
            (_, AgentEvent::SessionEnd) => AgentStatus::Idle,
        };
        w.agents.insert(agent.to_owned(), next);
        w.last_activity = w.last_activity.max(timestamp);
        let path = w.path.clone();
        self.resort();
        Some((path, next))
    }

    /// The user looked at a worktree's terminal: a finished turn is reviewed.
    pub fn mark_reviewed(&mut self, cwd: &Path) -> bool {
        let Some(w) = self.worktree_mut_for_path(cwd) else {
            return false;
        };
        let mut changed = false;
        for status in w.agents.values_mut() {
            if *status == AgentStatus::Review {
                *status = AgentStatus::Idle;
                changed = true;
            }
        }
        changed
    }

    /// Attach sessions to the worktrees that contain their cwd.
    pub fn set_sessions(&mut self, sessions: &[(PathBuf, SessionEntry)]) {
        for repo in &mut self.repos {
            for w in &mut repo.worktrees {
                w.sessions.clear();
            }
        }
        for (cwd, session) in sessions {
            if let Some(w) = self.worktree_mut_for_path(cwd) {
                w.last_activity = w.last_activity.max(session.started_at);
                w.sessions.push(session.clone());
            }
        }
        for repo in &mut self.repos {
            for w in &mut repo.worktrees {
                w.sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
            }
        }
        self.resort();
    }

    /// Record which panes live in which worktree.
    pub fn set_panes(&mut self, panes: &[(crate::PaneId, PathBuf)]) {
        for repo in &mut self.repos {
            for w in &mut repo.worktrees {
                w.panes.clear();
            }
        }
        for (pane, cwd) in panes {
            if let Some(w) = self.worktree_mut_for_path(cwd) {
                w.panes.push(*pane);
            }
        }
    }

    /// Store pull request info for a worktree's branch.
    pub fn set_pr(&mut self, worktree: &Path, pr: Option<PrInfo>) -> bool {
        match self.worktree_mut_for_path(worktree) {
            Some(w) if w.path == worktree => {
                let changed = w.pr != pr;
                w.pr = pr;
                changed
            }
            _ => false,
        }
    }

    pub fn set_sort(&mut self, sort: SortOrder) {
        self.sort = sort;
        self.resort();
    }

    fn resort(&mut self) {
        let sort = self.sort;
        for repo in &mut self.repos {
            sort_worktrees(&mut repo.worktrees, sort);
        }
    }

    /// Worktrees where an agent is waiting for the user.
    pub fn waiting(&self) -> Vec<&WorktreeEntry> {
        self.repos
            .iter()
            .flat_map(|r| r.worktrees.iter())
            .filter(|w| w.status() == AgentStatus::WaitingInput)
            .collect()
    }
}

fn sort_worktrees(worktrees: &mut [WorktreeEntry], sort: SortOrder) {
    worktrees.sort_by(|a, b| {
        b.is_main.cmp(&a.is_main).then_with(|| match sort {
            SortOrder::Activity => b
                .last_activity
                .cmp(&a.last_activity)
                .then_with(|| a.branch.cmp(&b.branch)),
            SortOrder::Name => a.branch.cmp(&b.branch),
        })
    });
}

/// Agent lifecycle events, as the hook receiver reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    SessionStart,
    PromptSubmitted,
    WaitingInput,
    Stopped,
    SessionEnd,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wt(path: &str, branch: &str, is_main: bool) -> WorktreeEntry {
        WorktreeEntry {
            path: PathBuf::from(path),
            branch: Some(branch.into()),
            is_main,
            ..Default::default()
        }
    }

    #[test]
    fn agent_events_drive_status_and_sorting() {
        let mut sb = Sidebar::new();
        assert!(sb.add_repo("/src/app".into()));
        assert!(!sb.add_repo("/src/app".into()));
        sb.set_worktrees(
            Path::new("/src/app"),
            vec![
                wt("/src/app.worktrees/b", "b", false),
                wt("/src/app", "main", true),
                wt("/src/app.worktrees/a", "a", false),
            ],
        );
        let branches = |sb: &Sidebar| -> Vec<String> {
            sb.repos[0]
                .worktrees
                .iter()
                .map(|w| w.branch.clone().unwrap())
                .collect()
        };
        assert_eq!(branches(&sb), vec!["main", "a", "b"]);

        let r = sb.apply_agent_event(
            "claude",
            Path::new("/src/app.worktrees/b/src"),
            AgentEvent::SessionStart,
            10,
        );
        assert_eq!(
            r,
            Some((PathBuf::from("/src/app.worktrees/b"), AgentStatus::Working))
        );
        assert_eq!(branches(&sb), vec!["main", "b", "a"]);
        sb.apply_agent_event(
            "claude",
            Path::new("/src/app.worktrees/b"),
            AgentEvent::WaitingInput,
            11,
        );
        assert_eq!(sb.waiting().len(), 1);
        sb.apply_agent_event(
            "claude",
            Path::new("/src/app.worktrees/b"),
            AgentEvent::Stopped,
            12,
        );
        let (_, w) = sb
            .worktree_for_path(Path::new("/src/app.worktrees/b"))
            .unwrap();
        assert_eq!(w.status(), AgentStatus::Review);
        assert!(sb.mark_reviewed(Path::new("/src/app.worktrees/b/x")));
        assert_eq!(
            sb.worktree_for_path(Path::new("/src/app.worktrees/b"))
                .unwrap()
                .1
                .status(),
            AgentStatus::Idle
        );
        assert_eq!(
            sb.apply_agent_event("claude", Path::new("/elsewhere"), AgentEvent::Stopped, 1),
            None
        );

        // Refresh keeps agent state and sessions for surviving worktrees.
        sb.set_sessions(&[(
            PathBuf::from("/src/app.worktrees/a"),
            SessionEntry {
                agent: "codex".into(),
                id: "s".into(),
                started_at: 50,
                snippet: "hi".into(),
                message_count: 1,
            },
        )]);
        assert_eq!(branches(&sb), vec!["main", "a", "b"]);
        sb.set_worktrees(
            Path::new("/src/app"),
            vec![
                wt("/src/app", "main", true),
                wt("/src/app.worktrees/a", "a", false),
            ],
        );
        assert_eq!(sb.repos[0].worktrees[1].sessions.len(), 1);
        sb.set_sort(SortOrder::Name);
        assert_eq!(branches(&sb), vec!["main", "a"]);
        assert!(sb.remove_repo(Path::new("/src/app")));
        assert!(sb.repos.is_empty());
    }
}
