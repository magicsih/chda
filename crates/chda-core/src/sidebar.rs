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
    /// The agent asked the user something (permission, approval).
    WaitingInput,
    /// The agent finished and the output has not been looked at yet.
    Review,
}

impl AgentStatus {
    /// How much a status needs the user, for picking the most urgent one.
    pub fn urgency(self) -> u8 {
        match self {
            AgentStatus::Idle => 0,
            AgentStatus::Review => 1,
            AgentStatus::Working => 2,
            AgentStatus::WaitingInput => 3,
        }
    }
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
    /// A merge, rebase, ... stopped half way.
    pub operation: Option<&'static str>,
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

/// How much a worktree changed against the branch it started from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffSummary {
    /// The base branch, e.g. `origin/main`.
    pub base: String,
    /// The commit the worktree's branch started from on `base`.
    pub merge_base: String,
    pub files: usize,
    pub added: usize,
    pub removed: usize,
}

/// A past agent session that can be resumed in this worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    pub agent: String,
    pub id: String,
    pub started_at: u64,
    /// When the session's last message was written.
    pub last_active_at: u64,
    pub snippet: String,
    pub message_count: usize,
    /// Tokens used, per model; empty when the transcript has none.
    pub usage: chda_agents::Usage,
}

/// How long ago `then` was, for compact lists: `now`, `3m`, `2h`,
/// `yesterday`, `5d`. Both in milliseconds since the epoch.
pub fn relative_age(now: u64, then: u64) -> String {
    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let ago = now.saturating_sub(then);
    match ago {
        _ if ago < MINUTE => "now".into(),
        _ if ago < HOUR => format!("{}m", ago / MINUTE),
        _ if ago < DAY => format!("{}h", ago / HOUR),
        _ if ago < 2 * DAY => "yesterday".into(),
        _ => format!("{}d", ago / DAY),
    }
}

/// Time since terminal activity, rather than agent progress. Zero is unknown.
pub fn activity_age(now: u64, then: u64) -> String {
    if then == 0 {
        return "\u{2014}".into();
    }
    let seconds = now.saturating_sub(then) / 1000;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
    pub badges: GitBadges,
    /// Changes against the default branch; `None` for the main worktree.
    pub diff: Option<DiffSummary>,
    pub pr: Option<PrInfo>,
    /// The branch's commits are in the default branch, by ancestry or by
    /// patch content (squash or rebase merge).
    pub merged: bool,
    /// Agent status per agent id.
    pub agents: BTreeMap<String, AgentStatus>,
    /// Most recently active first.
    pub sessions: Vec<SessionEntry>,
    /// Milliseconds since the epoch of the last agent event or session.
    pub last_activity: u64,
    pub previous_activity: Option<u64>,
    /// Panes whose cwd is inside this worktree.
    pub panes: Vec<crate::PaneId>,
    /// The folder was deleted outside git; only pruning makes sense.
    pub missing: bool,
    /// An operation is running on it, e.g. "deleting"; others are refused.
    pub busy: Option<String>,
    /// The last operation on it failed with this message.
    pub error: Option<String>,
    /// What the branch is for (`branch.<name>.description`); first line is
    /// the title.
    pub note: Option<String>,
}

impl WorktreeEntry {
    /// Merged by git, or the branch's pull request was merged (covers squash
    /// merges on a machine whose default branch is stale).
    pub fn is_merged(&self) -> bool {
        self.merged
            || self
                .pr
                .as_ref()
                .is_some_and(|pr| pr.state == PrState::Merged)
    }

    /// Merged, clean and not the main worktree: safe to remove.
    pub fn safe_to_delete(&self) -> bool {
        !self.is_main
            && self.is_merged()
            && self.badges.dirty_count() == 0
            && self.badges.ahead.unwrap_or(0) == 0
    }

    /// The note's first non-empty line.
    pub fn note_title(&self) -> Option<&str> {
        self.note
            .as_deref()?
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
    }

