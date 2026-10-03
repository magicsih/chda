//! Headless scenario tests: a real `WorkspaceView` in GPUI's test platform,
//! driven by key bindings, typed input, actions and agent hook events, with
//! real shells and git repositories under a temporary home. See
//! docs/testing.md for the list and what each covers.

mod agents;
mod config;
mod diagrams;
mod diff;
mod folders;
mod harness;
mod input;
mod mcp;
mod menus;
mod palette;
mod paths;
mod pull_requests;
mod release;
mod restore;
mod search;
mod sidebar;
mod workspace;
mod worktrees;
