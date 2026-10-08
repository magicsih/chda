//! Headless scenario tests: a real `WorkspaceView` in GPUI's test platform,
//! driven by key bindings, typed input, actions and agent hook events, with
//! real shells and git repositories under a temporary home. See
//! docs/testing.md for the list and what each covers.

mod agent_launch;
mod agent_restart;
mod agents;
mod children;
mod closing;
mod config;
mod diagrams;
mod diff;
mod folders;
mod git_graph;
mod harness;
mod idle_agents;
mod input;
mod mcp;
mod menus;
mod navigation;
mod notifications;
mod palette;
mod paths;
mod pull_requests;
mod quota;
mod redraw;
mod release;
mod reorder;
mod restore;
mod search;
mod sharing;
mod sidebar;
mod starred;
mod workspace;
mod worktrees;
