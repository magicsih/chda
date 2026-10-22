//! Platform-specific UI pieces: fonts, locale, alert sound, notifications,
//! the Dock badge and the folder picker.

use std::path::PathBuf;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::LocalBoxFuture;

mod keymap;
pub(crate) mod release;
pub(crate) mod sessions;
mod window;
#[cfg(test)]
pub(crate) use keymap::link_modifiers;
pub(crate) use keymap::{
    consumes_terminal_key, hide_key_bindings, link_modifier, shortcut, shortcut_hint,
};
pub(crate) use window::{controls as window_controls, frame as window_frame, has_application_menu};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub(crate) fn init_graphics_diagnostics() {
    #[cfg(target_os = "linux")]
    linux::init_graphics_diagnostics();
}

/// The outcome of [`pick_folders`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderPick {
    Chosen(Vec<PathBuf>),
    Cancelled,
    /// The system could not show the picker; the detail is for logs.
    Unavailable(String),
}

/// Ask the user for one or more folders. On macOS chda opens the panel
/// itself, in a later foreground task as GPUI does, so AppKit never calls
/// back while the app is borrowed. A panel the system cannot create reports
/// `Unavailable`, where GPUI's picker would abort the process.
pub fn pick_folders(prompt: &str, cx: &gpui::App) -> LocalBoxFuture<'static, FolderPick> {
    #[cfg(target_os = "macos")]
    {
        let (done, picked) = oneshot::channel();
        let prompt = prompt.to_owned();
        cx.foreground_executor()
            .spawn(async move { macos::begin_folder_panel(macos::open_panel(), &prompt, done) })
            .detach();
        async move { picked.await.unwrap_or(FolderPick::Cancelled) }.boxed_local()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some(prompt.to_owned().into()),
        });
        async move { from_path_prompt(picked.await) }.boxed_local()
    }
}

/// GPUI's path prompt result as a [`FolderPick`]: a dropped channel counts
/// as cancelled, an error (e.g. no file chooser portal on Linux) as
/// unavailable.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn from_path_prompt<E: std::fmt::Display>(
    result: Result<Result<Option<Vec<PathBuf>>, E>, oneshot::Canceled>,
) -> FolderPick {
    match result {
        Ok(Ok(Some(paths))) => FolderPick::Chosen(paths),
        Ok(Ok(None)) | Err(oneshot::Canceled) => FolderPick::Cancelled,
        Ok(Err(e)) => FolderPick::Unavailable(format!("{e:#}")),
    }
}

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
    #[cfg(target_os = "linux")]
    {
        Some("C.UTF-8".to_owned())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

/// Left padding of the window's own title bar: room for the macOS window
/// buttons, which sit over it.
pub const TITLE_BAR_INSET: f32 = if cfg!(target_os = "macos") { 78.0 } else { 8.0 };

/// Top of the macOS window buttons, and their height; the title bar's
/// content is centered on them.
pub const WINDOW_BUTTONS_TOP: f32 = 10.0;
pub const WINDOW_BUTTONS_HEIGHT: f32 = 14.0;

/// The title bar options for chda's windows. On macOS the system bar is
/// transparent and chda draws its own, with the window buttons kept;
/// elsewhere the system bar stays and chda's sits under it.
pub fn titlebar(title: &str) -> gpui::TitlebarOptions {
    gpui::TitlebarOptions {
        title: Some(title.to_owned().into()),
        appears_transparent: cfg!(target_os = "macos"),
        traffic_light_position: cfg!(target_os = "macos")
            .then(|| gpui::point(gpui::px(12.0), gpui::px(WINDOW_BUTTONS_TOP))),
    }
}

/// A GUI app that can open a folder, e.g. an editor or a git client.
#[derive(Clone, Debug, PartialEq)]
pub struct FolderApp {
    /// Stable id, saved as `open-in` in `config.toml`.
    pub id: String,
    pub name: String,
    /// The app's icon (PNG).
    pub icon: Option<std::sync::Arc<gpui::Image>>,
}

/// Apps the macOS title bar offers, in menu order: id, name, bundle id.
#[cfg(target_os = "macos")]
const FOLDER_APPS: &[(&str, &str, &str)] = &[
    ("finder", "Finder", "com.apple.finder"),
    ("vscode", "VS Code", "com.microsoft.VSCode"),
    ("cursor", "Cursor", "com.todesktop.230313mzl4w4u92"),
    ("windsurf", "Windsurf", "com.exafunction.windsurf"),
    ("zed", "Zed", "dev.zed.Zed"),
    ("xcode", "Xcode", "com.apple.dt.Xcode"),
    ("intellij", "IntelliJ IDEA", "com.jetbrains.intellij"),
    (
        "intellij-ce",
        "IntelliJ IDEA CE",
        "com.jetbrains.intellij.ce",
    ),
    (
        "android-studio",
        "Android Studio",
        "com.google.android.studio",
    ),
    ("sublime", "Sublime Text", "com.sublimetext.4"),
    ("fork", "Fork", "com.DanPristupov.Fork"),
    ("tower", "Tower", "com.fournova.Tower3"),
    (
        "github-desktop",
        "GitHub Desktop",
        "com.github.GitHubClient",
    ),
    ("terminal", "Terminal", "com.apple.Terminal"),
    ("iterm", "iTerm", "com.googlecode.iterm2"),
    ("ghostty", "Ghostty", "com.mitchellh.ghostty"),
];

/// Installed editors and file managers, with available application icons.
pub fn folder_apps() -> Vec<FolderApp> {
    #[cfg(target_os = "macos")]
    {
        FOLDER_APPS
            .iter()
            .filter_map(|(id, name, bundle)| {
                let path = macos::app_path(bundle)?;
                let icon = macos::app_icon_png(&path, 64).map(|png| {
                    std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png))
                });
                Some(FolderApp {
                    id: (*id).into(),
                    name: (*name).into(),
                    icon,
                })
            })
            .collect()
    }
    #[cfg(target_os = "linux")]
    {
        linux::folder_apps()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

/// Open `folder` in the installed app with `id`.
pub fn open_folder_in(id: &str, folder: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let Some((_, _, bundle)) = FOLDER_APPS.iter().find(|(i, _, _)| *i == id) else {
            return Err(std::io::Error::other(format!("unknown app {id}")));
        };
        let status = std::process::Command::new("open")
            .arg("-b")
            .arg(bundle)
            .arg(folder)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("open -b {bundle} failed")))
        }
    }
    #[cfg(target_os = "linux")]
    {
        linux::open_folder_in(id, folder)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (id, folder);
        Err(std::io::Error::other("not supported on this platform yet"))
    }
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

