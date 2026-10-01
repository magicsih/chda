//! Ghostty config parser (supported subset) and chda's own TOML config.
//!
//! Only the keys chda honors are kept; everything else is ignored silently,
//! as Ghostty itself does for unknown keys.

use std::fs;
use std::path::{Path, PathBuf};

/// RGB color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    /// Parse `#rrggbb` or `rrggbb`.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.trim().trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Self {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        })
    }
}

/// Cursor shape names Ghostty accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
    BlockHollow,
}

/// Padding in pixels on each side of the terminal grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Padding {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

/// The subset of Ghostty's configuration that chda reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GhosttyConfig {
    /// First `font-family` entry; later ones are fallbacks Ghostty would try.
    pub font_family: Option<String>,
    /// `font-size` in points.
    pub font_size: Option<f32>,
    pub theme: Option<String>,
    pub background: Option<Color>,
    pub foreground: Option<Color>,
    pub cursor_color: Option<Color>,
    pub cursor_style: Option<CursorStyle>,
    pub selection_background: Option<Color>,
    pub selection_foreground: Option<Color>,
    /// `palette = N=#rrggbb` entries, in file order.
    pub palette: Vec<(u8, Color)>,
    pub padding: Padding,
    /// Byte limit from `scrollback-limit`.
    pub scrollback_limit: Option<u64>,
}

impl GhosttyConfig {
    /// Apply one `key = value` line. Unknown keys are ignored.
    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "font-family" => {
                if self.font_family.is_none() && !value.is_empty() {
                    self.font_family = Some(value.to_owned());
                }
            }
            "font-size" => self.font_size = value.parse().ok(),
            "theme" => self.theme = (!value.is_empty()).then(|| value.to_owned()),
            "background" => self.background = Color::parse(value),
            "foreground" => self.foreground = Color::parse(value),
            "cursor-color" => self.cursor_color = Color::parse(value),
            "cursor-style" => {
                self.cursor_style = match value {
                    "block" => Some(CursorStyle::Block),
                    "bar" => Some(CursorStyle::Bar),
                    "underline" => Some(CursorStyle::Underline),
                    "block_hollow" => Some(CursorStyle::BlockHollow),
                    _ => None,
                }
            }
            "selection-background" => self.selection_background = Color::parse(value),
            "selection-foreground" => self.selection_foreground = Color::parse(value),
            "palette" => {
                if value.is_empty() {
                    self.palette.clear();
                } else if let Some((index, color)) = value.split_once('=')
                    && let Ok(index) = index.trim().parse::<u8>()
                    && let Some(color) = Color::parse(color)
                {
                    self.palette.push((index, color));
                }
            }
            "window-padding-x" => {
                if let Some((l, r)) = parse_pair(value) {
                    self.padding.left = l;
                    self.padding.right = r;
                }
            }
            "window-padding-y" => {
                if let Some((t, b)) = parse_pair(value) {
                    self.padding.top = t;
                    self.padding.bottom = b;
                }
            }
            "scrollback-limit" => self.scrollback_limit = value.parse().ok(),
            _ => {}
        }
    }

    /// Lay `other`'s explicitly set values over this config.
    fn overlay(&mut self, other: &GhosttyConfig) {
        macro_rules! take {
            ($($f:ident),*) => { $( if other.$f.is_some() { self.$f = other.$f.clone(); } )* };
        }
        take!(
            font_family,
            font_size,
            theme,
            background,
            foreground,
            cursor_color,
            cursor_style,
            selection_background,
            selection_foreground,
            scrollback_limit
        );
        self.palette.extend(other.palette.iter().copied());
        if other.padding != Padding::default() {
            self.padding = other.padding;
        }
    }

    /// Final palette after applying every entry in order.
    pub fn palette_colors(&self) -> Vec<(u8, Color)> {
        let mut out: Vec<(u8, Color)> = Vec::new();
        for &(i, c) in &self.palette {
            match out.iter_mut().find(|(j, _)| *j == i) {
                Some(slot) => slot.1 = c,
                None => out.push((i, c)),
            }
        }
        out
    }
}

/// `"4"` means both sides, `"2,6"` means first and second.
fn parse_pair(value: &str) -> Option<(u32, u32)> {
    match value.split_once(',') {
        Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
        None => {
            let v = value.trim().parse().ok()?;
            Some((v, v))
        }
    }
}

/// Where to look for config and theme files.
#[derive(Clone, Debug)]
pub struct Paths {
    /// Config files in load order; later files override earlier ones.
    pub config_files: Vec<PathBuf>,
    /// Theme directories in lookup order.
    pub theme_dirs: Vec<PathBuf>,
}

impl Paths {
    /// Ghostty's default locations for the current user.
    pub fn default_for_user() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        let mut config_files = Vec::new();
        let mut theme_dirs = Vec::new();
        if let Some(xdg) = &xdg {
            config_files.push(xdg.join("ghostty").join("config"));
            theme_dirs.push(xdg.join("ghostty").join("themes"));
        }
        if cfg!(target_os = "macos")
            && let Some(home) = &home
        {
            config_files
                .push(home.join("Library/Application Support/com.mitchellh.ghostty/config"));
            theme_dirs.push(PathBuf::from(
                "/Applications/Ghostty.app/Contents/Resources/ghostty/themes",
            ));
        }
        Self {
            config_files,
            theme_dirs,
        }
    }
}

