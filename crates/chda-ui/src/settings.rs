//! UI settings derived from the Ghostty config subset.

use chda_config::{Color, CursorStyle, GhosttyConfig, Padding};
use chda_term::{ColorConfig, CursorShape, DEFAULT_SCROLLBACK, Rgb, ShellIntegration};

use crate::fonts;
use gpui::{Font, FontFeatures, FontStyle, FontWeight, Pixels, px};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub font_family: String,
    pub font_size: Pixels,
    /// OpenType features for the terminal font (`font-feature`).
    pub font_features: FontFeatures,
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

/// The configured features, the last setting for a tag winning. Programming
/// ligatures (`calt`) stay off unless the config turns them on with
/// `font-feature = calt`.
fn font_features(settings: &[(String, u32)]) -> FontFeatures {
    let mut features: Vec<(String, u32)> = Vec::new();
    for (tag, value) in settings {
        match features.iter_mut().find(|(t, _)| t == tag) {
            Some(slot) => slot.1 = *value,
            None => features.push((tag.clone(), *value)),
        }
    }
    if !features.iter().any(|(t, _)| t == "calt") {
        features.insert(0, ("calt".into(), 0));
    }
    FontFeatures(features.into())
}

impl Settings {
    /// Ghostty's defaults fill in whatever the config leaves unset.
    pub fn from_ghostty(c: &GhosttyConfig) -> Self {
        Self {
            font_family: c
                .font_family
                .clone()
                .unwrap_or_else(|| fonts::DEFAULT_FAMILY.into()),
            font_size: px(c.font_size.unwrap_or(13.0)),
            font_features: font_features(&c.font_features),
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
                cursor_blink: c.cursor_blink,
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
            features: self.font_features.clone(),
            fallbacks: Some(fonts::fallbacks()),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(c: &str) -> Vec<(String, u32)> {
        let mut config = GhosttyConfig::default();
        chda_config::parse(c, None, &mut config);
        Settings::from_ghostty(&config)
            .font()
            .features
            .tag_value_list()
            .to_vec()
    }

    #[test]
    fn ligatures_are_off_until_the_config_turns_calt_on() {
        assert_eq!(tags(""), [("calt".to_owned(), 0)]);
        assert_eq!(
            tags("font-feature = ss01\n"),
            [("calt".to_owned(), 0), ("ss01".to_owned(), 1)]
        );
        assert_eq!(
            tags("font-feature = calt\nfont-feature = liga\nfont-feature = -liga\n"),
            [("calt".to_owned(), 1), ("liga".to_owned(), 0)]
        );
    }
}
