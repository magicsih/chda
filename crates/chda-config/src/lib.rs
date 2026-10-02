//! Ghostty config parser (supported subset) and chda's own TOML config.
//!
//! Only the Ghostty keys chda honors are kept; everything else is ignored
//! silently, as Ghostty itself does for unknown keys.

mod chda;
mod themes;

use std::fs;
use std::path::{Path, PathBuf};

pub use chda::*;
pub use themes::theme_names;

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
    /// `font-feature` settings as OpenType tag and value, in file order;
    /// later entries for the same tag win.
    pub font_features: Vec<(String, u32)>,
    pub theme: Option<String>,
    pub background: Option<Color>,
    pub foreground: Option<Color>,
    pub cursor_color: Option<Color>,
    pub cursor_style: Option<CursorStyle>,
    /// `cursor-style-blink`; unset lets the terminal decide.
    pub cursor_blink: Option<bool>,
    pub selection_background: Option<Color>,
    pub selection_foreground: Option<Color>,
    /// `palette = N=#rrggbb` entries, in file order.
    pub palette: Vec<(u8, Color)>,
    pub padding: Padding,
    /// `scrollback-limit` in bytes, as Ghostty defines it.
    pub scrollback_limit: Option<u64>,
    /// `shell-integration = none` turns prompt and cwd reporting off.
    pub shell_integration: Option<String>,
    /// Values of supported keys that Ghostty would reject, as
    /// `key = value` lines. Unknown keys are not problems.
    pub problems: Vec<String>,
    /// Files that were read: config files, includes and the theme.
    pub sources: Vec<PathBuf>,
}

/// A color value: `#rrggbb`, or a word, which Ghostty takes as an X11 color
/// name (chda does not resolve names and leaves the color unset).
fn color(value: &str, problems: &mut Vec<String>, key: &str) -> Option<Color> {
    let parsed = Color::parse(value);
    if parsed.is_none() && !value.chars().all(|c| c.is_ascii_alphabetic() || c == ' ') {
        problems.push(format!("{key} = {value}"));
    }
    parsed
}

impl GhosttyConfig {
    /// Apply one `key = value` line. Unknown keys are ignored; an empty
    /// value resets a key.
    fn apply(&mut self, key: &str, value: &str) {
        let problems = &mut self.problems;
        let mut check = |ok: bool| {
            if !ok && !value.is_empty() {
                problems.push(format!("{key} = {value}"));
            }
        };
        match key {
            "font-family" => {
                if self.font_family.is_none() && !value.is_empty() {
                    self.font_family = Some(value.to_owned());
                }
            }
            "font-feature" => {
                if value.is_empty() {
                    self.font_features.clear();
                }
                for setting in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let feature = parse_font_feature(setting);
                    check(feature.is_some());
                    self.font_features.extend(feature);
                }
            }
            "font-size" => {
                self.font_size = value.parse().ok().filter(|s: &f32| *s > 0.0);
                check(self.font_size.is_some());
            }
            "theme" => self.theme = (!value.is_empty()).then(|| value.to_owned()),
            "background" => self.background = color(value, &mut self.problems, key),
            "foreground" => self.foreground = color(value, &mut self.problems, key),
            "cursor-color" => self.cursor_color = color(value, &mut self.problems, key),
            "cursor-style" => {
                self.cursor_style = match value {
                    "block" => Some(CursorStyle::Block),
                    "bar" => Some(CursorStyle::Bar),
                    "underline" => Some(CursorStyle::Underline),
                    "block_hollow" => Some(CursorStyle::BlockHollow),
                    _ => None,
                };
                check(self.cursor_style.is_some());
            }
            "cursor-style-blink" => {
                self.cursor_blink = match value {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                };
                check(self.cursor_blink.is_some());
            }
            "selection-background" => {
                self.selection_background = color(value, &mut self.problems, key)
            }
            "selection-foreground" => {
                self.selection_foreground = color(value, &mut self.problems, key)
            }
            "palette" => {
                if value.is_empty() {
                    self.palette.clear();
                } else if let Some((index, color)) = value.split_once('=')
                    && let Ok(index) = index.trim().parse::<u8>()
                {
                    if let Some(color) = Color::parse(color) {
                        self.palette.push((index, color));
                    }
                } else {
                    check(false);
                }
            }
            "window-padding-x" => {
                let pair = parse_pair(value);
                check(pair.is_some());
                if let Some((l, r)) = pair {
                    self.padding.left = l;
                    self.padding.right = r;
                }
            }
            "window-padding-y" => {
                let pair = parse_pair(value);
                check(pair.is_some());
                if let Some((t, b)) = pair {
                    self.padding.top = t;
                    self.padding.bottom = b;
                }
            }
            "scrollback-limit" => {
                self.scrollback_limit = value.parse().ok();
                check(self.scrollback_limit.is_some());
            }
            "shell-integration" => self.shell_integration = Some(value.to_owned()),
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
            cursor_blink,
            selection_background,
            selection_foreground,
            scrollback_limit,
            shell_integration
        );
        self.palette.extend(other.palette.iter().copied());
        self.font_features
            .extend(other.font_features.iter().cloned());
        self.problems.extend(other.problems.iter().cloned());
        self.sources.extend(other.sources.iter().cloned());
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

/// One `font-feature` setting in Ghostty's (HarfBuzz's) syntax: `calt`,
/// `+calt` and `calt=1` enable, `-calt` and `calt=0` disable, `cv01=2` picks
/// an alternate; `calt on` and `calt off` work too.
fn parse_font_feature(setting: &str) -> Option<(String, u32)> {
    let setting = setting.trim_matches(|c| c == '"' || c == '\'');
    let (tag, value) = if let Some(tag) = setting.strip_prefix('-') {
        (tag, 0)
    } else if let Some(tag) = setting.strip_prefix('+') {
        (tag, 1)
    } else if let Some((tag, value)) = setting.split_once(['=', ' ']) {
        let value = match value.trim() {
            "on" | "true" => 1,
            "off" | "false" => 0,
            v => v.parse().ok()?,
        };
        (tag.trim(), value)
    } else {
        (setting, 1)
    };
    let tag = tag.trim_matches(|c| c == '"' || c == '\'');
    (tag.len() == 4 && tag.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| (tag.to_owned(), value))
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
    /// Ghostty's default locations for the current user, in the order
    /// Ghostty loads them: in each directory the legacy `config`, then
    /// `config.ghostty`; the XDG directory, then (macOS) Application Support.
    pub fn default_for_user() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        Self::under(xdg, home)
    }

