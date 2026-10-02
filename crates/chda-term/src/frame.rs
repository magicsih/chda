//! Plain-data snapshot of a terminal for one frame. No libghostty types.

/// RGB color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Default colors handed to the terminal at creation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorConfig {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    /// Palette overrides by index.
    pub palette: Vec<(u8, Rgb)>,
    pub cursor_shape: Option<CursorShape>,
    /// Whether the cursor blinks unless the application says otherwise.
    pub cursor_blink: Option<bool>,
}

/// Terminal size in cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// Underline decoration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// How many columns a cell occupies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellWidth {
    #[default]
    Narrow,
    /// First half of a two-column glyph.
    Wide,
    /// Second half of a wide glyph. Draw nothing.
    SpacerTail,
    /// Last column of a soft-wrapped row where a wide glyph did not fit.
    SpacerHead,
}

/// Text attributes of a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

/// One cell of the viewport. Text lives in [`Frame::text`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub(crate) text_start: u32,
    pub(crate) text_len: u16,
    pub width: CellWidth,
    /// Foreground, with palette and inverse already resolved.
    pub fg: Rgb,
    /// Background, with inverse resolved. `None` means the frame background.
    pub bg: Option<Rgb>,
    /// Underline color. `None` means the foreground.
    pub underline_color: Option<Rgb>,
    pub style: CellStyle,
    pub selected: bool,
    /// Highlight from an active scrollback search.
    pub search: crate::search::SearchMark,
}

impl Cell {
    /// Whether the cell has glyphs to draw.
    pub fn has_text(&self) -> bool {
        self.text_len > 0
    }
}

/// Per-row metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// The row is soft-wrapped into the next one.
    pub wrapped: bool,
    pub semantic_prompt: SemanticPrompt,
}

/// OSC 133 prompt marking for a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SemanticPrompt {
    #[default]
    None,
    Prompt,
    Continuation,
}

/// Cursor shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    #[default]
    Block,
    Underline,
    BlockHollow,
}

/// Cursor state in viewport coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub shape: CursorShape,
    pub blinking: bool,
    /// The cursor sits on the tail half of a wide glyph.
    pub at_wide_tail: bool,
    /// Cursor color. `None` means invert or use the theme.
    pub color: Option<Rgb>,
}

/// Scrollback position of the viewport, counted in rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// Everything a renderer needs to draw one frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub size: Size,
    /// `size.rows * size.cols` cells, row-major.
    pub cells: Vec<Cell>,
    pub rows: Vec<Row>,
    /// Backing store for cell text.
    pub text: String,
    /// `None` when hidden or scrolled out of the viewport.
    pub cursor: Option<Cursor>,
    /// The cursor cell even while the application hides the cursor, as
    /// TUIs that draw their own do. IME composition is drawn here.
    pub ime_anchor: Option<(u16, u16)>,
    pub foreground: Rgb,
    pub background: Rgb,
    pub scrollbar: Scrollbar,
    pub title: String,
    /// Raw OSC 7 value, e.g. `file://host/path`.
    pub pwd: String,
    /// The alternate screen is active (full-screen apps).
    pub alternate_screen: bool,
    /// OSC 8 hyperlinks in the viewport.
    pub hyperlinks: Vec<crate::links::Hyperlink>,
    /// Kitty graphics placements that touch the viewport, sorted by `z`.
    pub images: Vec<crate::graphics::ImagePlacement>,
    /// Mermaid diagrams whose whole source is in the viewport.
    pub diagrams: Vec<crate::mermaid::Diagram>,
    /// Monotonic counter, bumped on every rebuild.
    pub generation: u64,
}

impl Frame {
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.cells
            .get(usize::from(y) * usize::from(self.size.cols) + usize::from(x))
    }

    pub fn row_cells(&self, y: u16) -> &[Cell] {
        let cols = usize::from(self.size.cols);
        let start = usize::from(y) * cols;
        &self.cells[start..start + cols]
    }

    pub fn cell_text(&self, cell: &Cell) -> &str {
        let start = cell.text_start as usize;
        &self.text[start..start + usize::from(cell.text_len)]
    }

    /// Plain text of a row, with trailing blanks trimmed. Mostly for tests.
    pub fn row_text(&self, y: u16) -> String {
        let mut s = String::new();
        for cell in self.row_cells(y) {
            match cell.width {
                CellWidth::SpacerTail | CellWidth::SpacerHead => continue,
                _ => {}
            }
            if cell.has_text() {
                s.push_str(self.cell_text(cell));
            } else {
                s.push(' ');
            }
        }
        s.truncate(s.trim_end().len());
        s
    }

    pub(crate) fn push_cell_text(&mut self, text: &str) -> (u32, u16) {
        let start = self.text.len() as u32;
        self.text.push_str(text);
        (start, text.len() as u16)
    }

    pub(crate) fn set_cell_text(cell: &mut Cell, start: u32, len: u16) {
        cell.text_start = start;
        cell.text_len = len;
    }
}
