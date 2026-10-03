//! Fonts bundled with chda, as Ghostty bundles its own: JetBrains Mono for
//! text and Symbols Nerd Font Mono for the icons prompts and TUIs use.

use std::borrow::Cow;

use gpui::{App, FontFallbacks};

use crate::platform;

/// Default terminal family. Its programming ligatures stay off unless the
/// config turns them on (`font-feature = calt`).
pub const DEFAULT_FAMILY: &str = "JetBrains Mono";

/// Fallback for Nerd Font icons (private use area) that text fonts lack,
/// and the family the sidebar draws its badge icons in.
pub(crate) const SYMBOLS_FAMILY: &str = "Symbols Nerd Font Mono";

const TEXT_FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

const SYMBOLS_FONT: &[u8] = include_bytes!("../assets/fonts/SymbolsNerdFontMono-Regular.ttf");

/// Make the bundled fonts available. Call once at startup, before any text
/// is laid out.
pub fn register(cx: &App) {
    if let Err(e) = cx
        .text_system()
        .add_fonts(TEXT_FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
    {
        eprintln!("chda: could not load the bundled fonts: {e}");
    }
    platform::register_fallback_font("SymbolsNerdFontMono-Regular.ttf", SYMBOLS_FONT, cx);
}

/// Fallbacks every terminal font gets.
pub fn fallbacks() -> FontFallbacks {
    FontFallbacks::from_fonts(vec![SYMBOLS_FAMILY.into()])
}
