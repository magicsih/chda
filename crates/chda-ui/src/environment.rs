//! Where a window reads and writes its files and how it reaches the OS.
//! The app builds one from the user's real locations; tests build one over
//! temporary directories with a [`System`] that records instead of acting.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use chda_config::{ChdaConfig, Paths};
use chda_core::ForgeClis;
use chda_core::agents::{AgentAdapter, adapters, data_dir};

use futures::future::LocalBoxFuture;

use crate::platform::{self, FolderApp, FolderPick, NotificationTarget};

/// Side effects outside the window.
pub trait System {
    fn can_install_updates(&self) -> bool;
    fn notify(&self, title: &str, body: &str, target: &NotificationTarget, cx: &gpui::App);
    /// Show `count` on the Dock icon; zero clears it.
    fn set_badge(&self, count: usize);
    fn beep(&self);
    /// Un-minimize the app's windows.
    fn restore_windows(&self);
    /// `LANG` for new shells when the app's own environment has none.
    fn pane_locale(&self) -> Option<String>;
    /// Report clicks on notifications sent through [`System::notify`].
    fn on_notification_click(&self, handler: Box<dyn Fn(NotificationTarget)>, cx: &gpui::App);
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
    fn can_install_updates(&self) -> bool {
        platform::release::can_install()
    }
    fn notify(&self, title: &str, body: &str, target: &NotificationTarget, cx: &gpui::App) {
        platform::notify(title, body, target, cx);
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

    fn on_notification_click(&self, handler: Box<dyn Fn(NotificationTarget)>, cx: &gpui::App) {
        platform::init_notifications(handler, cx);
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
    pub session_service: Option<chda_session::Client>,
    pub session_service_error: Option<String>,
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
    /// Extra environment for every pane's shell, before the login shell's
    /// PATH; read it through [`Environment::pane_env`].
    pub pane_env: Vec<(String, String)>,
    /// The login shell's PATH, which can arrive after launch.
    pub shell_path: ShellPath,
    pub adapters: Arc<Vec<Box<dyn AgentAdapter>>>,
    /// The forge CLIs (`gh`, `glab`, `tea`) to ask for pull request state.
    pub forge_clis: ForgeClis,
    pub system: Rc<dyn System>,
    /// Asks GitHub for the latest platform release, or `None` when
    /// it cannot be reached. Runs off the main thread.
    pub latest_release: Arc<dyn Fn() -> Option<chda_core::release::Release> + Send + Sync>,
}

/// Reads the login shell's PATH within a deadline. Runs off the main thread.
pub type CapturePath = Arc<dyn Fn(Duration) -> std::io::Result<String> + Send + Sync>;

/// Where the login shell's PATH stands.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellPathState {
    /// Not captured by design (no login shell); the inherited PATH applies.
    Inherited,
    /// The quick capture at launch timed out; a longer one is running.
    Pending,
    Ready(String),
    /// The last capture failed; the inherited PATH applies until a retry
    /// succeeds.
    Failed(String),
}

/// The login shell's PATH for panes and agent discovery. Launch waits
/// [`ShellPath::QUICK`] for it; when the shell is slower, the window keeps
/// capturing in the background (see [`watch_shell_path`]).
pub struct ShellPath {
    state: RefCell<ShellPathState>,
    capture: Option<CapturePath>,
    watching: Cell<bool>,
    waiters: RefCell<Vec<futures::channel::oneshot::Sender<()>>>,
}

impl ShellPath {
    pub const QUICK: Duration = Duration::from_secs(2);
    pub const LONG: Duration = Duration::from_secs(20);
    /// Waits between failed background captures; the last one repeats.
    pub const RETRIES: [Duration; 3] = [
        Duration::from_secs(30),
        Duration::from_secs(120),
        Duration::from_secs(600),
    ];

    /// Try `capture` once with [`ShellPath::QUICK`]; `None` keeps the
    /// inherited PATH.
    pub fn start(capture: Option<CapturePath>) -> Self {
        let mut capture = capture;
        let state = match &capture {
            None => ShellPathState::Inherited,
            Some(attempt) => match attempt(Self::QUICK) {
                Ok(path) => ShellPathState::Ready(path),
                Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                    capture = None;
                    ShellPathState::Failed(e.to_string())
                }
                Err(_) => ShellPathState::Pending,
            },
        };
        Self {
            state: RefCell::new(state),
            capture,
            watching: Cell::new(false),
            waiters: RefCell::default(),
        }
    }

    pub fn state(&self) -> ShellPathState {
        self.state.borrow().clone()
    }

