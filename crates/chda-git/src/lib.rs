//! Git access for the sidebar (ADR-0003): gix for reads, the `git` binary
//! for writes (worktree add/remove, merge, branch delete).

mod cli;
mod read;

pub use cli::{
    MergeOutcome, add_worktree, branches_merged_into, current_branch, default_branch,
    delete_branch, fetch_default_branch, local_branches, merge_into, merge_target, patch_merged,
    purge_trash, remove_worktree, resolve, trash_worktree,
};
pub use read::{
    GitStatus, RemoteInfo, WorktreeInfo, list_worktrees, main_worktree, remotes, status,
};
