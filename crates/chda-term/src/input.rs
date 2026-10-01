//! Platform-neutral input events, translated to VT byte sequences inside the
//! terminal thread.

/// Modifier keys held during an input event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
}

/// A keyboard key, named after the logical key rather than a scancode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    /// A printable character as it appears without shift applied
    /// (`a`, `1`, `/`). Shift is reported through [`Modifiers`].
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Space,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    F(u8),
}

/// Key press, repeat or release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

/// One keyboard event from the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub action: KeyAction,
    pub key: KeyCode,
    pub mods: Modifiers,
    /// Text the key produces after modifiers, if any (`A` for shift-a).
    pub text: Option<String>,
}

/// Mouse button, as the terminal protocol names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// `click_count` is 1 for a single click, 2 for a double click, 3 for triple.
    Down {
        click_count: u8,
    },
    Up,
    /// Movement with a button held.
    Drag,
    /// Movement with no button held.
    Move,
}

/// One mouse event, positioned relative to the top-left of the cell grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseInput {
    pub action: MouseAction,
    pub button: MouseButton,
    pub mods: Modifiers,
    /// Cell under the pointer, clamped to the grid.
    pub cell_x: u16,
    pub cell_y: u16,
    /// Pixel position, for SGR-pixels mouse reporting.
    pub px_x: u32,
    pub px_y: u32,
}
