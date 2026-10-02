//! Git access for the sidebar (ADR-0003): gix for reads, the `git` binary
//! for writes (worktree add/remove, merge, branch delete).

mod cli;
mod read;

pub use cli::{
    DiffStat, MergeOutcome, PullMode, UpstreamState, add_worktree, branch_descriptions,
    branches_merged_into, current_branch, default_branch, delete_branch, diff_stat,
    fetch_default_branch, fetch_upstream, local_branches, merge_into, merge_target, patch_merged,
    pull_upstream, purge_trash, remove_worktree, resolve, set_branch_description, trash_worktree,
};
pub use read::{
    GitStatus, RemoteInfo, WorktreeInfo, list_worktrees, main_worktree, remotes, status,
};
