//! Domain hub: workspace, worktree and agent models plus the event bus.
//!
//! This crate knows nothing about GPUI, libghostty, or gix.

mod sidebar;
mod workspace;

pub use sidebar::*;
pub use workspace::*;
