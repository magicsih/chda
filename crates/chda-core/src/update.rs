//! "Update branch": bring a worktree's upstream in when that is safe, and
//! never rewrite commits that were already pushed.

use std::io;
use std::path::Path;

use chda_config::PullStrategy;
use chda_git::{MergeOutcome, PullMode};

use crate::sidebar::{AgentStatus, WorktreeEntry};

/// Why "Update branch" is not offered right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateBlocker {
    Missing,
    Detached,
    NoUpstream,
    /// Uncommitted changes or untracked files.
    Dirty(usize),
    /// A merge, rebase, ... stopped half way.
    InProgress(&'static str),
    /// An agent (by id) is working or waiting for input in the worktree.
    AgentBusy {
        agent: String,
        waiting: bool,
    },
}

impl UpdateBlocker {
    /// A sentence for the menu; `agent_name` turns an agent id into its
    /// display name.
    pub fn describe(&self, agent_name: impl Fn(&str) -> String) -> String {
        match self {
            UpdateBlocker::Missing => "the folder is missing".into(),
            UpdateBlocker::Detached => "detached HEAD".into(),
            UpdateBlocker::NoUpstream => "the branch has no upstream".into(),
            UpdateBlocker::Dirty(n) => format!("{n} uncommitted change(s)"),
            UpdateBlocker::InProgress(op) => format!("a {op} is in progress"),
            UpdateBlocker::AgentBusy { agent, waiting } => format!(
                "{} is {} here",
                agent_name(agent),
                if *waiting {
                    "waiting for input"
                } else {
                    "working"
                }
            ),
        }
    }
}

/// The first reason updating `worktree` is unsafe, judged from the
/// sidebar's last refresh; `update_branch` checks git again before acting.
pub fn update_blocker(worktree: &WorktreeEntry) -> Option<UpdateBlocker> {
    if worktree.missing {
        return Some(UpdateBlocker::Missing);
    }
    if worktree.branch.is_none() {
        return Some(UpdateBlocker::Detached);
    }
    if let Some((agent, status)) = worktree
        .agents
        .iter()
        .find(|(_, s)| matches!(s, AgentStatus::Working | AgentStatus::WaitingInput))
    {
        return Some(UpdateBlocker::AgentBusy {
            agent: agent.clone(),
            waiting: *status == AgentStatus::WaitingInput,
        });
    }
    if let Some(op) = worktree.badges.operation {
        return Some(UpdateBlocker::InProgress(op));
    }
    let dirty = worktree.badges.dirty_count();
    if dirty > 0 {
        return Some(UpdateBlocker::Dirty(dirty));
    }
    if worktree.badges.behind.is_none() {
        return Some(UpdateBlocker::NoUpstream);
    }
    None
}

/// What "Update branch" did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    UpToDate,
    /// The upstream's `commits` are in, by `mode`.
    Updated {
        mode: PullMode,
        commits: usize,
    },
    /// Both sides have commits and the strategy does not allow resolving
    /// that on its own. `pushed`: some local commits are on a remote, so
    /// only a merge is offered.
    Diverged {
        ahead: usize,
        behind: usize,
        pushed: bool,
    },
    /// The rebase or merge stopped on conflicts and was aborted; nothing
    /// changed.
    Conflicted(PullMode),
}