#[cfg(any(target_os = "macos", target_os = "linux", test))]
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
pub fn init_notifications(on_click: impl Fn(NotificationTarget) + 'static, cx: &gpui::App) {
    #[cfg(target_os = "macos")]
    macos::init_notifications(move |id| {
        if let Some(target) = NotificationTarget::decode(&id) {
            on_click(target);
        }
    });
    #[cfg(target_os = "linux")]
    cx.on_system_notification_response(move |response, _| {
        if let Some(target) = NotificationTarget::decode(&response.tag) {
            on_click(target);
        }
    });
    #[cfg(not(target_os = "linux"))]
    let _ = cx;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = on_click;
}

/// Show a user notification; clicking it reports `target`.
pub fn notify(title: &str, body: &str, target: &NotificationTarget, cx: &gpui::App) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let id = target.encode(NONCE.fetch_add(1, Ordering::Relaxed));
        #[cfg(target_os = "macos")]
        macos::notify(title, body, &id);
        #[cfg(target_os = "linux")]
        cx.show_system_notification(gpui::SystemNotification {
            tag: id.into(),
            title: title.to_owned().into(),
            body: body.to_owned().into(),
            actions: vec![gpui::SystemNotificationAction {
                id: "open".into(),
                label: "Open terminal".into(),
            }],
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cx;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = (title, body, target);
}

/// Show the standard About panel: name, version and icon from the bundle.
pub fn show_about(cx: &mut gpui::App) {
    #[cfg(target_os = "macos")]
    macos::show_about();
    #[cfg(target_os = "linux")]
    if let Some(window) = cx.active_window() {
        // Actions can run while this window is already borrowed by GPUI.
        // Keep its exact handle and open the prompt after that update ends.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let answer = window.prompt(
                    gpui::PromptLevel::Info,
                    &format!("chda {}", env!("CARGO_PKG_VERSION")),
                    // GPUI's 288px fallback prompt does not wrap detail text.
                    Some("Terminal with a git worktree\nsidebar and a live status board\nfor coding agents.\nMIT license.\nhttps://github.com/magicsih/chda"),
                    &["Close", "Open repository"],
                    cx,
                );
                cx.spawn(async move |cx| {
                    if answer.await == Ok(1) {
                        cx.update(|cx| cx.open_url(env!("CARGO_PKG_REPOSITORY")));
                    }
                }).detach();
            });
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cx;
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
    fn path_prompt_results_map_to_folder_picks() {
        type Answer = Result<Option<Vec<PathBuf>>, String>;
        let picked = vec![PathBuf::from("/src/a"), PathBuf::from("/src/b")];
        assert_eq!(
            from_path_prompt::<String>(Ok(Ok(Some(picked.clone())))),
            FolderPick::Chosen(picked)
        );
        assert_eq!(
            from_path_prompt::<String>(Ok(Ok(None))),
            FolderPick::Cancelled
        );
        assert_eq!(
            from_path_prompt::<String>(Ok(Err("no file chooser portal".into()))),
            FolderPick::Unavailable("no file chooser portal".into())
        );
        let (done, picked) = oneshot::channel::<Answer>();
        drop(done);
        assert_eq!(
            from_path_prompt(futures::executor::block_on(picked)),
            FolderPick::Cancelled
        );
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

    #[gpui::test]
    #[cfg(target_os = "linux")]
    fn linux_notification_clicks_preserve_the_exact_pane(cx: &mut gpui::TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        let received = Rc::new(RefCell::new(Vec::new()));
        let target = NotificationTarget {
            pane: Some(42),
            worktree: PathBuf::from("/work/a project/世界"),
        };
        cx.update(|cx| {
            cx.set_app_identity("com.magicsih.chda", "chda");
            init_notifications(
                {
                    let received = received.clone();
                    move |target| received.borrow_mut().push(target)
                },
                cx,
            );
            notify("Agent is waiting", "in main", &target, cx);
        });
        let notifications = cx.delivered_system_notifications();
        assert_eq!(notifications.len(), 1);
        let tag = notifications[0].tag.clone();
        for action_id in [None, Some("open".into())] {
            cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
                tag: tag.clone(),
                action_id,
            });
        }
        cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
            tag: "unrelated".into(),
            action_id: None,
        });
        assert_eq!(*received.borrow(), [target.clone(), target]);
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) mod resources;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) mod resources {
    #[derive(Default)]
    pub struct Sampler;
    impl Sampler {
        pub fn sample(
            &mut self,
            _: u32,
            _: u64,
            _: bool,
        ) -> Result<chda_core::resources::Resources, String> {
            Err("Session-resource sampling is not available on this platform".into())
        }
    }
}

pub(crate) mod update;