    /// The most urgent agent status across agents.
    pub fn status(&self) -> AgentStatus {
        self.agents
            .values()
            .copied()
            .max_by_key(|s| s.urgency())
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
    /// Why this repository has no pull request badges, e.g. the forge host
    /// is not logged in.
    pub pr_hint: Option<String>,
    /// A plain folder, not a git repository: one row for the folder itself
    /// and no git actions.
    pub folder: bool,
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
    pub agent_live: bool,
    /// Actual branch name, available when the label is an alias.
    pub branch: Option<String>,
    pub tab: crate::TabId,
    /// Most urgent agent status among the tab's panes.
    pub status: AgentStatus,
    pub title: String,
    /// Repository name, or `None` outside any repository.
    pub repo: Option<String>,
    pub last_activity: u64,
    pub previous_activity: Option<u64>,
}

/// A pane-bound live idle session, separate from historical session entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleAgent {
    pub pane: crate::PaneId,
    pub agent: String,
    pub tab: String,
    pub pane_index: usize,
    /// Repository and worktree context, including directories outside repos.
    pub location: String,
    pub cwd: Option<PathBuf>,
    /// Known completion timestamp; zero means the idle start is unavailable.
    pub since: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Sidebar {
    pub repos: Vec<RepoEntry>,
    pub sort: SortOrder,
    pub visible: bool,
    /// Open tabs in workspace order.
    pub active_tabs: Vec<ActiveTab>,
    /// Live idle panes in workspace tab/pane order.
    pub idle_agents: Vec<IdleAgent>,
    /// The STARRED list, as saved in the config.
    pub starred: Vec<chda_config::StarredBranch>,
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
                    w.busy = prev.busy.clone();
                    w.error = prev.error.clone();
                }
                w
            })
            .collect();
        entry.error = None;
        entry.folder = false;
        sort_worktrees(&mut entry.worktrees, sort);
    }

    /// Show `path` as a plain folder: a single row for the folder itself,
    /// keeping its agent and session state.
    pub fn set_folder(&mut self, path: &Path) {
        let row = WorktreeEntry {
            path: path.to_path_buf(),
            is_main: true,
            ..Default::default()
        };
        self.set_worktrees(path, vec![row]);
        if let Some(entry) = self.repo_mut(path) {
            entry.folder = true;
        }
    }

    /// Whether `path` was added as a plain folder.
    pub fn is_folder(&self, path: &Path) -> bool {
        self.repos.iter().any(|r| r.path == path && r.folder)
    }

    /// The worktree at exactly `path`.
    pub fn worktree_mut(&mut self, path: &Path) -> Option<&mut WorktreeEntry> {
        self.repos
            .iter_mut()
            .flat_map(|r| r.worktrees.iter_mut())
            .find(|w| w.path == path)
    }

    /// Drop a worktree from its repository's list until the next refresh.
    pub fn remove_worktree(&mut self, path: &Path) {
        for repo in &mut self.repos {
            repo.worktrees.retain(|w| w.path != path);
        }
    }

    /// The worktree that has `branch` checked out in the repository at
    /// `repo`; `None` when the repository or the worktree is gone.
    pub fn branch_worktree(
        &self,
        repo: &Path,
        branch: &str,
    ) -> Option<(&RepoEntry, &WorktreeEntry)> {
        let r = self.repos.iter().find(|r| r.path == repo && !r.folder)?;
        let w = r
            .worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some(branch))?;
        Some((r, w))
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
            (_, AgentEvent::SessionStart) => AgentStatus::Idle,
            (AgentStatus::Review, AgentEvent::Idle) => AgentStatus::Review,
            (_, AgentEvent::Idle) => AgentStatus::Idle,
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
                w.last_activity = w.last_activity.max(session.last_active_at);
                w.sessions.push(session.clone());
            }
        }
        for repo in &mut self.repos {
            for w in &mut repo.worktrees {
                w.sessions
                    .sort_by_key(|s| std::cmp::Reverse(s.last_active_at));
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
    Idle,
    SessionStart,
    PromptSubmitted,
    WaitingInput,
    Stopped,
    SessionEnd,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_activity_ages_cover_boundaries_and_unknown_times() {
        let then = 1000;
        for (delta, expected) in [
            (0, "0s"),
            (999, "0s"),
            (1000, "1s"),
            (59999, "59s"),
            (60000, "1m"),
            (3599999, "59m"),
            (3600000, "1h"),
            (86399999, "23h"),
            (86400000, "1d"),
            (172800000, "2d"),
        ] {
            assert_eq!(activity_age(then + delta, then), expected);
        }
        assert_eq!(activity_age(1000, 0), "\u{2014}");
        assert_eq!(activity_age(1000, 2000), "0s");
    }

    #[test]
    fn merged_pull_request_counts_as_merged() {
        let mut w = WorktreeEntry {
            branch: Some("feat".into()),
            ..Default::default()
        };
        assert!(!w.is_merged());
        w.pr = Some(PrInfo {
            number: 7,
            url: String::new(),
            state: PrState::Merged,
            checks: CheckState::None,
        });
        assert!(w.is_merged());
        assert!(w.safe_to_delete());
        w.badges.ahead = Some(1);
        assert!(!w.safe_to_delete());
    }

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
            Some((PathBuf::from("/src/app.worktrees/b"), AgentStatus::Idle))
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
                last_active_at: 60,
                snippet: "hi".into(),
                message_count: 1,
                usage: Default::default(),
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

    #[test]
    fn sessions_sort_by_last_message_and_show_their_age() {
        let mut sb = Sidebar::new();
        sb.add_repo(PathBuf::from("/src/app"));
        sb.set_worktrees(Path::new("/src/app"), vec![wt("/src/app", "main", true)]);
        let session = |id: &str, started_at, last_active_at| {
            (
                PathBuf::from("/src/app"),
                SessionEntry {
                    agent: "claude".into(),
                    id: id.into(),
                    started_at,
                    last_active_at,
                    snippet: String::new(),
                    message_count: 1,
                    usage: Default::default(),
                },
            )
        };
        // Started first but talked to last: listed first.
        sb.set_sessions(&[session("new", 20, 30), session("old", 10, 40)]);
        let ids: Vec<&str> = sb.repos[0].worktrees[0]
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, vec!["old", "new"]);
        assert_eq!(sb.repos[0].worktrees[0].last_activity, 40);

        let now = 10 * 24 * 3_600_000;
        let ages: Vec<String> = [
            0,
            30_000,
            3 * 60_000,
            2 * 3_600_000,
            30 * 3_600_000,
            5 * 86_400_000,
        ]
        .iter()
        .map(|ago| relative_age(now, now - ago))
        .collect();
        assert_eq!(ages, vec!["now", "now", "3m", "2h", "yesterday", "5d"]);
    }

    #[test]
    fn starred_branches_resolve_within_their_own_repository() {
        let mut s = Sidebar::new();
        s.add_repo("/src/a".into());
        s.add_repo("/src/b".into());
        s.set_worktrees(
            Path::new("/src/a"),
            vec![
                wt("/src/a", "main", true),
                wt("/src/a.wt/x", "feat/x", false),
            ],
        );
        s.set_worktrees(Path::new("/src/b"), vec![wt("/src/b", "main", true)]);
        let path = |repo: &str, branch: &str| {
            s.branch_worktree(Path::new(repo), branch)
                .map(|(_, w)| w.path.clone())
        };
        assert_eq!(path("/src/a", "main"), Some(PathBuf::from("/src/a")));
        assert_eq!(path("/src/b", "main"), Some(PathBuf::from("/src/b")));
        assert_eq!(path("/src/a", "feat/x"), Some(PathBuf::from("/src/a.wt/x")));
        assert_eq!(path("/src/b", "feat/x"), None, "never another repository's");
        assert_eq!(path("/src/gone", "main"), None);
    }
}
