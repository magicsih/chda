//! Platform-specific UI pieces: fonts, locale, alert sound, notifications and
//! the Dock badge.

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod macos;

/// `LANG` for shells when chda's own environment has no locale at all, as
/// for apps started from the Dock (launchd passes none). Without it shells
/// run in the C locale and show UTF-8 input as raw bytes. `None` when the
/// user set one, which then passes through unchanged.
pub fn default_lang() -> Option<String> {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if set("LC_ALL") || set("LC_CTYPE") || set("LANG") {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        Some(utf8_locale(&macos::locale_identifier(), |name| {
            std::path::Path::new("/usr/share/locale")
                .join(name)
                .is_dir()
        }))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// `ko_KR` (or `ko_KR@rg=krzzzz`) → `ko_KR.UTF-8` when the system has that
/// locale, else `en_US.UTF-8`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn utf8_locale(identifier: &str, exists: impl Fn(&str) -> bool) -> String {
    let base = identifier.split('@').next().unwrap_or("");
    let candidate = format!("{base}.UTF-8");
    if !base.is_empty() && exists(&candidate) {
        candidate
    } else {
        "en_US.UTF-8".to_owned()
    }
}

/// Make a font that is only used as a fallback available. GPUI loads a
/// family only when it has an `m` glyph, which icon fonts lack, so on macOS
/// the font is written to `data_dir/fonts` and registered with CoreText,
/// where fallback lists look.
pub fn register_fallback_font(file_name: &str, data: &'static [u8], cx: &gpui::App) {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        let path = chda_core::agents::data_dir()
            .ok_or_else(|| std::io::Error::other("no data directory"))
            .and_then(|dir| write_font(&dir.join("fonts"), file_name, data));
        match path {
            Ok(path) => macos::register_font_file(&path),
            Err(e) => eprintln!("chda: could not install {file_name}: {e}"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = file_name;
        if let Err(e) = cx
            .text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(data)])
        {
            eprintln!("chda: could not load a bundled font: {e}");
        }
    }
}

/// Write `data` to `dir/name` unless an identical copy is there.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn write_font(dir: &Path, name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    let path = dir.join(name);
    if std::fs::read(&path).ok().as_deref() == Some(data) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    // Write aside and rename, so a running instance never sees half a font.
    let partial = dir.join(format!("{name}.partial"));
    std::fs::write(&partial, data)?;
    std::fs::rename(&partial, &path)?;
    Ok(path)
}

/// What the system's file manager is called in menu items.
pub fn file_manager_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(target_os = "windows") {
        "File Explorer"
    } else {
        "File Manager"
    }
}

/// Play the system alert sound.
pub fn beep() {
    #[cfg(target_os = "macos")]
    macos::beep();
}

/// Where clicking a notification leads: the agent's pane, else its worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationTarget {
    pub pane: Option<u64>,
    pub worktree: PathBuf,
}

impl NotificationTarget {
    /// Encode as a notification identifier: `chda/<nonce>/<pane or ->/<path>`.
    /// The nonce keeps identifiers unique so notifications do not replace
    /// each other.
    fn encode(&self, nonce: u64) -> String {
        let pane = self.pane.map_or("-".to_owned(), |p| p.to_string());
        format!("chda/{nonce}/{pane}/{}", self.worktree.to_string_lossy())
    }

    fn decode(id: &str) -> Option<Self> {
        let mut parts = id.splitn(4, '/');
        if parts.next()? != "chda" {
            return None;
        }
        parts.next()?;
        let pane = parts.next()?;
        Some(Self {
            pane: pane.parse().ok(),
            worktree: PathBuf::from(parts.next()?),
        })
    }
}

/// Start delivering notification clicks to `on_click`. Call once, on the
/// main thread.
pub fn init_notifications(on_click: impl Fn(NotificationTarget) + 'static) {
    #[cfg(target_os = "macos")]
    macos::init_notifications(move |id| {
        if let Some(target) = NotificationTarget::decode(&id) {
            on_click(target);
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = on_click;
}

/// Show a user notification; clicking it reports `target`.
pub fn notify(title: &str, body: &str, target: &NotificationTarget) {
    #[cfg(target_os = "macos")]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let id = target.encode(NONCE.fetch_add(1, Ordering::Relaxed));
        macos::notify(title, body, &id);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (title, body, target);
}

/// Show the standard About panel: name, version and icon from the bundle.
pub fn show_about() {
    #[cfg(target_os = "macos")]
    macos::show_about();
}

/// Un-minimize the app's windows (before bringing one to the front).
pub fn restore_windows() {
    #[cfg(target_os = "macos")]
    macos::restore_windows();
}

/// Show `count` on the Dock icon; zero clears it.
pub fn set_badge(count: usize) {
    #[cfg(target_os = "macos")]
    macos::set_badge(count);
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_identifiers_become_utf8_locales() {
        let exists = |n: &str| matches!(n, "ko_KR.UTF-8" | "en_US.UTF-8");
        assert_eq!(utf8_locale("ko_KR", exists), "ko_KR.UTF-8");
        assert_eq!(utf8_locale("ko_KR@rg=krzzzz", exists), "ko_KR.UTF-8");
        assert_eq!(utf8_locale("en_KR", exists), "en_US.UTF-8");
        assert_eq!(utf8_locale("", exists), "en_US.UTF-8");
    }

    #[test]
    fn fonts_are_written_once() {
        let dir = std::env::temp_dir().join(format!("chda-font-test-{}", std::process::id()));
        let path = write_font(&dir, "a.ttf", b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_font(&dir, "a.ttf", b"one").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        write_font(&dir, "a.ttf", b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn notification_targets_round_trip() {
        let t = NotificationTarget {
            pane: Some(12),
            worktree: PathBuf::from("/src/app.worktrees/feat-x"),
        };
        assert_eq!(NotificationTarget::decode(&t.encode(3)), Some(t));
        let t = NotificationTarget {
            pane: None,
            worktree: PathBuf::from("/a/b"),
        };
        assert_eq!(NotificationTarget::decode(&t.encode(0)), Some(t));
        assert_eq!(NotificationTarget::decode("other/1/2/x"), None);
    }
}
