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

/// Worktrees safe to remove in bulk: merged with nothing uncommitted or
/// unpushed, or whose folder is already gone.
pub fn stale_worktrees(worktrees: &[WorktreeEntry]) -> Vec<&WorktreeEntry> {
    worktrees
        .iter()
        .filter(|w| w.safe_to_delete() || (w.missing && !w.is_main))
        .collect()
}

/// Remove a stale worktree without merging. Its branch is deleted when it
/// is merged; a missing worktree's unmerged branch is kept, so nothing is
/// lost.
pub fn remove_stale(repo: &Path, worktree: &WorktreeEntry) -> io::Result<()> {
    chda_git::remove_worktree(repo, &worktree.path, false)?;
    if worktree.missing && !worktree.is_merged() {
        return Ok(());
    }
    let branch = worktree
        .branch
        .clone()
        .ok_or_else(|| io::Error::other(Blocker::Detached.to_string()))?;
    chda_git::delete_branch(repo, &branch, false)
}

/// Delete a worktree and its branch whatever their state, after the user
/// confirmed. The folder is moved aside instead of deleted file by file, so
/// this returns in well under a second even for a large `node_modules`;
/// call [`purge_trash`] afterwards, in the background, to free the space.
pub fn delete_worktree_and_branch(repo: &Path, worktree: &WorktreeEntry) -> io::Result<()> {
    if worktree.is_main {
        return Err(io::Error::other(Blocker::MainWorktree.to_string()));
    }
    chda_git::trash_worktree(repo, &worktree.path)?;
    if let Some(branch) = &worktree.branch {
        chda_git::delete_branch(repo, branch, true).map_err(|e| {
            io::Error::other(format!(
                "removed the worktree, but deleting branch {branch} failed: {e}"
            ))
        })?;
    }
    Ok(())
}

/// Free the disk space of worktrees [`delete_worktree_and_branch`] removed.
pub fn purge_trash(repo: &Path) -> io::Result<()> {
    chda_git::purge_trash(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn missing_worktrees_are_listed_and_pruned_keeping_unmerged_branches() {
        let root = std::env::temp_dir().join(format!("chda-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        let wt = root.join("app.worktrees/feat");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
        );
        git(
            &wt,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "work",
            ],
        );
        std::fs::remove_dir_all(&wt).unwrap();

        let entries = crate::worktrees_of(&repo, false).unwrap();
        let feat = entries
            .iter()
            .find(|e| e.branch.as_deref() == Some("feat"))
            .unwrap();
        assert!(feat.missing);
        assert!(!feat.is_merged());
        assert_eq!(stale_worktrees(&entries).len(), 1);

        remove_stale(&repo, feat).unwrap();
        let entries = crate::worktrees_of(&repo, false).unwrap();
        assert_eq!(entries.len(), 1, "the record is pruned");
        git(&repo, &["rev-parse", "--verify", "-q", "refs/heads/feat"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
