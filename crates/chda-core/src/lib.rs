//! Domain hub: workspace, worktree and agent models plus the event bus.
//!
//! This crate knows nothing about GPUI, libghostty, or gix.

mod workspace;

pub use workspace::*;