    pub fn pending(&self) -> bool {
        matches!(*self.state.borrow(), ShellPathState::Pending)
    }

    pub fn path(&self) -> Option<String> {
        match &*self.state.borrow() {
            ShellPathState::Ready(path) => Some(path.clone()),
            _ => None,
        }
    }

    /// Resolves at the next change of state.
    pub fn changed(&self) -> futures::channel::oneshot::Receiver<()> {
        let (tx, rx) = futures::channel::oneshot::channel();
        let mut waiters = self.waiters.borrow_mut();
        waiters.retain(|waiter| !waiter.is_canceled());
        waiters.push(tx);
        rx
    }

    fn set(&self, state: ShellPathState) {
        *self.state.borrow_mut() = state;
        for waiter in self.waiters.borrow_mut().drain(..) {
            let _ = waiter.send(());
        }
    }
}

/// Keep capturing the login shell's PATH while it is pending or failed, and
/// redraw every window when it changes. One loop per app.
pub(crate) fn watch_shell_path(env: Rc<Environment>, cx: &mut gpui::App) {
    let path = &env.shell_path;
    let Some(capture) = path.capture.clone() else {
        return;
    };
    if path.watching.get() || !matches!(*path.state.borrow(), ShellPathState::Pending) {
        return;
    }
    path.watching.set(true);
    cx.spawn(async move |cx| {
        let mut failures = 0;
        loop {
            if env.windows.borrow().quitting {
                break;
            }
            let attempt = capture.clone();
            let result =
                gpui::AppContext::background_spawn(cx, async move { attempt(ShellPath::LONG) })
                    .await;
            let done = match result {
                Ok(path) => {
                    env.shell_path.set(ShellPathState::Ready(path));
                    true
                }
                Err(e) => {
                    env.shell_path.set(ShellPathState::Failed(e.to_string()));
                    false
                }
            };
            let entries = env.windows.borrow().entries.clone();
            cx.update(|cx| {
                for entry in entries {
                    let _ = entry.view.update(cx, |_, cx| cx.notify());
                }
            });
            if done {
                break;
            }
            let delay = ShellPath::RETRIES[failures.min(ShellPath::RETRIES.len() - 1)];
            failures += 1;
            cx.background_executor().timer(delay).await;
        }
        env.shell_path.watching.set(false);
    })
    .detach();
}

impl Environment {
    /// The current user's locations and the real OS.
    pub fn for_user() -> Self {
        let shell = chda_term::login_shell();
        let capture =
            shell
                .clone()
                .zip(std::env::var_os("HOME"))
                .map(|(shell, home)| -> CapturePath {
                    Arc::new(move |deadline| {
                        chda_term::shell_path(&shell, Path::new(&home), &[], deadline)
                    })
                });
        let service = platform::sessions::connect(data_dir().as_deref());
        let (session_service, session_service_error) = match service {
            Ok(client) => (client, None),
            Err(error) => (None, Some(error.to_string())),
        };
        if let Some(client) = &session_service
            && let Ok(first) = client.reserve_ids()
        {
            chda_core::reserve_pane_ids(first);
        }
        Self {
            session_service,
            session_service_error,
            collect_telemetry: true,
            update_launcher: Arc::new(platform::update::start),
            windows: Default::default(),
            config_path: ChdaConfig::default_path(),
            data_dir: data_dir(),
            ghostty: Paths::default_for_user(),
            shell,
            pane_env: Vec::new(),
            shell_path: ShellPath::start(capture),
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

    /// The environment every pane's shell and helper process gets: the
    /// extra variables plus the login shell's PATH once it is known.
    pub fn pane_env(&self) -> Vec<(String, String)> {
        let mut env = self.pane_env.clone();
        if let Some(path) = self.shell_path.path() {
            env.retain(|(k, _)| k != "PATH");
            env.push(("PATH".into(), path));
        }
        env
    }

    /// Exactly the PATH inherited by a PTY launched with [`Self::pane_env`].
    pub fn search_path(&self) -> std::ffi::OsString {
        self.shell_path
            .path()
            .or_else(|| {
                self.pane_env
                    .iter()
                    .rev()
                    .find(|(k, _)| k == "PATH")
                    .map(|(_, v)| v.clone())
            })
            .map(Into::into)
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
fn fetch_latest_release() -> Option<chda_core::release::Release> {
    let out = std::process::Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--location",
            "--max-time",
            "20",
            "--header",
            "Accept: application/vnd.github+json",
            platform::release::endpoint(),
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| {
            String::from_utf8(out.stdout)
                .ok()
                .as_deref()
                .and_then(platform::release::parse)
        })
        .flatten()
}
