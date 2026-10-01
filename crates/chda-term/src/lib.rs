//! Wrapper over `libghostty-vt`. PTY bytes go in, terminal state comes out as
//! plain Rust types. No FFI types leak out of this crate.

use libghostty_vt::render::RenderState;
use libghostty_vt::terminal::{Options, Terminal as VtTerminal};

/// Error raised by the terminal core.
#[derive(Debug)]
pub struct Error(libghostty_vt::Error);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<libghostty_vt::Error> for Error {
    fn from(value: libghostty_vt::Error) -> Self {
        Self(value)
    }
}

/// Terminal size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// Snapshot of what a renderer needs for one frame.
///
/// This is the plain-struct boundary between the terminal core and the UI.
/// It will grow into a full cell grid in M1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub size: Size,
    /// Cursor position in viewport cells, if the cursor is in the viewport.
    pub cursor: Option<(u16, u16)>,
}

/// A terminal: VT parser plus screen state, driven from a single thread.
pub struct Terminal {
    vt: VtTerminal<'static, 'static>,
    render: RenderState<'static>,
}

impl Terminal {
    /// Create a terminal with the given size and scrollback limit in lines.
    pub fn new(size: Size, max_scrollback: usize) -> Result<Self, Error> {
        let vt = VtTerminal::new(Options {
            cols: size.cols,
            rows: size.rows,
            max_scrollback,
        })?;
        let render = RenderState::new()?;
        Ok(Self { vt, render })
    }

    /// Feed raw bytes from the PTY into the VT parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.vt.vt_write(bytes);
    }

    /// Refresh the render state from the terminal and return a frame snapshot.
    pub fn snapshot(&mut self) -> Result<Frame, Error> {
        let snap = self.render.update(&self.vt)?;
        let size = Size {
            cols: snap.cols()?,
            rows: snap.rows()?,
        };
        let cursor = snap.cursor_viewport()?.map(|c| (c.x, c.y));
        Ok(Frame { size, cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_moves_cursor_and_snapshot_reports_it() {
        let size = Size { cols: 20, rows: 5 };
        let mut term = Terminal::new(size, 100).unwrap();

        let frame = term.snapshot().unwrap();
        assert_eq!(frame.size, size);
        assert_eq!(frame.cursor, Some((0, 0)));

        term.feed(b"hello\r\n");
        let frame = term.snapshot().unwrap();
        assert_eq!(frame.cursor, Some((0, 1)));

        term.feed(b"\x1b[3;7H");
        let frame = term.snapshot().unwrap();
        assert_eq!(frame.cursor, Some((6, 2)));
    }
}
