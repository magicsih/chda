//! Finishing a worktree: merge its branch into the default branch, then
//! remove the worktree and the branch (scenario S4).

use std::io;
use std::path::{Path, PathBuf};

use chda_git::MergeOutcome;

use crate::sidebar::{PrState, WorktreeEntry};

/// Why a worktree cannot be cleaned up right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// Uncommitted changes or untracked files.
    Dirty(usize),
    /// Local commits not on the upstream.
    Unpushed(usize),
    /// Its pull request is still open.
    PrOpen(u64),
    /// The repository's main worktree has another branch checked out.
    BaseNotCheckedOut(String),
    MainWorktree,
    Detached,
}

impl std::fmt::Display for Blocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Blocker::Dirty(n) => write!(f, "{n} uncommitted change(s)"),
            Blocker::Unpushed(n) => write!(f, "{n} commit(s) not pushed"),
            Blocker::PrOpen(n) => write!(f, "pull request #{n} is still open"),
            Blocker::BaseNotCheckedOut(b) => {
                write!(f, "main worktree does not have {b} checked out")
            }
            Blocker::MainWorktree => write!(f, "the main worktree cannot be removed"),
            Blocker::Detached => write!(f, "detached HEAD"),
        }
    }
}

/// Checks that make a cleanup safe without `force`.
pub fn blockers(worktree: &WorktreeEntry) -> Vec<Blocker> {
    let mut out = Vec::new();
    if worktree.is_main {
        out.push(Blocker::MainWorktree);
    }
    if worktree.branch.is_none() {
        out.push(Blocker::Detached);
    }
    let dirty = worktree.badges.dirty_count();
    if dirty > 0 {
        out.push(Blocker::Dirty(dirty));
    }
    if let Some(ahead) = worktree.badges.ahead.filter(|a| *a > 0) {
        out.push(Blocker::Unpushed(ahead));
    }
    if let Some(pr) = &worktree.pr
        && pr.state == PrState::Open
    {
        out.push(Blocker::PrOpen(pr.number));
    }
    out
}

/// What a cleanup did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupReport {
    pub branch: String,
    pub merged: MergeOutcome,
    pub removed: PathBuf,
}

/// Merge `branch` into the default branch in the repository's main
/// worktree, then remove `worktree` and delete the branch. Blocking; run it
/// off the UI thread. Returns the first blocker as an error unless `force`.
pub fn merge_and_clean(
    repo: &Path,
    worktree: &WorktreeEntry,
    force: bool,
) -> io::Result<CleanupReport> {
    if !force && let Some(b) = blockers(worktree).into_iter().next() {
        return Err(io::Error::other(b.to_string()));
    }
    let branch = worktree
        .branch
        .clone()
        .ok_or_else(|| io::Error::other(Blocker::Detached.to_string()))?;
    let base = chda_git::default_branch(repo)?;
    let checked_out = chda_git::current_branch(repo)?;
    if checked_out.as_deref() != Some(base.as_str()) {
        return Err(io::Error::other(
            Blocker::BaseNotCheckedOut(base).to_string(),
        ));
    }
    let merged = chda_git::merge_into(repo, &branch, &base)?;
    if merged == MergeOutcome::Conflicted {
        return Err(io::Error::other(format!(
            "merging {branch} into {base} conflicts"
        )));
    }
    chda_git::remove_worktree(repo, &worktree.path, force)?;
    chda_git::delete_branch(repo, &branch, force)?;
    Ok(CleanupReport {
        branch,
        merged,
        removed: worktree.path.clone(),
    })
}

/// Worktrees whose branch is already merged into the default branch and
/// that have nothing uncommitted or unpushed: safe to remove in bulk.
pub fn stale_worktrees<'a>(
    repo: &Path,
    worktrees: &'a [WorktreeEntry],
) -> io::Result<Vec<&'a WorktreeEntry>> {
    let base = chda_git::default_branch(repo)?;
    let merged = chda_git::branches_merged_into(repo, &base)?;
    Ok(worktrees
        .iter()
        .filter(|w| !w.is_main)
        .filter(|w| w.branch.as_ref().is_some_and(|b| merged.contains(b)))
        .filter(|w| blockers(w).is_empty())
        .collect())
}

/// Remove a stale worktree and its branch without merging.
pub fn remove_stale(repo: &Path, worktree: &WorktreeEntry) -> io::Result<()> {
    let branch = worktree
        .branch
        .clone()
        .ok_or_else(|| io::Error::other(Blocker::Detached.to_string()))?;
    chda_git::remove_worktree(repo, &worktree.path, false)?;
    chda_git::delete_branch(repo, &branch, false)
}