    /// [`Paths::default_for_user`] for an XDG config directory and a home.
    pub fn under(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Self {
        let mut config_files = Vec::new();
        let mut theme_dirs = Vec::new();
        let names = |dir: PathBuf| [dir.join("config"), dir.join("config.ghostty")];
        if let Some(xdg) = &xdg {
            config_files.extend(names(xdg.join("ghostty")));
            theme_dirs.push(xdg.join("ghostty").join("themes"));
        }
        if cfg!(target_os = "macos")
            && let Some(home) = &home
        {
            config_files.extend(names(
                home.join("Library/Application Support/com.mitchellh.ghostty"),
            ));
            theme_dirs.push(PathBuf::from(
                "/Applications/Ghostty.app/Contents/Resources/ghostty/themes",
            ));
        }
        Self {
            config_files,
            theme_dirs,
        }
    }

    /// The file Ghostty itself would edit: the last existing one in load
    /// order, which also wins over the others; with none, the last listed
    /// (`config.ghostty`, in Application Support on macOS).
    pub fn preferred_config_file(&self) -> Option<&Path> {
        self.config_files
            .iter()
            .rev()
            .find(|p| p.is_file())
            .or(self.config_files.last())
            .map(PathBuf::as_path)
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
                out.sources.push(path.clone());
                parse(&text, path.parent(), out);
            }
            continue;
        }
        out.apply(key, value);
    }
}

/// Load the user's Ghostty config, resolving `theme` through `paths` and
/// the bundled themes. `theme`, chda's own setting, replaces the config's.
///
/// Theme colors come first; anything the user set explicitly wins.
pub fn load(paths: &Paths, theme: Option<&str>) -> GhosttyConfig {
    let mut user = GhosttyConfig::default();
    for file in &paths.config_files {
        if let Ok(text) = fs::read_to_string(file) {
            user.sources.push(file.clone());
            parse(&text, file.parent(), &mut user);
        }
    }
    if let Some(theme) = theme {
        user.theme = Some(theme.to_owned());
    }
    resolve_theme(user, &paths.theme_dirs)
}

