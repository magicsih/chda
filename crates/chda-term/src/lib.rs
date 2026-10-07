//! Wrapper over `libghostty-vt`. PTY bytes go in, terminal state comes out as
//! plain Rust types. No FFI types leak out of this crate.

mod frame;
mod graphics;
mod input;
mod links;
pub mod mermaid;
mod pwd;
mod search;
mod session;
mod shell_integration;
mod vt;

pub use chda_pty::ExitStatus;
/// Process transport for session handoff; the UI stays above the PTY layer.
pub use chda_pty::handoff;
pub use frame::*;
pub use graphics::{Image, ImageLayer, ImagePlacement};
pub use input::*;
pub use links::{Hyperlink, Link, LinkTarget, links_at};
pub use pwd::parse_pwd_report;
pub use search::{SearchMark, SearchQuery, SearchStatus};
pub use session::{DEFAULT_SCROLLBACK, DetachedSession, Event, Session, SessionOptions};
pub use shell_integration::{
    ShellIntegration, ShellLaunch, default_data_dir, launch_for, login_shell,
};
pub use vt::{Effects, Error, Result, Terminal};

/// Login-shell PATH capture lives with platform process handling.
pub use chda_pty::shell_path;
