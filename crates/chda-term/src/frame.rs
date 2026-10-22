//! Plain-data snapshot of a terminal for one frame. No libghostty types.

/// RGB color.
#[derive(
    serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash,
)]
#[serde(into = "[u8;3]", from = "[u8;3]")]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
impl From<Rgb> for [u8; 3] {
    fn from(c: Rgb) -> Self {
        [c.r, c.g, c.b]
    }
}
impl From<[u8; 3]> for Rgb {
    fn from(c: [u8; 3]) -> Self {
        Self {
            r: c[0],
            g: c[1],
            b: c[2],
        }
    }
}

/// Default colors handed to the terminal at creation.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// Underline decoration.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(into = "WireCell", from = "WireCell")]
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

// Repeated field names and boolean objects dominated frame transfer time.
// Keep the in-memory model readable, with one compact tuple on the wire.
type WireCell = (u32, u16, Rgb, Option<Rgb>, Option<Rgb>, u16);
impl From<Cell> for WireCell {
    fn from(c: Cell) -> Self {
        let s = c.style;
        let mut flags = [
            s.bold,
            s.italic,
            s.faint,
            s.blink,
            s.inverse,
            s.invisible,
            s.strikethrough,
            s.overline,
        ]
        .into_iter()
        .enumerate()
        .fold(0, |bits, (i, on)| bits | ((on as u16) << i));
        flags |= (match s.underline {
            Underline::None => 0,
            Underline::Single => 1,
            Underline::Double => 2,
            Underline::Curly => 3,
            Underline::Dotted => 4,
            Underline::Dashed => 5,
        }) << 8;
        flags |= (match c.width {
            CellWidth::Narrow => 0,
            CellWidth::Wide => 1,
            CellWidth::SpacerTail => 2,
            CellWidth::SpacerHead => 3,
        }) << 11;
        flags |= (c.selected as u16) << 13;
        flags |= (match c.search {
            crate::SearchMark::None => 0,
            crate::SearchMark::Match => 1,
            crate::SearchMark::Current => 2,
        }) << 14;
        (
            c.text_start,
            c.text_len,
            c.fg,
            c.bg,
            c.underline_color,
            flags,
        )
    }
}
impl From<WireCell> for Cell {
    fn from((text_start, text_len, fg, bg, underline_color, f): WireCell) -> Self {
        Self {
            text_start,
            text_len,
            fg,
            bg,
            underline_color,
            width: match (f >> 11) & 3 {
                1 => CellWidth::Wide,
                2 => CellWidth::SpacerTail,
                3 => CellWidth::SpacerHead,
                _ => CellWidth::Narrow,
            },
            selected: f & (1 << 13) != 0,
            search: match (f >> 14) & 3 {
                1 => crate::SearchMark::Match,
                2 => crate::SearchMark::Current,
                _ => crate::SearchMark::None,
            },
            style: CellStyle {
                bold: f & 1 != 0,
                italic: f & 2 != 0,
                faint: f & 4 != 0,
                blink: f & 8 != 0,
                inverse: f & 16 != 0,
                invisible: f & 32 != 0,
                strikethrough: f & 64 != 0,
                overline: f & 128 != 0,
                underline: match (f >> 8) & 7 {
                    1 => Underline::Single,
                    2 => Underline::Double,
                    3 => Underline::Curly,
                    4 => Underline::Dotted,
                    5 => Underline::Dashed,
                    _ => Underline::None,
                },
            },
        }
    }
}

impl Cell {
    /// Whether the cell has glyphs to draw.
    pub fn has_text(&self) -> bool {
        self.text_len > 0
    }
}

/// Per-row metadata.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// The row is soft-wrapped into the next one.
    pub wrapped: bool,
    pub semantic_prompt: SemanticPrompt,
}

/// OSC 133 prompt marking for a row.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SemanticPrompt {
    #[default]
    None,
    Prompt,
    Continuation,
}

/// Cursor shape.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    #[default]
    Block,
    Underline,
    BlockHollow,
}

/// Cursor state in viewport coordinates.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// Everything a renderer needs to draw one frame.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
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
    /// The application asked for mouse reports, so clicks belong to it.
    pub mouse_tracking: bool,
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

    /// The cursor is visible on a shell prompt row (OSC 133), so the shell
    /// is waiting for a command rather than running one.
    pub fn at_prompt(&self) -> bool {
        self.cursor
            .and_then(|c| self.rows.get(usize::from(c.y)))
            .is_some_and(|r| r.semantic_prompt != SemanticPrompt::None)
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

#[cfg(test)]
mod wire_tests {
    use super::*;

    #[test]
    fn wire_cells_preserve_glyph_ranges_colors_and_every_attribute() {
        for underline in [
            Underline::None,
            Underline::Single,
            Underline::Double,
            Underline::Curly,
            Underline::Dotted,
            Underline::Dashed,
        ] {
            for width in [
                CellWidth::Narrow,
                CellWidth::Wide,
                CellWidth::SpacerTail,
                CellWidth::SpacerHead,
            ] {
                for search in [
                    crate::SearchMark::None,
                    crate::SearchMark::Match,
                    crate::SearchMark::Current,
                ] {
                    for bits in 0..512 {
                        let cell = Cell {
                            text_start: 1234,
                            text_len: 9,
                            width,
                            fg: Rgb {
                                r: 23,
                                g: 45,
                                b: 67,
                            },
                            bg: Some(Rgb {
                                r: 89,
                                g: 1,
                                b: 255,
                            }),
                            underline_color: Some(Rgb {
                                r: 11,
                                g: 22,
                                b: 33,
                            }),
                            style: CellStyle {
                                bold: bits & 1 != 0,
                                italic: bits & 2 != 0,
                                faint: bits & 4 != 0,
                                blink: bits & 8 != 0,
                                inverse: bits & 16 != 0,
                                invisible: bits & 32 != 0,
                                strikethrough: bits & 64 != 0,
                                overline: bits & 128 != 0,
                                underline,
                            },
                            selected: bits & 256 != 0,
                            search,
                        };
                        let bytes = serde_json::to_vec(&cell).unwrap();
                        assert!(bytes.len() < 80);
                        assert_eq!(serde_json::from_slice::<Cell>(&bytes).unwrap(), cell);
                    }
                }
            }
        }
        let cell = Cell::default();
        assert_eq!(
            serde_json::from_slice::<Cell>(&serde_json::to_vec(&cell).unwrap()).unwrap(),
            cell
        );
    }
}