/// Fetch `worktree`'s upstream and bring it in: fast-forward when the
/// branch is only behind; when it has diverged, rebase or merge as
/// `strategy` says. Commits already pushed are never rebased. Refuses when
/// the worktree has uncommitted changes or a stopped merge or rebase.
/// Blocking: run it off the UI thread.
pub fn update_branch(worktree: &Path, strategy: PullStrategy) -> io::Result<UpdateOutcome> {
    let status = chda_git::status(worktree)?;
    if let Some(op) = status.operation {
        return Err(io::Error::other(
            UpdateBlocker::InProgress(op).describe(str::to_owned),
        ));
    }
    if !status.is_clean() {
        let dirty = status.changed + status.staged + status.untracked + status.conflicted;
        return Err(io::Error::other(
            UpdateBlocker::Dirty(dirty).describe(str::to_owned),
        ));
    }
    let Some(st) = chda_git::fetch_upstream(worktree)? else {
        return Err(io::Error::other(
            UpdateBlocker::NoUpstream.describe(str::to_owned),
        ));
    };
    if st.behind == 0 {
        return Ok(UpdateOutcome::UpToDate);
    }
    let pushed = st.local_only < st.ahead;
    let mode = match (st.ahead, strategy) {
        (0, _) => PullMode::FastForward,
        (_, PullStrategy::Rebase) if !pushed => PullMode::Rebase,
        (_, PullStrategy::Merge) => PullMode::Merge,
        _ => {
            return Ok(UpdateOutcome::Diverged {
                ahead: st.ahead,
                behind: st.behind,
                pushed,
            });
        }
    };
    Ok(match chda_git::pull_upstream(worktree, mode)? {
        MergeOutcome::Conflicted => UpdateOutcome::Conflicted(mode),
        _ => UpdateOutcome::Updated {
            mode,
            commits: st.behind,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
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

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).unwrap();
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", file]);
    }

    /// `origin` with one commit and a clone tracking it.
    fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("chda-update-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let origin = root.join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "-b", "main"]);
        commit(&origin, "a");
        git(&root, &["clone", "-q", "origin", "clone"]);
        let clone = root.join("clone");
        (root, origin, clone)
    }

    #[test]
    fn behind_fast_forwards_and_diverged_follows_the_strategy() {
        let (root, origin, clone) = setup("ff");
        assert_eq!(
            update_branch(&clone, PullStrategy::FfOnly).unwrap(),
            UpdateOutcome::UpToDate
        );
        commit(&origin, "b");
        assert_eq!(
            update_branch(&clone, PullStrategy::FfOnly).unwrap(),
            UpdateOutcome::Updated {
                mode: PullMode::FastForward,
                commits: 1
            }
        );

        // Diverged, nothing pushed: ff-only stops, rebase replays.
        commit(&origin, "c");
        commit(&clone, "local");
        assert_eq!(
            update_branch(&clone, PullStrategy::FfOnly).unwrap(),
            UpdateOutcome::Diverged {
                ahead: 1,
                behind: 1,
                pushed: false
            }
        );
        assert_eq!(
            update_branch(&clone, PullStrategy::Rebase).unwrap(),
            UpdateOutcome::Updated {
                mode: PullMode::Rebase,
                commits: 1
            }
        );

        // Pushed local commits are never rebased; merge brings them together.
        git(&clone, &["push", "-q", "origin", "HEAD:refs/heads/wip"]);
        git(&clone, &["fetch", "-q", "origin"]);
        commit(&origin, "d");
        let diverged = UpdateOutcome::Diverged {
            ahead: 1,
            behind: 1,
            pushed: true,
        };
        assert_eq!(
            update_branch(&clone, PullStrategy::FfOnly).unwrap(),
            diverged
        );
        assert_eq!(
            update_branch(&clone, PullStrategy::Rebase).unwrap(),
            diverged
        );
        assert_eq!(
            update_branch(&clone, PullStrategy::Merge).unwrap(),
            UpdateOutcome::Updated {
                mode: PullMode::Merge,
                commits: 1
            }
        );
        assert!(clone.join("d").is_file());

        // Uncommitted changes refuse.
        std::fs::write(clone.join("dirty"), "x").unwrap();
        commit(&origin, "e");
        let err = update_branch(&clone, PullStrategy::FfOnly).unwrap_err();
        assert!(err.to_string().contains("uncommitted"), "{err}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn blockers_come_from_the_sidebar_state() {
        let mut w = WorktreeEntry {
            branch: Some("feat".into()),
            ..Default::default()
        };
        assert_eq!(update_blocker(&w), Some(UpdateBlocker::NoUpstream));
        w.badges.ahead = Some(0);
        w.badges.behind = Some(3);
        assert_eq!(update_blocker(&w), None);
        w.badges.untracked = 2;
        assert_eq!(update_blocker(&w), Some(UpdateBlocker::Dirty(2)));
        w.badges.operation = Some("rebase");
        assert_eq!(
            update_blocker(&w),
            Some(UpdateBlocker::InProgress("rebase"))
        );
        w.agents.insert("claude".into(), AgentStatus::Working);
        let blocker = update_blocker(&w).unwrap();
        assert_eq!(
            blocker.describe(|_| "Claude Code".into()),
            "Claude Code is working here"
        );
        w.agents.insert("claude".into(), AgentStatus::Review);
        assert_eq!(
            update_blocker(&w),
            Some(UpdateBlocker::InProgress("rebase"))
        );
    }
}