/// Theme directories first, as in Ghostty, then the bundled themes, then
/// `name` as a path.
fn resolve_theme(user: GhosttyConfig, theme_dirs: &[PathBuf]) -> GhosttyConfig {
    let Some(name) = user.theme.as_deref().map(theme_name) else {
        return user;
    };
    let read = |p: PathBuf| fs::read_to_string(&p).ok().map(|t| (Some(p), t));
    let Some((theme_path, theme_text)) = theme_dirs
        .iter()
        .find_map(|d| read(d.join(name)))
        .or_else(|| themes::bundled(name).map(|t| (None, t.to_owned())))
        .or_else(|| read(PathBuf::from(name)))
    else {
        return user;
    };
    let mut merged = GhosttyConfig {
        sources: theme_path.into_iter().collect(),
        ..Default::default()
    };
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
    fn font_features_use_ghosttys_syntax() {
        let mut c = GhosttyConfig::default();
        parse(
            "font-feature = calt\nfont-feature = -liga, ss01\nfont-feature = cv01=2\n\
             font-feature = \"zero\" on\nfont-feature = dlig=off\nfont-feature = toolong\n",
            None,
            &mut c,
        );
        let tags: Vec<(&str, u32)> = c
            .font_features
            .iter()
            .map(|(t, v)| (t.as_str(), *v))
            .collect();
        assert_eq!(
            tags,
            [
                ("calt", 1),
                ("liga", 0),
                ("ss01", 1),
                ("cv01", 2),
                ("zero", 1),
                ("dlig", 0)
            ]
        );
        assert_eq!(c.problems, ["font-feature = toolong"]);

        parse("font-feature =\n", None, &mut c);
        assert!(c.font_features.is_empty());
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
            theme_dirs: vec![themes.clone()],
        };
        let c = load(&paths, None);
        assert_eq!(c.background, Color::parse("#111111"));
        assert_eq!(c.foreground, Color::parse("#eeeeee"));
        assert_eq!(c.palette_colors(), vec![(0, Color::default())]);
        assert_eq!(c.font_size, Some(11.0));
        assert_eq!(
            c.sources,
            vec![themes.join("Mocha"), dir.join("config"), dir.join("extra")]
        );
        assert!(c.problems.is_empty());

        // chda's theme replaces the config's; a bundled one has no file.
        let c = load(&paths, Some("Catppuccin Mocha"));
        assert_eq!(c.background, Color::parse("#1e1e2e"));
        assert_eq!(c.foreground, Color::parse("#eeeeee"));
        assert_eq!(c.sources, vec![dir.join("config"), dir.join("extra")]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_ghostty_loads_after_the_legacy_name_and_is_preferred() {
        let dir = std::env::temp_dir().join(format!("chda-names-{}", std::process::id()));
        let paths = Paths::under(Some(dir.join("xdg")), Some(dir.join("home")));
        let xdg = dir.join("xdg/ghostty");
        let support = dir.join("home/Library/Application Support/com.mitchellh.ghostty");
        let mut expected = vec![xdg.join("config"), xdg.join("config.ghostty")];
        if cfg!(target_os = "macos") {
            expected.extend([support.join("config"), support.join("config.ghostty")]);
        }
        assert_eq!(paths.config_files, expected);
        fs::create_dir_all(&xdg).unwrap();
        fs::create_dir_all(&support).unwrap();
        let newest = expected.last().unwrap();
        assert_eq!(paths.preferred_config_file(), Some(newest.as_path()));
        assert_eq!(
            paths.preferred_config_file(),
            Some(support.join("config.ghostty").as_path())
        );

        fs::write(xdg.join("config"), "font-size = 11\nfont-family = A\n").unwrap();
        assert_eq!(
            paths.preferred_config_file(),
            Some(xdg.join("config").as_path())
        );
        fs::write(xdg.join("config.ghostty"), "font-size = 12\n").unwrap();
        let c = load(&paths, None);
        assert_eq!(c.font_size, Some(12.0));
        assert_eq!(c.font_family.as_deref(), Some("A"));
        assert_eq!(
            paths.preferred_config_file(),
            Some(xdg.join("config.ghostty").as_path())
        );
        if cfg!(target_os = "macos") {
            fs::write(support.join("config"), "font-size = 14\n").unwrap();
            assert_eq!(load(&paths, None).font_size, Some(14.0));
            assert_eq!(
                paths.preferred_config_file(),
                Some(support.join("config").as_path())
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_values_ghostty_would_reject() {
        let mut c = GhosttyConfig::default();
        parse(
            "font-size = big\nbackground = #12\nforeground = black\ncursor-style = blob\n\
             window-padding-x = a,b\nfont-size =\nunknown-key = whatever\n",
            None,
            &mut c,
        );
        assert_eq!(
            c.problems,
            vec![
                "font-size = big",
                "background = #12",
                "cursor-style = blob",
                "window-padding-x = a,b"
            ]
        );
    }
}
