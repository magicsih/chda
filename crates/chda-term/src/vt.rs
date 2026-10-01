//! Single-threaded wrapper over `libghostty-vt`.

use std::cell::{Cell as StdCell, RefCell};
use std::rc::Rc;

use libghostty_vt::key::{self, Encoder as KeyEncoder};
use libghostty_vt::mouse::{self, Encoder as MouseEncoder};
use libghostty_vt::render::{
    CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator, Snapshot,
};
use libghostty_vt::screen::{CellWide, RowSemanticPrompt, Screen, TrackedGridRef};
use libghostty_vt::selection::{FormatOptions, SelectLineOptions, SelectWordOptions, Selection};
use libghostty_vt::style::{Palette, PaletteIndex, RgbColor, StyleColor, Underline as VtUnderline};
use libghostty_vt::terminal::{
    CursorStyle as VtCursorStyle, Mode, Options, Point, PointCoordinate, ScrollViewport,
    Terminal as VtTerminal,
};
use libghostty_vt::{focus, paste};

use crate::frame::{
    Cell, CellStyle, CellWidth, ColorConfig, Cursor, CursorShape, Frame, Rgb, Row, Scrollbar,
    SemanticPrompt, Size, Underline,
};
use crate::input::{KeyAction, KeyCode, KeyInput, Modifiers, MouseAction, MouseButton, MouseInput};

