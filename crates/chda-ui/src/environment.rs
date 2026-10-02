//! Where a window reads and writes its files and how it reaches the OS.
//! The app builds one from the user's real locations; tests build one over
//! temporary directories with a [`System`] that records instead of acting.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use chda_config::{ChdaConfig, Paths};
use chda_core::agents::{AgentAdapter, adapters, data_dir};

use crate::platform::{self, NotificationTarget};

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
}

pub struct Environment {
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
    /// Ask `gh` for pull request state.
    pub use_gh: bool,
    pub system: Rc<dyn System>,
}

impl Environment {
    /// The current user's locations and the real OS.
    pub fn for_user() -> Self {
        Self {
            config_path: ChdaConfig::default_path(),
            data_dir: data_dir(),
            ghostty: Paths::default_for_user(),
            shell: chda_term::login_shell(),
            pane_env: Vec::new(),
            adapters: Arc::new(adapters()),
            use_gh: true,
            system: Rc::new(NativeSystem),
        }
    }

    /// `config.toml`, or the defaults when it is missing or broken.
    pub fn load_config(&self) -> ChdaConfig {
        self.config_path
            .as_deref()
            .and_then(|p| ChdaConfig::load(p).ok())
            .unwrap_or_default()
    }
}
