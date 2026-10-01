//! UI settings derived from the Ghostty config subset.

use chda_config::{Color, CursorStyle, GhosttyConfig, Padding};
use chda_term::{ColorConfig, CursorShape, DEFAULT_SCROLLBACK, Rgb, ShellIntegration};

use crate::platform;
use gpui::{Font, FontFeatures, FontStyle, FontWeight, Pixels, px};

#[derive(Clone, Debug)]
pub struct Settings {
    pub font_family: String,
    pub font_size: Pixels,
    pub padding: Padding,
    pub colors: ColorConfig,
    pub selection_background: Option<Rgb>,
    pub selection_foreground: Option<Rgb>,
    /// Scrollback limit in bytes.
    pub scrollback: usize,
    pub shell_integration: ShellIntegration,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_ghostty(&GhosttyConfig::default())
    }
}

fn rgb(c: Color) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

impl Settings {
    /// Ghostty's defaults fill in whatever the config leaves unset.
    pub fn from_ghostty(c: &GhosttyConfig) -> Self {
        Self {
            font_family: c
                .font_family
                .clone()
                .unwrap_or_else(|| platform::DEFAULT_MONOSPACE.into()),
            font_size: px(c.font_size.unwrap_or(13.0)),
            padding: c.padding,
            colors: ColorConfig {
                foreground: Some(rgb(c.foreground.unwrap_or(Color {
                    r: 0xff,
                    g: 0xff,
                    b: 0xff,
                }))),
                background: Some(rgb(c.background.unwrap_or(Color {
                    r: 0x28,
                    g: 0x2c,
                    b: 0x34,
                }))),
                cursor: c.cursor_color.map(rgb),
                palette: c
                    .palette_colors()
                    .into_iter()
                    .map(|(i, c)| (i, rgb(c)))
                    .collect(),
                cursor_shape: c.cursor_style.map(|s| match s {
                    CursorStyle::Block => CursorShape::Block,
                    CursorStyle::Bar => CursorShape::Bar,
                    CursorStyle::Underline => CursorShape::Underline,
                    CursorStyle::BlockHollow => CursorShape::BlockHollow,
                }),
            },
            selection_background: c.selection_background.map(rgb),
            selection_foreground: c.selection_foreground.map(rgb),
            scrollback: c
                .scrollback_limit
                .map(|b| usize::try_from(b).unwrap_or(usize::MAX))
                .unwrap_or(DEFAULT_SCROLLBACK),
            shell_integration: match c.shell_integration.as_deref() {
                Some("none") => ShellIntegration::None,
                _ => ShellIntegration::Detect,
            },
        }
    }

    pub fn font(&self) -> Font {
        Font {
            family: self.font_family.clone().into(),
            features: FontFeatures::disable_ligatures(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }
}
