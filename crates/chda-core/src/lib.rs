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
mod managed_run;
mod names;
pub mod notifications;
mod refresh;
pub mod release;
pub mod resources;
mod restore;
pub use managed_run::ManagedRun;
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
        AgentAdapter, AgentId, AgentLaunchContext, AgentSession, CODEX_TITLE_CONFIG,
        CODEX_TITLE_ENV, ChildActivity, ChildBoard, ChildEvent, ChildState, CodexRunState,
        HookEvent, HookInstallReport, HookKind, LaunchHistory, LimitUsage, ModelUsage, PANE_ENV,
        PermissionPolicy, SessionCache, SessionId, Usage, adapters, codex_notify_config,
        command_argv, compact_tokens, ipc, parse_codex_title, permission_arguments,
        read_codex_children, validate_resume_options,
    };
    pub use chda_agents::{control, quota, statusline, which};
}
pub use chda_git::PullMode;
pub use sidebar::*;
pub use update::*;
pub use watch::{FileWatcher, RepoWatcher};
pub use workspace::*;

pub mod self_update;
