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

/// Worktrees of `repo` with fresh git badges and merged flags.
pub fn worktrees_of(repo: &Path) -> io::Result<Vec<WorktreeEntry>> {
    let merged: Vec<String> = chda_git::default_branch(repo)
        .and_then(|base| chda_git::branches_merged_into(repo, &base))
        .unwrap_or_default();
    let mut out = Vec::new();
    for info in chda_git::list_worktrees(repo)? {
        let badges = chda_git::status(&info.path).map(badges).unwrap_or_default();
        let merged = info.branch.as_ref().is_some_and(|b| merged.contains(b));
        out.push(WorktreeEntry {
            path: info.path,
            branch: info.branch,
            is_main: info.is_main,
            badges,
            merged,
            ..Default::default()
        });
    }
    Ok(out)
}

/// Local branches of `repo` that no worktree has checked out.
pub fn unchecked_branches(repo: &Path, worktrees: &[WorktreeEntry]) -> io::Result<Vec<String>> {
    let mut branches = chda_git::local_branches(repo)?;
    branches.retain(|b| !worktrees.iter().any(|w| w.branch.as_deref() == Some(b)));
    Ok(branches)
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

/// Delete a local branch; `force` deletes unmerged branches too.
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> io::Result<()> {
    chda_git::delete_branch(repo, branch, force)
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
