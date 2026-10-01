//! Domain hub: workspace, worktree and agent models plus the event bus.
//!
//! This crate knows nothing about GPUI, libghostty, or gix.

mod refresh;
mod sidebar;
mod watch;
mod workspace;

pub use refresh::*;

/// Agent integration surface the UI needs (ADR: the UI talks to core only).
pub mod agents {
    pub use chda_agents::hook::data_dir;
    pub use chda_agents::{
        AgentAdapter, AgentId, HookEvent, HookKind, SessionCache, SessionId, adapters, ipc,
    };
}
pub use sidebar::*;
pub use watch::RepoWatcher;
pub use workspace::*;