/// Error raised by the terminal core.
#[derive(Clone, Copy, Debug)]
pub struct Error(libghostty_vt::Error);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<libghostty_vt::Error> for Error {
    fn from(value: libghostty_vt::Error) -> Self {
        Self(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Side effects produced while feeding bytes, drained after each feed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Bytes the application must send back to the PTY (query replies).
    pub pty_output: Vec<u8>,
    pub bell: bool,
    pub title_changed: bool,
    pub pwd_changed: bool,
    /// Text an application asked to put on the clipboard (OSC 52).
    pub clipboard_write: Option<String>,
}

#[derive(Default)]
struct Hooks {
    pty_output: RefCell<Vec<u8>>,
    bell: StdCell<bool>,
    title_changed: StdCell<bool>,
    pwd_changed: StdCell<bool>,
    clipboard_write: RefCell<Option<String>>,
}

/// A terminal: VT parser plus screen state, driven from a single thread.
pub struct Terminal {
    vt: VtTerminal<'static, 'static>,
    render: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    keys: KeyEncoder<'static>,
    mouse: MouseEncoder<'static>,
    /// Where a left-button selection started.
    anchor: Option<TrackedGridRef>,
    cell_px: (u32, u32),
    hooks: Rc<Hooks>,
    generation: u64,
}

impl Terminal {
    /// Create a terminal with the given size and scrollback limit in bytes.
    pub fn new(size: Size, max_scrollback: usize) -> Result<Self> {
        let mut vt = VtTerminal::new(Options {
            cols: size.cols,
            rows: size.rows,
            max_scrollback,
        })?;
        let hooks = Rc::new(Hooks::default());
        vt.on_pty_write({
            let h = Rc::clone(&hooks);
            move |_, data| h.pty_output.borrow_mut().extend_from_slice(data)
        })?
        .on_bell({
            let h = Rc::clone(&hooks);
            move |_| h.bell.set(true)
        })?
        .on_title_changed({
            let h = Rc::clone(&hooks);
            move |_| h.title_changed.set(true)
        })?
        .on_pwd_changed({
            let h = Rc::clone(&hooks);
            move |_| h.pwd_changed.set(true)
        })?
        .on_clipboard_write({
            let h = Rc::clone(&hooks);
            move |_, write| {
                let text = write
                    .contents()
                    .find(|c| c.mime.is_empty() || c.mime.starts_with("text/"))
                    .map(|c| c.data.to_owned())
                    .unwrap_or_default();
                *h.clipboard_write.borrow_mut() = Some(text);
                Ok(())
            }
        })?;

        Ok(Self {
            vt,
            render: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            keys: KeyEncoder::new()?,
            mouse: MouseEncoder::new()?,
            anchor: None,
            cell_px: (0, 0),
            hooks,
            generation: 0,
        })
    }

    /// Set the default colors and cursor shape (the values a reset returns to).
    pub fn set_colors(&mut self, colors: &ColorConfig) -> Result<()> {
        let vt = &mut self.vt;
        vt.set_default_fg_color(colors.foreground.map(vt_rgb))?;
        vt.set_default_bg_color(colors.background.map(vt_rgb))?;
        vt.set_default_cursor_color(colors.cursor.map(vt_rgb))?;
        if !colors.palette.is_empty() {
            let mut palette = Palette::default();
            for &(i, c) in &colors.palette {
                palette.set(PaletteIndex(i), vt_rgb(c));
            }
            vt.set_default_color_palette(Some(palette))?;
        }
        vt.set_default_cursor_style(colors.cursor_shape.map(|s| match s {
            CursorShape::Bar => VtCursorStyle::Bar,
            CursorShape::Block => VtCursorStyle::Block,
            CursorShape::Underline => VtCursorStyle::Underline,
            CursorShape::BlockHollow => VtCursorStyle::BlockHollow,
        }))?;
        Ok(())
    }

    /// Feed raw bytes from the PTY into the VT parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.vt.vt_write(bytes);
    }

    /// Take the side effects accumulated since the last call.
    pub fn take_effects(&mut self) -> Effects {
        let h = &self.hooks;
        Effects {
            pty_output: std::mem::take(&mut *h.pty_output.borrow_mut()),
            bell: h.bell.replace(false),
            title_changed: h.title_changed.replace(false),
            pwd_changed: h.pwd_changed.replace(false),
            clipboard_write: h.clipboard_write.borrow_mut().take(),
        }
    }

    pub fn resize(&mut self, size: Size, cell_width_px: u32, cell_height_px: u32) -> Result<()> {
        self.cell_px = (cell_width_px, cell_height_px);
        self.vt
            .resize(size.cols, size.rows, cell_width_px, cell_height_px)?;
        Ok(())
    }

    /// Whether the application asked for mouse reports.
    pub fn mouse_tracking(&self) -> bool {
        self.vt.is_mouse_tracking().unwrap_or(false)
    }

    /// Alternate scroll: wheel events become arrow keys in full-screen apps.
    pub fn alt_scroll(&self) -> bool {
        self.alternate_screen() && self.vt.mode(Mode::ALT_SCROLL).unwrap_or(true)
    }

    /// Encode a mouse event for an application that tracks the mouse.
    /// Returns an empty vector when the current mode reports nothing.
    pub fn encode_mouse(&mut self, input: &MouseInput) -> Result<Vec<u8>> {
        let mut event = mouse::Event::new()?;
        event
            .set_action(match input.action {
                MouseAction::Down { .. } => mouse::Action::Press,
                MouseAction::Up => mouse::Action::Release,
                MouseAction::Drag | MouseAction::Move => mouse::Action::Motion,
            })
            .set_button(match input.action {
                MouseAction::Move => None,
                _ => Some(match input.button {
                    MouseButton::Left => mouse::Button::Left,
                    MouseButton::Middle => mouse::Button::Middle,
                    MouseButton::Right => mouse::Button::Right,
                }),
            })
            .set_mods(key_mods(input.mods))
            .set_position(mouse::Position {
                x: input.px_x as f32,
                y: input.px_y as f32,
            });
        self.mouse_options();
        self.mouse
            .set_any_button_pressed(matches!(input.action, MouseAction::Drag));
        let mut out = Vec::new();
        self.mouse.encode_to_vec(&event, &mut out)?;
        Ok(out)
    }

    /// Encode one wheel notch as a mouse report (buttons 4 and 5).
    pub fn encode_wheel(
        &mut self,
        up: bool,
        mods: Modifiers,
        px_x: u32,
        px_y: u32,
    ) -> Result<Vec<u8>> {
        let mut event = mouse::Event::new()?;
        event
            .set_action(mouse::Action::Press)
            .set_button(Some(if up {
                mouse::Button::Four
            } else {
                mouse::Button::Five
            }))
            .set_mods(key_mods(mods))
            .set_position(mouse::Position {
                x: px_x as f32,
                y: px_y as f32,
            });
        self.mouse_options();
        let mut out = Vec::new();
        self.mouse.encode_to_vec(&event, &mut out)?;
        Ok(out)
    }

    fn mouse_options(&mut self) {
        let (cw, ch) = self.cell_px;
        let cols = u32::from(self.vt.cols().unwrap_or(1));
        let rows = u32::from(self.vt.rows().unwrap_or(1));
        self.mouse
            .set_options_from_terminal(&self.vt)
            .set_size(mouse::EncoderSize {
                screen_width: cw.max(1) * cols,
                screen_height: ch.max(1) * rows,
                cell_width: cw.max(1),
                cell_height: ch.max(1),
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            });
    }

    /// Update the selection from a left-button gesture.
    pub fn select(&mut self, input: &MouseInput) -> Result<()> {
        let point = Point::Viewport(PointCoordinate {
            x: input.cell_x,
            y: u32::from(input.cell_y),
        });
        match input.action {
            MouseAction::Down { click_count } => {
                self.anchor = None;
                self.vt.set_selection(None)?;
                let here = self.vt.grid_ref(point)?;
                match click_count {
                    1 => self.anchor = Some(self.vt.track_grid_ref(point)?),
                    2 => {
                        let sel = self.vt.select_word(SelectWordOptions::new(here))?;
                        self.vt.set_selection(sel.as_ref())?;
                    }
                    _ => {
                        let sel = self.vt.select_line(SelectLineOptions::new(here))?;
                        self.vt.set_selection(sel.as_ref())?;
                    }
                }
            }
            MouseAction::Drag => {
                let Some(anchor) = self.anchor.as_ref() else {
                    return Ok(());
                };
                let Some(start) = anchor.snapshot(&self.vt)? else {
                    return Ok(());
                };
                let end = self.vt.grid_ref(point)?;
                let sel = Selection::new(start, end, input.mods.alt);
                self.vt.set_selection(Some(&sel))?;
            }
            MouseAction::Up | MouseAction::Move => {}
        }
        Ok(())
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
        let _ = self.vt.set_selection(None);
    }

    /// The selected text, unwrapped and trimmed, if anything is selected.
    pub fn selection_text(&self) -> Result<Option<String>> {
        let opts = FormatOptions::new().with_unwrap(true).with_trim(true);
        Ok(self
            .vt
            .format_selection_alloc(None, opts)?
            .map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    pub fn title(&self) -> String {
        self.vt.title().map(str::to_owned).unwrap_or_default()
    }

    pub fn pwd(&self) -> String {
        self.vt.pwd().map(str::to_owned).unwrap_or_default()
    }

    /// Scroll the viewport by `delta` rows; negative is towards history.
    pub fn scroll(&mut self, delta: isize) {
        self.vt.scroll_viewport(ScrollViewport::Delta(delta));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.vt.scroll_viewport(ScrollViewport::Bottom);
    }

    /// Whether the alternate screen (full-screen app) is active.
    pub fn alternate_screen(&self) -> bool {
        matches!(self.vt.active_screen(), Ok(Screen::Alternate))
    }

    /// Encode a key event to the bytes the application expects.
    /// Returns an empty vector when the key produces no output.
    pub fn encode_key(&mut self, input: &KeyInput) -> Result<Vec<u8>> {
        let mut event = key::Event::new()?;
        let action = match input.action {
            KeyAction::Press => key::Action::Press,
            KeyAction::Repeat => key::Action::Repeat,
            KeyAction::Release => key::Action::Release,
        };
        let (vt_key, unshifted) = map_key(input.key);
        event
            .set_action(action)
            .set_key(vt_key)
            .set_mods(key_mods(input.mods))
            .set_utf8(input.text.as_deref().filter(|t| !t.is_empty()));
        if let Some(c) = unshifted {
            event.set_unshifted_codepoint(c);
        }
        self.keys
            .set_options_from_terminal(&self.vt)
            .set_macos_option_as_alt(key::OptionAsAlt::True);
        let mut out = Vec::new();
        self.keys.encode_to_vec(&event, &mut out)?;
        Ok(out)
    }

    /// Encode committed text. Plain modes send the UTF-8 as is; when the
    /// application enabled the kitty keyboard protocol each character goes
    /// through the key encoder so it is reported the way the app expects.
    pub fn encode_text(&mut self, text: &str) -> Result<Vec<u8>> {
        let flags = self.vt.kitty_keyboard_flags()?;
        if flags.is_empty() {
            return Ok(text.as_bytes().to_vec());
        }
        let mut out = Vec::new();
        for c in text.chars() {
            out.extend(self.encode_key(&KeyInput {
                action: KeyAction::Press,
                key: KeyCode::Char(c),
                mods: Modifiers::default(),
                text: Some(c.to_string()),
            })?);
        }
        Ok(out)
    }

    /// Encode pasted text, honoring bracketed paste mode.
    pub fn encode_paste(&self, text: &str) -> Result<Vec<u8>> {
        let bracketed = self.vt.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
        let mut data = text.as_bytes().to_vec();
        let mut buf = vec![0u8; data.len() + 16];
        let n = match paste::encode(&mut data, bracketed, &mut buf) {
            Ok(n) => n,
            Err(libghostty_vt::Error::OutOfSpace { required }) => {
                buf.resize(required, 0);
                paste::encode(&mut data, bracketed, &mut buf)?
            }
            Err(e) => return Err(e.into()),
        };
        buf.truncate(n);
        Ok(buf)
    }

    /// Encode a focus change, or nothing when the app did not ask for them.
    pub fn encode_focus(&self, focused: bool) -> Vec<u8> {
        if !self.vt.mode(Mode::FOCUS_EVENT).unwrap_or(false) {
            return Vec::new();
        }
        let event = if focused {
            focus::Event::Gained
        } else {
            focus::Event::Lost
        };
        let mut buf = [0u8; 8];
        match event.encode(&mut buf) {
            Ok(n) => buf[..n].to_vec(),
            Err(_) => Vec::new(),
        }
    }

    /// Whether anything changed since the last [`Terminal::frame`].
    pub fn is_dirty(&mut self) -> Result<bool> {
        let snapshot = self.render.update(&self.vt)?;
        Ok(snapshot.dirty()? != Dirty::Clean)
    }

    /// Build a plain-data snapshot of the viewport.
    pub fn frame(&mut self) -> Result<Frame> {
        self.generation += 1;
        let title = self.title();
        let pwd = self.pwd();
        let alternate_screen = self.alternate_screen();
        let scrollbar = self.vt.scrollbar().map(|s| Scrollbar {
            total: s.total,
            offset: s.offset,
            len: s.len,
        })?;

        let snapshot = self.render.update(&self.vt)?;
        let colors = snapshot.colors()?;
        let size = Size {
            cols: snapshot.cols()?,
            rows: snapshot.rows()?,
        };
        let mut frame = Frame {
            size,
            cells: Vec::with_capacity(usize::from(size.cols) * usize::from(size.rows)),
            rows: Vec::with_capacity(usize::from(size.rows)),
            text: String::new(),
            cursor: cursor_from(&snapshot)?,
            foreground: rgb(colors.foreground),
            background: rgb(colors.background),
            scrollbar,
            title,
            pwd,
            alternate_screen,
            generation: self.generation,
        };

        let mut text = String::new();
        {
            let mut row_iter = self.rows.update(&snapshot)?;
            while let Some(row) = row_iter.next() {
                let raw = row.raw_row()?;
                frame.rows.push(Row {
                    wrapped: raw.is_wrapped().unwrap_or(false),
                    semantic_prompt: match raw.semantic_prompt() {
                        Ok(RowSemanticPrompt::Prompt) => SemanticPrompt::Prompt,
                        Ok(RowSemanticPrompt::Continuation) => SemanticPrompt::Continuation,
                        _ => SemanticPrompt::None,
                    },
                });

                let mut cell_iter = self.cells.update(row)?;
                while let Some(c) = cell_iter.next() {
                    let style = c.style()?;
                    let raw = c.raw_cell()?;
                    let width = match raw.wide() {
                        Ok(CellWide::Wide) => CellWidth::Wide,
                        Ok(CellWide::SpacerTail) => CellWidth::SpacerTail,
                        Ok(CellWide::SpacerHead) => CellWidth::SpacerHead,
                        _ => CellWidth::Narrow,
                    };
                    let mut fg = c.fg_color()?.map(rgb).unwrap_or(frame.foreground);
                    let mut bg = c.bg_color()?.map(rgb);
                    if style.inverse {
                        let new_fg = bg.unwrap_or(frame.background);
                        bg = Some(fg);
                        fg = new_fg;
                    }
                    let underline_color = match style.underline_color {
                        StyleColor::Rgb(c) => Some(rgb(c)),
                        StyleColor::Palette(i) => Some(rgb(colors.palette[usize::from(i.0)])),
                        StyleColor::None => None,
                    };
                    let mut cell = Cell {
                        width,
                        fg,
                        bg,
                        underline_color,
                        style: CellStyle {
                            bold: style.bold,
                            italic: style.italic,
                            faint: style.faint,
                            blink: style.blink,
                            inverse: style.inverse,
                            invisible: style.invisible,
                            strikethrough: style.strikethrough,
                            overline: style.overline,
                            underline: match style.underline {
                                VtUnderline::Single => Underline::Single,
                                VtUnderline::Double => Underline::Double,
                                VtUnderline::Curly => Underline::Curly,
                                VtUnderline::Dotted => Underline::Dotted,
                                VtUnderline::Dashed => Underline::Dashed,
                                _ => Underline::None,
                            },
                        },
                        selected: c.is_selected()?,
                        ..Cell::default()
                    };
                    if c.graphemes_len()? > 0 {
                        text.clear();
                        c.graphemes_utf8(&mut text)?;
                        let (start, len) = frame.push_cell_text(&text);
                        Frame::set_cell_text(&mut cell, start, len);
                    }
                    frame.cells.push(cell);
                }
                row.set_dirty(false)?;
            }
        }
        snapshot.set_dirty(Dirty::Clean)?;
        Ok(frame)
    }
}

fn key_mods(m: Modifiers) -> key::Mods {
    let mut mods = key::Mods::empty();
    mods.set(key::Mods::SHIFT, m.shift);
    mods.set(key::Mods::ALT, m.alt);
    mods.set(key::Mods::CTRL, m.ctrl);
    mods.set(key::Mods::SUPER, m.meta);
    mods
}

fn vt_rgb(c: Rgb) -> RgbColor {
    RgbColor {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

fn rgb(c: RgbColor) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

fn cursor_from(snapshot: &Snapshot<'_, '_>) -> Result<Option<Cursor>> {
    if !snapshot.cursor_visible()? {
        return Ok(None);
    }
    let Some(vp) = snapshot.cursor_viewport()? else {
        return Ok(None);
    };
    Ok(Some(Cursor {
        x: vp.x,
        y: vp.y,
        shape: match snapshot.cursor_visual_style()? {
            CursorVisualStyle::Bar => CursorShape::Bar,
            CursorVisualStyle::Underline => CursorShape::Underline,
            CursorVisualStyle::BlockHollow => CursorShape::BlockHollow,
            _ => CursorShape::Block,
        },
        blinking: snapshot.cursor_blinking()?,
        at_wide_tail: vp.at_wide_tail,
        color: snapshot.cursor_color()?.map(rgb),
    }))
}

/// Map a logical key to the W3C key code libghostty expects, plus the
/// unshifted codepoint for printable keys.
fn map_key(code: KeyCode) -> (key::Key, Option<char>) {
    use key::Key as K;
    let k = match code {
        KeyCode::Char(c) => {
            let k = match c.to_ascii_lowercase() {
                'a' => K::A,
                'b' => K::B,
                'c' => K::C,
                'd' => K::D,
                'e' => K::E,
                'f' => K::F,
                'g' => K::G,
                'h' => K::H,
                'i' => K::I,
                'j' => K::J,
                'k' => K::K,
                'l' => K::L,
                'm' => K::M,
                'n' => K::N,
                'o' => K::O,
                'p' => K::P,
                'q' => K::Q,
                'r' => K::R,
                's' => K::S,
                't' => K::T,
                'u' => K::U,
                'v' => K::V,
                'w' => K::W,
                'x' => K::X,
                'y' => K::Y,
                'z' => K::Z,
                '0' => K::Digit0,
                '1' => K::Digit1,
                '2' => K::Digit2,
                '3' => K::Digit3,
                '4' => K::Digit4,
                '5' => K::Digit5,
                '6' => K::Digit6,
                '7' => K::Digit7,
                '8' => K::Digit8,
                '9' => K::Digit9,
                '`' => K::Backquote,
                '\\' => K::Backslash,
                '[' => K::BracketLeft,
                ']' => K::BracketRight,
                ',' => K::Comma,
                '=' => K::Equal,
                '-' => K::Minus,
                '.' => K::Period,
                '\'' => K::Quote,
                ';' => K::Semicolon,
                '/' => K::Slash,
                ' ' => K::Space,
                _ => K::Unidentified,
            };
            return (k, Some(c));
        }
        KeyCode::Enter => K::Enter,
        KeyCode::Escape => K::Escape,
        KeyCode::Backspace => K::Backspace,
        KeyCode::Tab => K::Tab,
        KeyCode::Space => return (K::Space, Some(' ')),
        KeyCode::Delete => K::Delete,
        KeyCode::Insert => K::Insert,
        KeyCode::Home => K::Home,
        KeyCode::End => K::End,
        KeyCode::PageUp => K::PageUp,
        KeyCode::PageDown => K::PageDown,
        KeyCode::ArrowUp => K::ArrowUp,
        KeyCode::ArrowDown => K::ArrowDown,
        KeyCode::ArrowLeft => K::ArrowLeft,
        KeyCode::ArrowRight => K::ArrowRight,
        KeyCode::F(n) => match n {
            1 => K::F1,
            2 => K::F2,
            3 => K::F3,
            4 => K::F4,
            5 => K::F5,
            6 => K::F6,
            7 => K::F7,
            8 => K::F8,
            9 => K::F9,
            10 => K::F10,
            11 => K::F11,
            12 => K::F12,
            _ => K::Unidentified,
        },
    };
    (k, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Modifiers;

    fn term() -> Terminal {
        Terminal::new(Size { cols: 20, rows: 5 }, 100).unwrap()
    }

    #[test]
    fn feed_moves_cursor_and_frame_reports_it() {
        let size = Size { cols: 20, rows: 5 };
        let mut term = term();

        let frame = term.frame().unwrap();
        assert_eq!(frame.size, size);
        assert_eq!(frame.cursor.map(|c| (c.x, c.y)), Some((0, 0)));
        assert_eq!(frame.cells.len(), 100);

        term.feed(b"hello\r\n");
        let frame = term.frame().unwrap();
        assert_eq!(frame.cursor.map(|c| (c.x, c.y)), Some((0, 1)));
        assert_eq!(frame.row_text(0), "hello");

        term.feed(b"\x1b[3;7H");
        let frame = term.frame().unwrap();
        assert_eq!(frame.cursor.map(|c| (c.x, c.y)), Some((6, 2)));
    }

    #[test]
    fn styles_colors_and_wide_glyphs() {
        let mut term = term();
        term.feed(b"\x1b[1;4;31mab\x1b[0m \xe4\xb8\xad");
        let frame = term.frame().unwrap();
        let a = frame.cell(0, 0).unwrap();
        assert!(a.style.bold);
        assert_eq!(a.style.underline, Underline::Single);
        assert_ne!(a.fg, frame.foreground);
        assert_eq!(frame.cell_text(a), "a");
        let plain = frame.cell(2, 0).unwrap();
        assert_eq!(frame.cell_text(plain), " ");
        assert!(!frame.cell(10, 0).unwrap().has_text());
        let wide = frame.cell(3, 0).unwrap();
        assert_eq!(wide.width, CellWidth::Wide);
        assert_eq!(frame.cell_text(wide), "中");
        assert_eq!(frame.cell(4, 0).unwrap().width, CellWidth::SpacerTail);
        assert_eq!(frame.row_text(0), "ab 中");
    }

    #[test]
    fn title_pwd_bell_and_query_reply_effects() {
        let mut term = term();
        term.feed(b"\x1b]2;my title\x07\x1b]7;file://host/tmp/x\x07\x07\x1b[6n");
        let fx = term.take_effects();
        assert!(fx.bell);
        assert!(fx.title_changed);
        assert!(fx.pwd_changed);
        assert_eq!(fx.pty_output, b"\x1b[1;1R");
        assert_eq!(term.title(), "my title");
        assert_eq!(term.pwd(), "file://host/tmp/x");
        assert_eq!(term.take_effects(), Effects::default());
    }

    #[test]
    fn key_encoding_follows_terminal_modes() {
        let mut term = term();
        let press = |key, mods, text: Option<&str>| KeyInput {
            action: KeyAction::Press,
            key,
            mods,
            text: text.map(str::to_owned),
        };
        assert_eq!(
            term.encode_key(&press(KeyCode::Char('a'), Modifiers::default(), Some("a")))
                .unwrap(),
            b"a"
        );
        assert_eq!(
            term.encode_key(&press(
                KeyCode::Char('c'),
                Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                None
            ))
            .unwrap(),
            b"\x03"
        );
        assert_eq!(
            term.encode_key(&press(KeyCode::ArrowUp, Modifiers::default(), None))
                .unwrap(),
            b"\x1b[A"
        );
        term.feed(b"\x1b[?1h");
        assert_eq!(
            term.encode_key(&press(KeyCode::ArrowUp, Modifiers::default(), None))
                .unwrap(),
            b"\x1bOA"
        );
        assert_eq!(term.encode_text("한a").unwrap(), "한a".as_bytes());
        term.feed(b"\x1b[>1u");
        assert_eq!(term.encode_text("a").unwrap(), b"a");
        term.feed(b"\x1b[>8u");
        assert_eq!(term.encode_text("a").unwrap(), b"\x1b[97u");
        term.feed(b"\x1b[<u\x1b[<u");
        assert_eq!(term.encode_paste("a\nb").unwrap(), b"a\rb");
        term.feed(b"\x1b[?2004h");
        assert_eq!(
            term.encode_paste("a\nb").unwrap(),
            b"\x1b[200~a\nb\x1b[201~"
        );
        assert!(term.encode_focus(true).is_empty());
        term.feed(b"\x1b[?1004h");
        assert_eq!(term.encode_focus(true), b"\x1b[I");
    }

    fn mouse(action: MouseAction, x: u16, y: u16) -> MouseInput {
        MouseInput {
            action,
            button: MouseButton::Left,
            mods: Modifiers::default(),
            cell_x: x,
            cell_y: y,
            px_x: u32::from(x) * 8 + 4,
            px_y: u32::from(y) * 16 + 8,
        }
    }

    #[test]
    fn drag_selects_text_and_double_click_selects_a_word() {
        let mut term = term();
        term.feed(b"hello brave world\r\nsecond line");
        term.select(&mouse(MouseAction::Down { click_count: 1 }, 6, 0))
            .unwrap();
        term.select(&mouse(MouseAction::Drag, 2, 1)).unwrap();
        assert_eq!(
            term.selection_text().unwrap().as_deref(),
            Some("brave world\nsec")
        );
        let frame = term.frame().unwrap();
        assert!(frame.cell(6, 0).unwrap().selected);
        assert!(!frame.cell(5, 0).unwrap().selected);

        term.select(&mouse(MouseAction::Down { click_count: 2 }, 7, 0))
            .unwrap();
        assert_eq!(term.selection_text().unwrap().as_deref(), Some("brave"));
        term.select(&mouse(MouseAction::Down { click_count: 3 }, 7, 0))
            .unwrap();
        assert_eq!(
            term.selection_text().unwrap().as_deref(),
            Some("hello brave world")
        );
        term.clear_selection();
        assert_eq!(term.selection_text().unwrap(), None);
    }

    #[test]
    fn mouse_reports_follow_tracking_mode() {
        let mut term = term();
        term.resize(Size { cols: 20, rows: 5 }, 8, 16).unwrap();
        assert!(!term.mouse_tracking());
        term.feed(b"\x1b[?1000h\x1b[?1006h");
        assert!(term.mouse_tracking());
        let bytes = term
            .encode_mouse(&mouse(MouseAction::Down { click_count: 1 }, 3, 2))
            .unwrap();
        assert_eq!(bytes, b"\x1b[<0;4;3M");
        let bytes = term.encode_mouse(&mouse(MouseAction::Up, 3, 2)).unwrap();
        assert_eq!(bytes, b"\x1b[<0;4;3m");
        let bytes = term.encode_wheel(true, Modifiers::default(), 4, 8).unwrap();
        assert_eq!(bytes, b"\x1b[<64;1;1M");
    }

    #[test]
    fn default_colors_and_palette_apply() {
        let mut term = term();
        let red = Rgb { r: 255, g: 0, b: 0 };
        let bg = Rgb { r: 1, g: 2, b: 3 };
        term.set_colors(&ColorConfig {
            foreground: Some(Rgb { r: 9, g: 9, b: 9 }),
            background: Some(bg),
            cursor: None,
            palette: vec![(1, red)],
            cursor_shape: Some(CursorShape::Bar),
        })
        .unwrap();
        term.feed(b"\x1b[31mx");
        let frame = term.frame().unwrap();
        assert_eq!(frame.background, bg);
        assert_eq!(frame.foreground, Rgb { r: 9, g: 9, b: 9 });
        assert_eq!(frame.cell(0, 0).unwrap().fg, red);
        assert_eq!(frame.cursor.unwrap().shape, CursorShape::Bar);
    }

    #[test]
    fn scrollback_and_alternate_screen() {
        let mut term = term();
        for i in 0..10 {
            term.feed(format!("line{i}\r\n").as_bytes());
        }
        let frame = term.frame().unwrap();
        assert_eq!(frame.scrollbar.total, 11);
        assert_eq!(frame.row_text(0), "line6");
        term.scroll(-3);
        let frame = term.frame().unwrap();
        assert_eq!(frame.row_text(0), "line3");
        assert!(frame.cursor.is_none());
        term.scroll_to_bottom();
        assert!(term.frame().unwrap().cursor.is_some());
        assert!(!term.alternate_screen());
        term.feed(b"\x1b[?1049h");
        assert!(term.alternate_screen());
        assert!(term.frame().unwrap().alternate_screen);
    }
}

/// Throughput check for the parse-and-snapshot path. Run with
/// `CHDA_BENCH_FILE=<path> cargo test --release -p chda-term -- --ignored bench --nocapture`.
#[cfg(test)]
mod bench {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "throughput check; needs CHDA_BENCH_FILE"]
    fn bench_feed_and_frame() {
        let Some(path) = std::env::var_os("CHDA_BENCH_FILE") else {
            return;
        };
        let data = std::fs::read(path).unwrap();
        let mut term = Terminal::new(
            Size {
                cols: 120,
                rows: 40,
            },
            10_000,
        )
        .unwrap();
        let start = Instant::now();
        let mut frames = 0;
        let mut last_frame = start;
        for chunk in data.chunks(64 * 1024) {
            term.feed(chunk);
            if last_frame.elapsed() >= Duration::from_millis(8) {
                term.frame().unwrap();
                frames += 1;
                last_frame = Instant::now();
            }
        }
        term.frame().unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "fed {} bytes in {elapsed:?} ({:.1} MB/s), {frames} frames built",
            data.len(),
            data.len() as f64 / 1e6 / elapsed.as_secs_f64()
        );
    }
}