/// Parse config text. `base_dir` resolves relative `config-file` includes.
pub fn parse(text: &str, base_dir: Option<&Path>, out: &mut GhosttyConfig) {
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        if key == "config-file" {
            let (optional, path) = match value.strip_prefix('?') {
                Some(p) => (true, p),
                None => (false, value),
            };
            let path = Path::new(path);
            let path = match (path.is_absolute(), base_dir) {
                (false, Some(dir)) => dir.join(path),
                _ => path.to_path_buf(),
            };
            // Missing includes are skipped; `?` only silences Ghostty's own
            // warning, which chda does not emit anyway.
            let _ = optional;
            if let Ok(text) = fs::read_to_string(&path) {
                parse(&text, path.parent(), out);
            }
            continue;
        }
        out.apply(key, value);
    }
}

/// Load the user's Ghostty config, resolving `theme` through `paths`.
///
/// Theme colors come first; anything the user set explicitly wins.
pub fn load(paths: &Paths) -> GhosttyConfig {
    let mut user = GhosttyConfig::default();
    for file in &paths.config_files {
        if let Ok(text) = fs::read_to_string(file) {
            parse(&text, file.parent(), &mut user);
        }
    }
    resolve_theme(user, &paths.theme_dirs)
}

fn resolve_theme(user: GhosttyConfig, theme_dirs: &[PathBuf]) -> GhosttyConfig {
    let Some(name) = user.theme.as_deref().map(theme_name) else {
        return user;
    };
    let Some(theme_text) = theme_dirs
        .iter()
        .map(|d| d.join(name))
        .chain(std::iter::once(PathBuf::from(name)))
        .find_map(|p| fs::read_to_string(p).ok())
    else {
        return user;
    };
    let mut merged = GhosttyConfig::default();
    parse(&theme_text, None, &mut merged);
    merged.overlay(&user);
    merged
}

/// `theme = light:X,dark:Y` picks per appearance; chda uses the dark one.
fn theme_name(value: &str) -> &str {
    let mut dark = None;
    let mut single = Some(value);
    for part in value.split(',') {
        if let Some((kind, name)) = part.split_once(':') {
            single = None;
            if kind.trim() == "dark" {
                dark = Some(name.trim());
            }
        }
    }
    dark.or(single).unwrap_or(value).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_keys_and_ignores_the_rest() {
        let mut c = GhosttyConfig::default();
        parse(
            "# comment\nfont-family = JetBrains Mono\nfont-family = Menlo\nfont-size = 14.5\n\
             background = #1e1e2e\nforeground = cdd6f4\npalette = 1=#f38ba8\npalette = 1=#ff0000\n\
             cursor-style = bar\nwindow-padding-x = 12\nwindow-padding-y = 4,8\n\
             macos-titlebar-style = tabs\nscrollback-limit = 1000\n",
            None,
            &mut c,
        );
        assert_eq!(c.font_family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(c.font_size, Some(14.5));
        assert_eq!(c.background, Color::parse("#1e1e2e"));
        assert_eq!(c.foreground, Color::parse("#cdd6f4"));
        assert_eq!(
            c.palette_colors(),
            vec![(1, Color::parse("#ff0000").unwrap())]
        );
        assert_eq!(c.cursor_style, Some(CursorStyle::Bar));
        assert_eq!(
            c.padding,
            Padding {
                left: 12,
                right: 12,
                top: 4,
                bottom: 8
            }
        );
        assert_eq!(c.scrollback_limit, Some(1000));
    }

    #[test]
    fn theme_colors_are_overridden_by_user_values() {
        let dir = std::env::temp_dir().join(format!("chda-config-{}", std::process::id()));
        let themes = dir.join("themes");
        fs::create_dir_all(&themes).unwrap();
        fs::write(
            themes.join("Mocha"),
            "background = #111111\nforeground = #222222\npalette = 0=#000000\n",
        )
        .unwrap();
        fs::write(dir.join("extra"), "font-size = 11\n").unwrap();
        fs::write(
            dir.join("config"),
            "theme = light:Latte,dark:Mocha\nforeground = #eeeeee\nconfig-file = extra\nconfig-file = ?missing\n",
        )
        .unwrap();

        let paths = Paths {
            config_files: vec![dir.join("config")],
            theme_dirs: vec![themes],
        };
        let c = load(&paths);
        assert_eq!(c.background, Color::parse("#111111"));
        assert_eq!(c.foreground, Color::parse("#eeeeee"));
        assert_eq!(c.palette_colors(), vec![(0, Color::default())]);
        assert_eq!(c.font_size, Some(11.0));
        fs::remove_dir_all(&dir).unwrap();
    }
}
