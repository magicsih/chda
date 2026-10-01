//! Blocking refresh helpers the UI runs on background threads.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chda_agents::{AgentAdapter, SessionCache};

use crate::sidebar::{GitBadges, SessionEntry, WorktreeEntry};

fn badges(status: chda_git::GitStatus) -> GitBadges {
    GitBadges {
        changed: status.changed,
        staged: status.staged,
        untracked: status.untracked,
        conflicted: status.conflicted,
        ahead: status.ahead,
        behind: status.behind,
    }
}

/// Worktrees of `repo` with fresh git badges.
pub fn worktrees_of(repo: &Path) -> io::Result<Vec<WorktreeEntry>> {
    let mut out = Vec::new();
    for info in chda_git::list_worktrees(repo)? {
        let badges = chda_git::status(&info.path).map(badges).unwrap_or_default();
        out.push(WorktreeEntry {
            path: info.path,
            branch: info.branch,
            is_main: info.is_main,
            badges,
            ..Default::default()
        });
    }
    Ok(out)
}

/// The main worktree of the repository containing `path`.
pub fn repo_of(path: &Path) -> io::Result<PathBuf> {
    chda_git::main_worktree(path)
}

/// Create a worktree for `branch` at `path` (new branch from HEAD unless it exists).
pub fn create_worktree(repo: &Path, branch: &str, path: &Path) -> io::Result<()> {
    chda_git::add_worktree(repo, branch, path, None)
}

/// Remove a worktree and prune.
pub fn delete_worktree(repo: &Path, path: &Path, force: bool) -> io::Result<()> {
    chda_git::remove_worktree(repo, path, force)
}

/// Fresh badges for one worktree.
pub fn badges_of(worktree: &Path) -> io::Result<GitBadges> {
    chda_git::status(worktree).map(badges)
}

/// Index every known agent session, newest first, keyed by its cwd.
pub fn index_sessions(
    adapters: &[Box<dyn AgentAdapter>],
    cache: &Arc<Mutex<SessionCache>>,
) -> Vec<(PathBuf, SessionEntry)> {
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    chda_agents::index_sessions(adapters, &mut cache)
        .into_iter()
        .map(|s| {
            (
                s.cwd,
                SessionEntry {
                    agent: s.agent.as_str().to_owned(),
                    id: s.id.0,
                    started_at: s.started_at,
                    snippet: s.snippet,
                    message_count: s.message_count,
                },
            )
        })
        .collect()
}
