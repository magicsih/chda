//! Fonts bundled with chda, as Ghostty bundles its own: JetBrains Mono for
//! text and its Nerd Font variant for the icons prompts and TUIs use.

use std::borrow::Cow;

use gpui::{App, FontFallbacks};

/// Default terminal family. Its programming ligatures stay off unless the
/// config turns them on (`font-feature = calt`).
pub const DEFAULT_FAMILY: &str = "JetBrains Mono";

/// Fallback for Nerd Font icons (private use area) that text fonts lack,
/// and the family the sidebar draws its badge icons in.
pub(crate) const SYMBOLS_FAMILY: &str = "JetBrainsMono NFM";

const TEXT_FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

const SYMBOLS_FONT: &[u8] = include_bytes!("../assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf");

/// Make the bundled fonts available. Call once at startup, before any text
/// is laid out.
pub fn register(cx: &App) {
    if let Err(e) = cx
        .text_system()
        .add_fonts(TEXT_FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
    {
        eprintln!("chda: could not load the bundled fonts: {e}");
    }
    if let Err(e) = cx
        .text_system()
        .add_fonts(vec![Cow::Borrowed(SYMBOLS_FONT)])
    {
        eprintln!("chda: could not load the bundled icon font: {e}");
    }
}

/// Fallbacks every terminal font gets.
pub fn fallbacks() -> FontFallbacks {
    FontFallbacks::from_fonts(vec![SYMBOLS_FAMILY.into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icons_are_loadable_by_the_platform_text_system() {
        let face = ttf_parser::Face::parse(SYMBOLS_FONT, 0).unwrap();
        // GPUI rejects a family without this glyph before shaping any icons.
        assert!(face.glyph_index('m').is_some());
        for icon in ['\u{f10aa}', '\u{f054e}', '\u{f054f}', '\u{f062d}'] {
            assert!(face.glyph_index(icon).is_some(), "missing icon {icon}");
        }
        assert!(face.names().into_iter().any(|name| {
            name.name_id == ttf_parser::name_id::FAMILY
                && name.to_string().as_deref() == Some(SYMBOLS_FAMILY)
        }));
    }
}
