//! Headless scenario tests: a real `WorkspaceView` in GPUI's test platform,
//! driven by key bindings, typed input, actions and agent hook events, with
//! real shells and git repositories under a temporary home. See
//! docs/testing.md for the list and what each covers.

mod agents;
mod config;
mod harness;
mod input;
mod palette;
mod restore;
mod search;
mod workspace;
mod worktrees;
