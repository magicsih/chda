//! App-owned window state: unique pane routing, recent navigation and one save.
use crate::workspace_view::WorkspaceView;
use chda_core::{PaneId, SavedSession, SavedWindow, agents::control::Incoming};
use gpui::{AnyWindowHandle, WeakEntity, WindowId};
use std::{collections::HashMap, path::PathBuf, sync::mpsc};

#[derive(Clone)]
pub(crate) struct WindowEntry {
    pub window: AnyWindowHandle,
    pub view: WeakEntity<WorkspaceView>,
    pub saved: SavedWindow,
    pub panes: HashMap<PaneId, (PathBuf, u64)>,
}
#[derive(Default)]
pub(crate) struct WindowRegistry {
    pub entries: Vec<WindowEntry>,
    pub active: Option<WindowId>,
    pub navigation: u64,
    pub restoring: bool,
    pub quitting: bool,
    pub wake: std::sync::Arc<std::sync::Mutex<Vec<futures::channel::mpsc::UnboundedSender<()>>>>,
    pub events: Option<mpsc::Receiver<Incoming>>,
    pub started_at: Option<u64>,
    pub quotas: std::rc::Rc<std::cell::RefCell<crate::status_bar::ProviderQuotas>>,
    pub codex_polling: bool,
    pub update: crate::upgrade::UpdateState,
    pub replay: Vec<Incoming>,
    pub ipc_server: Option<chda_core::agents::ipc::Server>,
    pub inherited_notifications:
        std::collections::VecDeque<chda_core::notifications::NotificationQueue>,
}
impl WindowRegistry {
    pub fn snapshot(&self) -> SavedSession {
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|e| !e.saved.tabs.is_empty())
            .collect();
        SavedSession {
            active_window: entries
                .iter()
                .position(|e| Some(e.window.window_id()) == self.active)
                .unwrap_or(0),
            windows: entries.into_iter().map(|e| e.saved.clone()).collect(),
        }
    }
}
