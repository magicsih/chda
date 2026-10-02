//! Themes bundled from iTerm2-Color-Schemes, the collection Ghostty ships,
//! so `theme` works without Ghostty installed.

use std::fs;

use crate::Paths;

/// `[Name]` lines, each followed by that theme's config lines. Written by
/// `scripts/update-themes.sh`.
const BUNDLED: &str = include_str!("../assets/themes.txt");

fn bundled_themes() -> impl Iterator<Item = (&'static str, &'static str)> {
    BUNDLED.split("\n[").skip(1).filter_map(|section| {
        let (header, body) = section.split_once('\n')?;
        Some((header.strip_suffix(']')?, body))
    })
}

/// Config text of the bundled theme `name`.
pub(crate) fn bundled(name: &str) -> Option<&'static str> {
    bundled_themes()
        .find(|(n, _)| *n == name)
        .map(|(_, body)| body)
}

/// Every theme `theme = ...` can name: the files in the theme directories
/// and the bundled ones, sorted case-insensitively.
pub fn theme_names(paths: &Paths) -> Vec<String> {
    let mut names: Vec<String> = bundled_themes().map(|(n, _)| n.to_owned()).collect();
    for dir in &paths.theme_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        names.extend(
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| e.file_name().into_string().ok()),
        );
    }
    names.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_themes_parse() {
        let count = bundled_themes().count();
        assert!(count > 400, "{count} themes");
        let mocha = bundled("Catppuccin Mocha").unwrap();
        assert!(mocha.contains("background = #1e1e2e"), "{mocha}");
        assert!(!mocha.contains('['));
        assert_eq!(bundled("No Such Theme"), None);
    }

    #[test]
    fn names_merge_theme_directories() {
        let dir = std::env::temp_dir().join(format!("chda-themes-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("my theme"), "background = #000000\n").unwrap();
        fs::write(dir.join("Catppuccin Mocha"), "background = #000000\n").unwrap();
        let names = theme_names(&Paths {
            config_files: vec![],
            theme_dirs: vec![dir.clone()],
        });
        assert!(names.iter().any(|n| n == "my theme"));
        assert_eq!(names.iter().filter(|n| *n == "Catppuccin Mocha").count(), 1);
        assert!(
            names
                .windows(2)
                .all(|w| w[0].to_lowercase() <= w[1].to_lowercase())
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
