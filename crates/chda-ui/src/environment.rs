//! Where a window reads and writes its files and how it reaches the OS.
//! The app builds one from the user's real locations; tests build one over
//! temporary directories with a [`System`] that records instead of acting.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use chda_config::{ChdaConfig, Paths};
use chda_core::ForgeClis;
use chda_core::agents::{AgentAdapter, adapters, data_dir};

use futures::future::LocalBoxFuture;

use crate::platform::{self, FolderApp, FolderPick, NotificationTarget};

/// Side effects outside the window.
pub trait System {
    fn notify(&self, title: &str, body: &str, target: &NotificationTarget);
    /// Show `count` on the Dock icon; zero clears it.
    fn set_badge(&self, count: usize);
    fn beep(&self);
    /// Un-minimize the app's windows.
    fn restore_windows(&self);
    /// `LANG` for new shells when the app's own environment has none.
    fn pane_locale(&self) -> Option<String>;
    /// Report clicks on notifications sent through [`System::notify`].
    fn on_notification_click(&self, handler: Box<dyn Fn(NotificationTarget)>);
    /// Open a file with the system's default application.
    fn open_file(&self, path: &Path, cx: &gpui::App);
    /// Show `path` selected in its folder in the file manager.
    fn reveal_path(&self, path: &Path, cx: &gpui::App);
    /// Installed apps that can open a folder (editors, git clients, ...).
    fn folder_apps(&self) -> Vec<FolderApp>;
    /// Open `folder` in the app with `id` (a [`FolderApp::id`]).
    fn open_folder_in(&self, id: &str, folder: &Path) -> std::io::Result<()>;
    /// Ask the user for folders, with `prompt` on the confirm button.
    fn pick_folders(&self, prompt: &str, cx: &gpui::App) -> LocalBoxFuture<'static, FolderPick>;
}

/// The real OS.
pub struct NativeSystem;

impl System for NativeSystem {
    fn notify(&self, title: &str, body: &str, target: &NotificationTarget) {
        platform::notify(title, body, target);
    }

    fn set_badge(&self, count: usize) {
        platform::set_badge(count);
    }

    fn beep(&self) {
        platform::beep();
    }

    fn restore_windows(&self) {
        platform::restore_windows();
    }

    fn pane_locale(&self) -> Option<String> {
        platform::default_lang()
    }

    fn on_notification_click(&self, handler: Box<dyn Fn(NotificationTarget)>) {
        platform::init_notifications(handler);
    }

    fn open_file(&self, path: &Path, cx: &gpui::App) {
        cx.open_with_system(path);
    }

    fn reveal_path(&self, path: &Path, cx: &gpui::App) {
        cx.reveal_path(path);
    }

    fn folder_apps(&self) -> Vec<FolderApp> {
        platform::folder_apps()
    }

    fn open_folder_in(&self, id: &str, folder: &Path) -> std::io::Result<()> {
        platform::open_folder_in(id, folder)
    }

    fn pick_folders(&self, prompt: &str, cx: &gpui::App) -> LocalBoxFuture<'static, FolderPick> {
        platform::pick_folders(prompt, cx)
    }
}

pub struct Environment {
    pub collect_telemetry: bool,
    pub update_launcher: crate::upgrade::LaunchUpdate,
    pub(crate) windows: Rc<std::cell::RefCell<crate::window_registry::WindowRegistry>>,
    /// chda's `config.toml`.
    pub config_path: Option<PathBuf>,
    /// Hook socket, session index, saved session, shell integration scripts.
    pub data_dir: Option<PathBuf>,
    /// Ghostty config locations.
    pub ghostty: Paths,
    /// Shell for new panes.
    pub shell: Option<PathBuf>,
    /// Extra environment for every pane's shell.
    pub pane_env: Vec<(String, String)>,
    pub adapters: Arc<Vec<Box<dyn AgentAdapter>>>,
    /// The forge CLIs (`gh`, `glab`, `tea`) to ask for pull request state.
    pub forge_clis: ForgeClis,
    pub system: Rc<dyn System>,
    /// Asks GitHub for the latest release: the API's JSON, or `None` when
    /// it cannot be reached. Runs off the main thread.
    pub latest_release: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

/// PATH for panes and agent discovery, captured once from the login shell.
/// A failed probe preserves the inherited environment.
pub(crate) fn shell_path_env(
    shell: &Path,
    home: &Path,
    env: &[(String, String)],
) -> Vec<(String, String)> {
    let pairs: Vec<_> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    match chda_term::shell_path(shell, home, &pairs, std::time::Duration::from_secs(2)) {
        Ok(path) => vec![("PATH".into(), path)],
        Err(_) => Vec::new(),
    }
}

impl Environment {
    /// The current user's locations and the real OS.
    pub fn for_user() -> Self {
        let shell = chda_term::login_shell();
        let pane_env = shell
            .as_deref()
            .zip(std::env::var_os("HOME"))
            .map(|(shell, home)| shell_path_env(shell, Path::new(&home), &[]))
            .unwrap_or_default();
        Self {
            collect_telemetry: true,
            update_launcher: Arc::new(platform::update::start),
            windows: Default::default(),
            config_path: ChdaConfig::default_path(),
            data_dir: data_dir(),
            ghostty: Paths::default_for_user(),
            shell,
            pane_env,
            adapters: Arc::new(adapters()),
            forge_clis: ForgeClis::default(),
            system: Rc::new(NativeSystem),
            latest_release: Arc::new(fetch_latest_release),
        }
    }

    pub fn home(&self) -> PathBuf {
        self.pane_env
            .iter()
            .rev()
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_default()
    }

    /// Exactly the PATH inherited by a PTY launched with `pane_env`.
    pub fn search_path(&self) -> std::ffi::OsString {
        self.pane_env
            .iter()
            .rev()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.into())
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default()
    }

    /// `config.toml`, or the defaults when it is missing or broken.
    pub fn load_config(&self) -> ChdaConfig {
        self.config_path
            .as_deref()
            .and_then(|p| ChdaConfig::load(p).ok())
            .unwrap_or_default()
    }
}

/// One `curl` request to the GitHub API, with curl's own user agent; no
/// other data goes with it.
fn fetch_latest_release() -> Option<String> {
    let out = std::process::Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--location",
            "--max-time",
            "20",
            "--header",
            "Accept: application/vnd.github+json",
            chda_core::release::LATEST_RELEASE_URL,
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8(out.stdout).ok())
        .flatten()
}
