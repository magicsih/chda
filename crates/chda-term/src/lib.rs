//! Wrapper over `libghostty-vt`. PTY bytes go in, terminal state comes out as
//! plain Rust types. No FFI types leak out of this crate.

mod frame;
mod input;
mod session;
mod vt;

pub use chda_pty::ExitStatus;
pub use frame::*;
pub use input::*;
pub use session::{DEFAULT_SCROLLBACK, Event, Session, SessionOptions};
pub use vt::{Effects, Error, Result, Terminal};
