//! Domain hub: workspace, worktree and agent models plus the event bus.
//!
//! This crate knows nothing about GPUI, libghostty, or gix.

mod cleanup;
mod forge;
mod gitea;
mod github;
mod gitlab;
mod graph;
pub mod handoff;
pub use graph::*;
mod names;
mod refresh;
pub mod release;
pub mod resources;
mod restore;
mod sidebar;
mod update;
mod watch;
mod workspace;

pub use cleanup::*;
pub use forge::{Forge, ForgeClis, Forges, RemoteRepo, login_hint, repo_remote};
pub use names::random_branch_name;
pub use refresh::*;
pub use restore::*;

/// Agent integration surface the UI needs (ADR: the UI talks to core only).
pub mod agents {
    pub use chda_agents::hook::data_dir;
    pub use chda_agents::{
        AgentAdapter, AgentId, AgentSession, CODEX_TITLE_CONFIG, CODEX_TITLE_ENV, CodexRunState,
        HookEvent, HookInstallReport, HookKind, LimitUsage, ModelUsage, PANE_ENV, SessionCache,
        SessionId, Usage, adapters, codex_notify_config, command_argv, compact_tokens, ipc,
        parse_codex_title,
    };
    pub use chda_agents::{control, quota, statusline, which};
}
pub use chda_git::PullMode;
pub use sidebar::*;
pub use update::*;
pub use watch::{FileWatcher, RepoWatcher};
pub use workspace::*;
