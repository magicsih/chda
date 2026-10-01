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
