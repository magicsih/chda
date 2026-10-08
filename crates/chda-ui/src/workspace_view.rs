//! The window's root view: sidebar, tab bar and the split tree of panes.
//! It also owns the agent hook receiver and the sidebar refresh schedule.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chda_config::{ChdaConfig, GhosttyConfig, Paths, PullStrategy, TabTitle};

use crate::environment::Environment;
use chda_core::agents::control::{Incoming, Reply, Request};
use chda_core::agents::{
    AgentAdapter, AgentId, CodexRunState, HookEvent, HookKind, SessionCache, SessionId, ipc,
    parse_codex_title,
};
use chda_core::notifications::Severity;
use chda_core::release::{Release, ReleaseCheck, parse_latest};
mod agent_launch;
mod agent_restart;
mod children;
mod notifications;
mod review;
mod sharing;
mod upgrade;
use chda_core::{
    ActiveTab, AgentEvent, AgentSessionRef, AgentStatus, Axis, Direction, FileWatcher, IdleAgent,
    Listing, Node, PaneId, RepoWatcher, SavedBounds, SavedWindow, TabId, TitleMode, Workspace,
};
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{
    AnyElement, AnyView, App, Context, Entity, FocusHandle, Focusable, Hsla, MouseButton,
    MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Point, PromptLevel, Render,
    StyleRefinement, Subscription, Window, actions, anchored, deferred, div, img, point,
    prelude::*, px, relative,
};

use crate::palette::{Palette, PaletteCommand, PaletteEvent, PaletteItem};
use crate::platform;
use crate::settings::Settings;
use crate::sidebar_view::{AgentLabel, SessionPick, SidebarEvent, SidebarView};
use crate::terminal_element::hsla;
use crate::terminal_view::{TerminalEvent, TerminalView, now_ms};
use crate::text_input::{TextInput, TextInputEvent};

actions!(
    workspace,
    [
        NewTab,
        NewWindow,
        CloseSurface,
        NextTab,
        PrevTab,
        GotoTab1,
        GotoTab2,
        GotoTab3,
        GotoTab4,
        GotoTab5,
        GotoTab6,
        GotoTab7,
        GotoTab8,
        LastTab,
        SplitRight,
        SplitDown,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        NextSplit,
        PrevSplit,
        ResizeLeft,
        ResizeRight,
        ResizeUp,
        ResizeDown,
        EqualizeSplits,
        ToggleZoom,
        ToggleSidebar,
        AddRepo,
        NewWorktree,
        TogglePalette,
        Dismiss,
        Quit,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        GoToWaitingAgent,
        SelectTheme,
        OpenConfig,
        OpenGhosttyConfig,
        ReloadConfig,
        Minimize,
        ZoomWindow,
        RunClaude,
        RunCodex,
        ResumeSession,
    ]
);

/// Fraction of the tab a keyboard resize moves the divider by.
const RESIZE_STEP: f32 = 0.05;
/// Height of the window's own title bar, bottom border included. Its
/// content is centered on the macOS window buttons.
pub(crate) const TITLE_BAR_HEIGHT: f32 = 2.0
    * (crate::platform::WINDOW_BUTTONS_TOP + crate::platform::WINDOW_BUTTONS_HEIGHT / 2.0)
    + 1.0;
/// Font size bounds and step for the runtime font size actions (Ghostty's).
const FONT_SIZE_STEP: f32 = 1.0;
const FONT_SIZE_MIN: f32 = 4.0;
const FONT_SIZE_MAX: f32 = 255.0;
/// Safety-net refresh while the window is active; the real triggers are
/// shell prompts, `.git` changes and agent events.
const STATUS_REFRESH: Duration = Duration::from_secs(60);
/// How often session transcripts are re-indexed while the window is active.
const SESSION_REFRESH: Duration = Duration::from_secs(120);
/// Sidebar width bounds while dragging its border, and the room the panes
/// keep next to it.
const SIDEBAR_MIN: f32 = 180.0;
const SIDEBAR_MAX: f32 = 600.0;
const PANES_MIN: f32 = 320.0;
/// Width of the grab area on the sidebar's border.
const SIDEBAR_GRIP: f32 = 6.0;

/// `width` kept within the bounds and leaving the panes their room in a
/// window `window_width` wide.
fn clamp_sidebar(width: f32, window_width: f32) -> f32 {
    let max = SIDEBAR_MAX.min(window_width - PANES_MIN).max(SIDEBAR_MIN);
    width.clamp(SIDEBAR_MIN, max)
}

const ICON_SIDEBAR: &str = "\u{f10aa}"; // nf-md-dock_left

/// Answers when an added folder is not a git repository.
const INIT_GIT: &str = "Initialize git";
const ADD_AS_FOLDER: &str = "Add as folder";
/// How often to see whether the daily release check is due.
const RELEASE_CHECK: Duration = Duration::from_secs(60 * 60);
/// How long terminal activity waits before it is saved for the next launch.
const ACTIVITY_SAVE_DELAY: Duration = Duration::from_secs(5);

/// A popup menu anchored at a window position.
pub(crate) struct ContextMenu {
    position: Point<Pixels>,
    pub(crate) items: Vec<(String, MenuAction)>,
}

/// Stable identities for the two distinct close operations.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CloseTarget {
    Tab(TabId),
    Pane(PaneId),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CloseFocus {
    tab: TabId,
    pane: Option<PaneId>,
}

#[derive(Clone, Debug)]
pub(crate) enum MenuAction {
    CopySelection(PaneId),
    ShareSelection {
        pane: PaneId,
        identity: Option<(String, String)>,
    },
    StopAndClose {
        target: CloseTarget,
        focus: CloseFocus,
    },
    OpenTerminal(PathBuf),
    /// A read-only tab with the worktree's diff against its base.
    ViewDiff(PathBuf),
    ReviewDiff(PathBuf),
    ViewGitTree(PathBuf),
    RunAgent(PathBuf, AgentId),
    /// Run the `agent-presets` entry with this name.
    RunPreset(PathBuf, String),
    NewWorktree(PathBuf),
    /// New worktree on a new branch starting at `base`'s HEAD.
    NewWorktreeFrom {
        repo: PathBuf,
        base: String,
    },
    /// Edit the branch's note (its git branch description).
    EditNote {
        repo: PathBuf,
        branch: String,
    },
    /// Add the branch to the sidebar's STARRED list, or take it out.
    Star {
        repo: PathBuf,
        branch: String,
        starred: bool,
    },
    /// Merge into the default branch, remove the worktree and branch.
    MergeAndClean {
        repo: PathBuf,
        worktree: PathBuf,
        force: bool,
    },
    /// Remove every merged, clean worktree of the repository.
    CleanStale(PathBuf),
    /// Remove one worktree and its branch; `force` discards changes.
    DeleteWorktreeAndBranch {
        repo: PathBuf,
        worktree: PathBuf,
        force: bool,
    },
    /// Drop git's record of a worktree whose folder is gone; keep its branch.
    PruneMissing {
        repo: PathBuf,
        worktree: PathBuf,
    },
    /// Pick an existing branch to check out as a new worktree.
    PickBranch(PathBuf),
    RemoveRepo(PathBuf),
    RefreshRepo(PathBuf),
    /// Bring the worktree's upstream in; `None`: the configured strategy.
    UpdateBranch {
        repo: PathBuf,
        worktree: PathBuf,
        strategy: Option<PullStrategy>,
    },
    /// A menu item that cannot be used now, with the reason.
    Unavailable(String),
    /// Open a file or folder with the editor or the default application.
    OpenPath {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
    /// Show a file or folder selected in the file manager.
    RevealPath(PathBuf),
    /// Render a Markdown file and open it in the browser.
    PreviewMarkdown(PathBuf),
    /// Make this app (a `FolderApp` id) the title bar's choice.
    PickFolderApp(String),
    /// Hide the new-version notice until the next release.
    DismissUpdate,
    /// `git init` in a plain folder.
    InitGit(PathBuf),
    /// Open with the system's default application (a folder: the file manager).
    OpenWithSystem(PathBuf),
    /// Type `cd <dir>` at the shell prompt of `pane`.
    CdHere {
        pane: PaneId,
        dir: PathBuf,
    },
    OpenUrl(String),
    Copy(String),
}

/// A yes/no sheet before a destructive action.
pub(crate) struct ConfirmSheet {
    title: String,
    pub(crate) lines: Vec<String>,
    action: MenuAction,
}

/// The "new worktree" sheet.
struct NewWorktreeSheet {
    repo: PathBuf,
    /// Branch the new one starts from; `None`: the main worktree's HEAD.
    base: Option<String>,
    /// Random name used when the field is left empty.
    suggestion: String,
    input: Entity<TextInput>,
    /// Optional note: what the branch is for.
    note: Entity<TextInput>,
    busy: bool,
    _subs: [Subscription; 2],
    error: Option<String>,
}

/// The "edit note" sheet for a branch.
struct NoteSheet {
    repo: PathBuf,
    branch: String,
    input: Entity<TextInput>,
    _sub: Subscription,
    error: Option<String>,
}

pub struct WorkspaceView {
    pub(crate) env: std::rc::Rc<Environment>,
    /// Settings new panes start with; `font_size` follows the font size
    /// actions, while `configured_font_size` is what `cmd-0` returns to.
    pub(crate) settings: Settings,
    configured_font_size: Pixels,
    pub(crate) config: ChdaConfig,
    pub(crate) ws: Workspace,
    pub(crate) panes: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    pub(crate) graphs: HashMap<TabId, Entity<crate::git_graph::GitGraphView>>,
    pub(crate) reviews: HashMap<TabId, (Entity<crate::diff_review::DiffReviewView>, Subscription)>,
    previous_runs: HashMap<PaneId, Entity<crate::readonly_text::ReadOnlyText>>,
    previous_open: HashSet<PaneId>,
    owned_outputs: std::cell::RefCell<HashMap<PaneId, String>>,
    focus_handle: FocusHandle,
    pub(crate) sidebar: Entity<SidebarView>,
    _sidebar_sub: Subscription,
    pub(crate) sidebar_visible: bool,
    /// The sidebar's border is being dragged.
    sidebar_drag: bool,
    adapters: Arc<Vec<Box<dyn AgentAdapter>>>,
    session_cache: Arc<Mutex<SessionCache>>,
    refreshing: HashSet<PathBuf>,
    refresh_again: HashSet<PathBuf>,
    watcher: Option<RepoWatcher>,
    watch_events: Option<mpsc::Receiver<PathBuf>>,
    pub(crate) context_menu: Option<ContextMenu>,
    /// Installed apps the title bar can open the worktree in.
    pub(crate) folder_apps: Vec<crate::platform::FolderApp>,
    /// A newer release than the running one, until dismissed.
    pub(crate) release_notice: Option<Release>,
    /// The left button went down on the title bar: the next move drags
    /// the window.
    title_drag: bool,
    sheet: Option<NewWorktreeSheet>,
    launch_sheet: Option<agent_launch::LaunchSheet>,
    note_sheet: Option<NoteSheet>,
    pub(crate) share_sheet: Option<sharing::ShareSheet>,
    share_generation: u64,
    pub(crate) confirm: Option<ConfirmSheet>,
    pub(crate) palette: Option<(Entity<Palette>, Subscription)>,
    /// Inline editor for a tab title.
    renaming: Option<(TabId, Entity<TextInput>, Subscription)>,
    /// Repository whose tabs the tab bar shows (`None`: tabs outside repos).
    tab_group: Option<PathBuf>,
    /// The forge CLIs, once one of them is known to be installed.
    forges: Option<Arc<chda_core::Forges>>,
    /// Last time pull requests were fetched per repository.
    pr_fetched: HashMap<PathBuf, std::time::Instant>,
    /// Last `origin/<default>` fetch per repository.
    base_fetched: HashMap<PathBuf, std::time::Instant>,
    pub(crate) notifications: chda_core::notifications::NotificationQueue,
    pub(crate) notifications_open: bool,
    notification_focus: Option<gpui::WeakFocusHandle>,
    pub(crate) child_detail: Option<chda_core::agents::ChildActivity>,
    child_focus: Option<gpui::WeakFocusHandle>,
    /// The title last given to the OS window.
    window_title: String,
    /// A session save for new terminal activity is scheduled.
    activity_save_pending: bool,
    /// Where the Ghostty config lives, and the files the last load read.
    ghostty_paths: Paths,
    ghostty_sources: Vec<PathBuf>,
    /// Watches the Ghostty config files and `config.toml`.
    config_watcher: Option<FileWatcher>,
    /// The last reload found a broken config; repeated polls do not add events.
    config_problem: Option<String>,
    /// The theme palette shows a theme that is not the configured one yet.
    previewing_theme: bool,
    /// Frame of the "working" spinner, and whether its timer runs.
    spin: u32,
    pub(crate) spinning: bool,
    /// The pane "go to waiting agent" jumped to last, to cycle onwards.
    last_jump: Option<PaneId>,
    /// Window frame, kept for session restore.
    bounds: Option<SavedBounds>,
    /// The app is quitting: panes going away must not shrink the saved
    /// session.
    quitting: bool,
    self_weak: gpui::WeakEntity<Self>,
    pub(crate) status_bar: crate::status_bar::StatusBar,
    pane_navigation: HashMap<PaneId, u64>,
    initializing_panes: HashSet<PaneId>,
}

impl WorkspaceView {
    pub fn new(
        ghostty: GhosttyConfig,
        saved: Option<SavedWindow>,
        env: std::rc::Rc<Environment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Settings::from_ghostty(&ghostty);
        let config = env.load_config();
        let bg = hsla(settings.colors.background.unwrap_or_default());
        let fg = hsla(settings.colors.foreground.unwrap_or_default());
        let agents = env
            .adapters
            .iter()
            .enumerate()
            .map(|(i, a)| AgentLabel::new(i, a.id().as_str(), a.short_label(), a.display_name()))
            .collect();
        let sidebar = cx.new(|cx| SidebarView::new(fg, blend(bg, fg, 0.04), agents, cx));
        let sidebar_sub = cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| {
            this.on_sidebar_event(event.clone(), window, cx)
        });
        let adapters = Arc::clone(&env.adapters);
        let quotas = env.windows.borrow().quotas.clone();
        let notifications = env
            .windows
            .borrow_mut()
            .inherited_notifications
            .pop_front()
            .unwrap_or_default();
        let child_collapsed = env
            .windows
            .borrow_mut()
            .inherited_child_collapsed
            .pop_front()
            .unwrap_or_default();
        sidebar.update(cx, |s, _| {
            s.child_collapsed = child_collapsed.into_iter().collect()
        });
        let window_handle = window.window_handle();
        env.windows
            .borrow_mut()
            .entries
            .push(crate::window_registry::WindowEntry {
                window: window_handle,
                view: cx.entity().downgrade(),
                saved: SavedWindow::default(),
                panes: HashMap::new(),
                sessions: HashMap::new(),
            });
        Self::start_hook_receiver(env.clone(), window, cx);
        if env.windows.borrow().entries.len() == 1 {
            let registry = env.windows.clone();
            let directory = env.data_dir.clone();
            let restore = config.restore_session;
            cx.on_window_closed(move |cx, _| {
                let mut registry = registry.borrow_mut();
                if registry.quitting {
                    return;
                }
                let windows = cx.windows();
                registry.entries.retain(|e| windows.contains(&e.window));
                if restore && let Some(dir) = directory.as_deref() {
                    let _ = registry.snapshot().save(dir);
                }
            })
            .detach();
        }
        let (watcher, watch_events) = Self::start_watcher(window, cx);

        let mut ws = Workspace::new();
        ws.title_mode = match config.tab_title {
            TabTitle::Branch => TitleMode::Branch,
            TabTitle::Path => TitleMode::Path,
        };
        let mut this = Self {
            sidebar_visible: config.sidebar_visible,
            sidebar_drag: false,
            configured_font_size: settings.font_size,
            settings,
            config,
            ws,
            panes: HashMap::new(),
            graphs: HashMap::new(),
            reviews: HashMap::new(),
            previous_runs: HashMap::new(),
            previous_open: HashSet::new(),
            owned_outputs: Default::default(),
            focus_handle: cx.focus_handle(),
            sidebar,
            _sidebar_sub: sidebar_sub,
            adapters,
            session_cache: Arc::new(Mutex::new(
                env.data_dir
                    .as_deref()
                    .map(SessionCache::load)
                    .unwrap_or_default(),
            )),
            refreshing: HashSet::new(),
            refresh_again: HashSet::new(),
            watcher,
            watch_events,
            context_menu: None,
            folder_apps: Vec::new(),
            release_notice: None,
            title_drag: false,
            sheet: None,
            launch_sheet: None,
            note_sheet: None,
            share_sheet: None,
            share_generation: 0,
            confirm: None,
            palette: None,
            renaming: None,
            tab_group: None,
            forges: None,
            pr_fetched: HashMap::new(),
            base_fetched: HashMap::new(),
            notifications,
            notifications_open: false,
            child_detail: None,
            child_focus: None,
            notification_focus: None,
            window_title: String::new(),
            activity_save_pending: false,
            ghostty_paths: env.ghostty.clone(),
            ghostty_sources: ghostty.sources,
            config_watcher: None,
            config_problem: None,
            previewing_theme: false,
            spin: 0,
            spinning: false,
            last_jump: None,
            bounds: None,
            quitting: false,
            self_weak: cx.entity().downgrade(),
            status_bar: crate::status_bar::StatusBar {
                quotas,
                ..Default::default()
            },
            pane_navigation: HashMap::new(),
            initializing_panes: HashSet::new(),
            env,
        };
        let registry = this.env.windows.clone();
        window.on_window_should_close(cx, move |_, _| !registry.borrow().update.frozen);
        this.note_bounds(window);
        cx.observe_window_bounds(window, |this, window, _| {
            this.note_bounds(window);
            this.save_session();
        })
        .detach();
        cx.on_app_quit(|this, _| {
            this.save_session();
            this.quitting = true;
            this.env.windows.borrow_mut().quitting = true;
            async {}
        })
        .detach();
        this.start_notification_clicks(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.focus_active(window, cx);
            }
        })
        .detach();
        this.config_watcher = Self::start_config_watcher(window, cx);
        this.watch_config_files();
        let clis = this.env.forge_clis.clone();
        let forges = cx.background_spawn(async move {
            let forges = chda_core::Forges::new(clis);
            forges.any_installed().then(|| Arc::new(forges))
        });
        cx.spawn(async move |this, cx| {
            let forges = forges.await;
            let _ = this.update(cx, |view, cx| {
                if forges.is_some() {
                    view.forges = forges;
                    view.refresh_prs(false, cx);
                }
            });
        })
        .detach();
        for repo in this.config.repos.clone() {
            this.sidebar.update(cx, |s, _| {
                s.model.add_repo(repo.clone());
            });
            this.watch_repo(&repo);
        }
        this.sync_starred(cx);
        this.install_hooks();
        match saved {
            Some(saved) => this.restore_session(&saved, window, cx),
            None => this.new_tab(&NewTab, window, cx),
        }
        this.refresh_all(cx);
        this.refresh_sessions(cx);
        Self::schedule_refreshes(window, cx);
        Self::schedule_activity_ages(cx);
        if this.env.collect_telemetry {
            Self::schedule_telemetry(cx);
            this.start_codex_quota(cx);
        }
        Self::schedule_release_checks(window, cx);
        Self::watch_update(window, cx);
        // Looking up apps and drawing their icons takes tens of milliseconds
        // on a cold start; do it after the first frame.
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |view, cx| {
                view.folder_apps = view.env.system.folder_apps();
                cx.notify();
            });
        })
        .detach();
        this
    }

    fn hook_bin() -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("chda"))
    }

    fn install_hooks(&self) {
        let Some(data_dir) = self.env.data_dir.as_deref() else {
            return;
        };
        for adapter in self.adapters.iter() {
            let _ = adapter.install_hooks(data_dir, &Self::hook_bin());
        }
    }

    /// Listen for `chda hook` on the local socket and drain the fallback log.
    fn start_hook_receiver(
        env: std::rc::Rc<Environment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Pane IDs belong to this process. A fallback hook log from an earlier
        // launch must not make a newly restored pane appear to have a live agent.
        env.windows
            .borrow_mut()
            .started_at
            .get_or_insert_with(now_ms);
        let (wake_tx, mut wake_rx) = unbounded::<()>();
        let wake = env.windows.borrow().wake.clone();
        wake.lock().unwrap().push(wake_tx.clone());
        if env.windows.borrow().events.is_none()
            && let Some(dir) = env.data_dir.as_ref()
        {
            let (tx, rx) = mpsc::channel();
            let journal = env.windows.borrow().update.journal.clone();
            if let Ok(server) = ipc::serve_journaled(
                &ipc::socket_path(dir),
                tx.clone(),
                move || {
                    wake.lock()
                        .unwrap()
                        .retain(|sender| sender.unbounded_send(()).is_ok());
                },
                journal.as_deref(),
            ) {
                env.windows.borrow_mut().ipc_server = Some(server);
                if !env.windows.borrow().update.adopting
                    && let Ok(text) = ipc::take_fallback(dir)
                {
                    for line in text.lines() {
                        if let Ok(quota) =
                            serde_json::from_str::<chda_core::agents::quota::QuotaSnapshot>(line)
                        {
                            let _ = tx.send(Incoming::Quota(quota));
                        } else if let Ok(event) = serde_json::from_str::<HookEvent>(line) {
                            let _ = tx.send(Incoming::Event(event));
                        }
                    }
                }
                env.windows.borrow_mut().events = Some(rx);
            }
        }
        let _ = wake_tx.unbounded_send(());
        cx.spawn_in(window, async move |this, cx| {
            while wake_rx.next().await.is_some() {
                if this
                    .update_in(cx, |view, window, cx| view.drain_hook_events(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Watch `.git` directories so ref and worktree changes refresh at once.
    fn start_watcher(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<RepoWatcher>, Option<mpsc::Receiver<PathBuf>>) {
        let (tx, rx) = mpsc::channel();
        let (wake_tx, mut wake_rx) = unbounded::<()>();
        let watcher = RepoWatcher::new(move |repo| {
            let _ = tx.send(repo);
            let _ = wake_tx.unbounded_send(());
        })
        .ok();
        if watcher.is_none() {
            return (None, None);
        }
        cx.spawn_in(window, async move |this, cx| {
            while wake_rx.next().await.is_some() {
                let alive = this.update(cx, |view, cx| {
                    let repos: Vec<PathBuf> = view
                        .watch_events
                        .as_ref()
                        .map(|rx| rx.try_iter().collect())
                        .unwrap_or_default();
                    for repo in repos {
                        view.refresh_repo(repo, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        (watcher, Some(rx))
    }

    /// Reload the configuration whenever one of its files changes.
    fn start_config_watcher(window: &mut Window, cx: &mut Context<Self>) -> Option<FileWatcher> {
        let (wake_tx, mut wake_rx) = unbounded::<()>();
        let watcher = FileWatcher::new(move || {
            let _ = wake_tx.unbounded_send(());
        })
        .ok()?;
        cx.spawn_in(window, async move |this, cx| {
            while wake_rx.next().await.is_some() {
                if this
                    .update_in(cx, |view, window, cx| view.reload_config(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Some(watcher)
    }

    fn watch_config_files(&mut self) {
        let files: Vec<PathBuf> = self
            .ghostty_paths
            .config_files
            .iter()
            .chain(&self.ghostty_sources)
            .cloned()
            .chain(self.env.config_path.clone())
            .collect();
        if let Some(w) = &mut self.config_watcher {
            w.set_files(files);
        }
    }

    /// Re-read the Ghostty config and `config.toml` and apply what changed.
    /// A config with errors keeps the previous values and says why.
    fn reload_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut problems = Vec::new();
        if let Some(path) = self.env.config_path.clone() {
            match ChdaConfig::load(&path) {
                Ok(config) => {
                    if config != self.config {
                        self.apply_config(config, window, cx);
                    }
                }
                Err(e) => problems.push(format!(
                    "{}: {e}. Kept the previous values.",
                    path.display()
                )),
            }
        }
        let ghostty = chda_config::load(&self.ghostty_paths, self.config.theme.as_deref());
        if ghostty.problems.is_empty() {
            let mut settings = Settings::from_ghostty(&ghostty);
            // Keep a size picked with cmd-= / cmd-- relative to the config.
            let offset = f32::from(self.settings.font_size) - f32::from(self.configured_font_size);
            self.configured_font_size = settings.font_size;
            settings.font_size =
                px((f32::from(settings.font_size) + offset).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX));
            if settings != self.settings {
                self.apply_settings(settings, cx);
            }
        } else {
            problems.push(format!(
                "Ghostty config: invalid {}. Kept the previous settings.",
                ghostty.problems.join(", ")
            ));
        }
        self.ghostty_sources = ghostty.sources;
        self.watch_config_files();
        let problem = (!problems.is_empty()).then(|| problems.join(" "));
        if problem != self.config_problem {
            if let Some(message) = &problem {
                self.notify_error(message.clone());
            } else if self.config_problem.is_some() {
                self.notify("Configuration reloaded successfully".into());
            }
        }
        self.config_problem = problem;
        cx.notify();
    }

    fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        let bg = hsla(settings.colors.background.unwrap_or_default());
        let fg = hsla(settings.colors.foreground.unwrap_or_default());
        self.sidebar
            .update(cx, |s, cx| s.set_colors(fg, blend(bg, fg, 0.04), cx));
        for (view, _) in self.panes.values() {
            let settings = settings.clone();
            view.update(cx, |view, cx| view.apply_settings(settings, cx));
        }
        for graph in self.graphs.values() {
            graph.update(cx, |graph, cx| {
                graph.background = bg;
                graph.foreground = fg;
                cx.notify();
            });
        }
        for (review, _) in self.reviews.values() {
            review.update(cx, |v, cx| {
                v.background = bg;
                v.foreground = fg;
                v.font_family = settings.font_family.clone();
                v.font_size = settings.font_size;
                cx.notify();
            });
        }
        for previous in self.previous_runs.values() {
            previous.update(cx, |view, cx| {
                view.settings = settings.clone();
                cx.notify();
            });
        }
        self.settings = settings;
    }

    /// Apply a changed `config.toml`: repositories, sidebar, tab titles and
    /// everything read on use (agents, notifications, editor, templates).
    fn apply_config(&mut self, config: ChdaConfig, window: &mut Window, cx: &mut Context<Self>) {
        self.ws.title_mode = match config.tab_title {
            TabTitle::Branch => TitleMode::Branch,
            TabTitle::Path => TitleMode::Path,
        };
        let added: Vec<PathBuf> = config
            .repos
            .iter()
            .filter(|r| !self.config.repos.contains(r))
            .cloned()
            .collect();
        let removed: Vec<PathBuf> = self
            .config
            .repos
            .iter()
            .filter(|r| !config.repos.contains(r))
            .cloned()
            .collect();
        for repo in &removed {
            if let Some(w) = &mut self.watcher {
                w.unwatch(repo);
            }
            self.sidebar.update(cx, |s, _| {
                s.model.remove_repo(repo);
            });
        }
        let sidebar_was_visible = self.sidebar_visible;
        self.sidebar_visible = config.sidebar_visible;
        self.config = config;
        for repo in added {
            self.sidebar.update(cx, |s, _| {
                s.model.add_repo(repo.clone());
            });
            self.watch_repo(&repo);
            self.refresh_repo(repo, cx);
        }
        let order = self.config.repos.clone();
        self.sidebar.update(cx, |s, _| s.model.order_repos(&order));
        self.sync_starred(cx);
        if self.sidebar_visible && !sidebar_was_visible {
            self.refresh_all(cx);
        }
        self.sync_panes(cx);
        self.sync_title(window, cx);
        self.sidebar.update(cx, |_, cx| cx.notify());
    }

    /// Show the config's STARRED list in the sidebar.
    fn sync_starred(&mut self, cx: &mut Context<Self>) {
        let starred = self.config.starred.clone();
        self.sidebar.update(cx, |s, cx| {
            if s.model.starred != starred {
                s.model.starred = starred;
                cx.notify();
            }
        });
    }

    /// Add a branch to STARRED or take it out, and save the config.
    fn set_starred(&mut self, repo: &Path, branch: &str, starred: bool, cx: &mut Context<Self>) {
        let changed = if starred {
            self.config.star(repo, branch)
        } else {
            self.config.unstar(repo, branch)
        };
        if changed {
            self.save_config();
            self.sync_starred(cx);
        }
    }

    fn watch_repo(&mut self, repo: &Path) {
        if let Some(w) = &mut self.watcher {
            let _ = w.watch(repo);
        }
    }

    fn drain_hook_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.env.windows.borrow().update.frozen {
            return;
        }
        let mut incoming = {
            let mut registry = self.env.windows.borrow_mut();
            let mut items = std::mem::take(&mut registry.replay);
            if let Some(rx) = &registry.events {
                items.extend(rx.try_iter());
            }
            items
        };
        incoming.sort_by_key(|item| match item {
            Incoming::Event(e) => e.timestamp,
            Incoming::Quota(q) => q.observed_at,
            Incoming::Request(..) => u64::MAX,
        });
        for item in incoming {
            let raw = match &item {
                Incoming::Quota(q) => q.pane,
                Incoming::Event(e) => e.pane,
                Incoming::Request(..) => None,
            };
            let target = {
                let registry = self.env.windows.borrow();
                registry
                    .entries
                    .iter()
                    .find(|entry| match &item {
                        Incoming::Event(event) if event.child.is_some() => {
                            entry.sessions.values().any(|(agent, session, _)| {
                                agent == &event.agent && session == &event.session_id
                            })
                        }
                        _ => match raw {
                            Some(raw) => entry.panes.keys().any(|p| p.raw() == raw),
                            None => Some(entry.window.window_id()) == registry.active,
                        },
                    })
                    .cloned()
            };
            if let Some(target) = target
                && target.window != window.window_handle()
            {
                let _ = target.window.update(cx, |_, window, cx| {
                    let _ = target
                        .view
                        .update(cx, |view, cx| view.receive_hook(item, window, cx));
                });
            } else {
                self.receive_hook(item, window, cx);
            }
        }
    }

    fn receive_hook(&mut self, item: Incoming, window: &mut Window, cx: &mut Context<Self>) {
        let observed_at = match &item {
            Incoming::Event(event) => Some(event.timestamp),
            Incoming::Quota(report) => Some(report.observed_at),
            Incoming::Request(..) => None,
        };
        if let (Some(observed_at), Some(started_at)) =
            (observed_at, self.env.windows.borrow().started_at)
            && observed_at < started_at
        {
            return;
        }
        match item {
            Incoming::Quota(report) => {
                if let Some(raw) = report.pane
                    && let Some(pane) = self.panes.keys().copied().find(|p| p.raw() == raw)
                {
                    if let Some(session) = report.session.clone() {
                        self.capture_launch_session(pane, &report.provider, &session);
                        self.ws.set_agent_session(
                            pane,
                            Some(AgentSessionRef {
                                agent: report.provider.clone(),
                                session,
                            }),
                        );
                        self.initializing_panes.remove(&pane);
                    }
                    self.status_bar.quotas.borrow_mut().report(report);
                    self.save_session();
                    cx.notify();
                    let entries = self.env.windows.borrow().entries.clone();
                    for entry in entries {
                        if entry.window != window.window_handle() {
                            let _ = entry.view.update(cx, |_, cx| cx.notify());
                        }
                    }
                }
            }
            Incoming::Event(ev) => self.apply_hook_event(ev, window, cx),
            Incoming::Request(request, reply) => {
                let _ = reply.send(self.handle_request(request, window, cx));
            }
        }
    }

    /// Do what `chda mcp` asked for on behalf of an agent.
    fn handle_request(
        &mut self,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Reply {
        if self.env.windows.borrow().update.frozen {
            return Reply::err("chda is reconnecting; retry shortly");
        }
        match request {
            Request::CreateWorktree {
                cwd,
                branch,
                base,
                note,
                open,
                agent,
            } => {
                let agent = match self.requested_agent(agent.as_deref()) {
                    Ok(agent) => agent,
                    Err(reply) => return reply,
                };
                let Some(repo) = self.repo_for_request(&cwd, cx) else {
                    return Reply::err(format!("{} is not in a git repository", cwd.display()));
                };
                let branch = branch.unwrap_or_else(|| {
                    chda_core::new_branch_name(&repo, |b| self.config.worktree_path(&repo, b))
                });
                let path = self.config.worktree_path(&repo, &branch);
                if let Err(e) = chda_core::create_worktree(&repo, &branch, &path, base.as_deref()) {
                    return Reply::err(format!("Could not create {branch}: {e}"));
                }
                let mut message = format!("Created branch {branch} in {}", path.display());
                if let Some(note) = note
                    && let Err(e) = chda_core::set_note(&repo, &branch, &note)
                {
                    message.push_str(&format!("; saving its note failed: {e}"));
                }
                self.refresh_repo(repo.clone(), cx);
                if open {
                    match agent {
                        Some(agent) => self.run_agent(&path, agent, None, window, cx),
                        None => self.open_default_action(&path, window, cx),
                    }
                    message.push_str(if self.launch_sheet.is_some() {
                        "; confirm the agent's launch options in chda"
                    } else {
                        " and opened a tab there"
                    });
                }
                Reply::ok(
                    message,
                    serde_json::json!({ "path": path, "branch": branch, "repo": repo }),
                )
            }
            Request::OpenTab { path, agent } => {
                if !path.is_dir() {
                    return Reply::err(format!("{} is not a directory", path.display()));
                }
                let agent = match self.requested_agent(agent.as_deref()) {
                    Ok(agent) => agent,
                    Err(reply) => return reply,
                };
                match agent {
                    Some(agent) => self.run_agent(&path, agent, None, window, cx),
                    None => self.open_tab_at(Some(path.clone()), None, window, cx),
                }
                Reply::ok(
                    if self.launch_sheet.is_some() {
                        "Confirm the agent's launch options in chda".into()
                    } else {
                        format!("Opened a tab in {}", path.display())
                    },
                    serde_json::json!({ "path": path }),
                )
            }
            Request::ListWorktrees { cwd } => {
                let model = &self.sidebar.read(cx).model;
                let repo = model.worktree_for_path(&cwd).map(|(r, _)| r.path.clone());
                let worktrees: Vec<serde_json::Value> = model
                    .repos
                    .iter()
                    .filter(|r| repo.as_ref().is_none_or(|p| *p == r.path))
                    .flat_map(|r| r.worktrees.iter().map(move |w| (r, w)))
                    .map(|(r, w)| worktree_json(r, w))
                    .collect();
                Reply::ok(
                    format!("{} worktrees", worktrees.len()),
                    serde_json::json!({ "worktrees": worktrees }),
                )
            }
        }
    }

    /// The agent a request names, checked to be known and installed;
    /// `Ok(None)` when it names none.
    fn requested_agent(&self, agent: Option<&str>) -> Result<Option<AgentId>, Reply> {
        let Some(id) = agent else {
            return Ok(None);
        };
        let Some(adapter) =
            AgentId::parse(id).and_then(|agent| self.adapters.iter().find(|a| a.id() == agent))
        else {
            return Err(Reply::err(format!("unknown agent {id}")));
        };
        if !adapter
            .executable(&self.env.home(), &self.env.search_path())
            .is_some()
        {
            return Err(Reply::err(format!(
                "{} is not on PATH",
                adapter.display_name()
            )));
        }
        Ok(Some(adapter.id()))
    }

    /// The repository a request's directory belongs to, added to the sidebar
    /// when it is not there yet.
    fn repo_for_request(&mut self, cwd: &Path, cx: &mut Context<Self>) -> Option<PathBuf> {
        if let Some((repo, _)) = self.sidebar.read(cx).model.worktree_for_path(cwd) {
            return Some(repo.path.clone());
        }
        let repo = chda_core::repo_of(cwd).ok()?;
        self.add_to_sidebar(repo.clone(), cx);
        Some(repo)
    }

    fn apply_hook_event(&mut self, ev: HookEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(child) = ev.child.clone() {
            self.apply_child_activity(
                chda_core::agents::ChildActivity {
                    agent: ev.agent,
                    parent: ev.session_id,
                    child,
                    observed_at: ev.timestamp,
                },
                cx,
            );
            return;
        }
        if let Some(run) = ev
            .pane
            .and_then(|raw| self.ws.pane_by_raw(raw))
            .and_then(|pane| self.ws.pane(pane))
            .and_then(|info| info.managed_run.as_ref())
            && (run.stopped() || run.started_at().is_some_and(|at| ev.timestamp < at))
        {
            return;
        }
        // An old pane ID must never fall back to a replacement shell in its worktree.
        if ev
            .pane
            .is_some_and(|raw| self.ws.pane_by_raw(raw).is_none())
        {
            return;
        }
        if ev.kind == HookKind::SessionEnd
            && let Some(info) = ev
                .pane
                .and_then(|raw| self.ws.pane_by_raw(raw))
                .and_then(|p| self.ws.pane(p))
            && (info.agent.as_ref().is_some_and(|a| a.agent != ev.agent)
                || info
                    .agent_session
                    .as_ref()
                    .is_some_and(|s| !ev.session_id.is_empty() && s.session != ev.session_id))
        {
            return;
        }
        if matches!(ev.kind, HookKind::SessionStart | HookKind::PromptSubmitted)
            && let Some(pane) = ev.pane.and_then(|raw| self.ws.pane_by_raw(raw))
        {
            self.initializing_panes.remove(&pane);
        }
        if ev.kind != HookKind::SessionEnd
            && let Some(pane) = ev.pane.and_then(|raw| self.ws.pane_by_raw(raw))
        {
            self.capture_launch_session(pane, &ev.agent, &ev.session_id);
        }
        // SessionStart also fires on compaction/resume within a running
        // conversation. It must not turn an existing busy session idle.
        let continuing_session = ev.kind == HookKind::SessionStart
            && ev
                .pane
                .and_then(|raw| self.ws.pane_by_raw(raw))
                .and_then(|p| self.ws.pane(p))
                .is_some_and(|info| {
                    info.agent_live
                        && info.agent.as_ref().is_some_and(|a| a.agent == ev.agent)
                        && info
                            .agent_session
                            .as_ref()
                            .is_some_and(|s| s.agent == ev.agent && s.session == ev.session_id)
                });
        if ev.kind == HookKind::SessionStart
            && let Some(pane) = ev.pane.and_then(|raw| self.ws.pane_by_raw(raw))
            && self
                .ws
                .pane(pane)
                .and_then(|i| i.agent_session.as_ref())
                .is_some_and(|s| s.agent != ev.agent || s.session != ev.session_id)
        {
            self.ws.end_agent(pane);
        }
        if ev.agent == "codex"
            && ev.kind == HookKind::Stopped
            && !ev.session_id.is_empty()
            && let Some(p) = ev.pane.and_then(|raw| self.ws.pane_by_raw(raw))
            && self
                .ws
                .pane(p)
                .is_some_and(|i| parse_codex_title(&i.title) == Some(CodexRunState::Ready))
        {
            if self.ws.set_agent_session(
                p,
                Some(AgentSessionRef {
                    agent: ev.agent,
                    session: ev.session_id,
                }),
            ) {
                self.save_session();
            }
            return;
        }
        let event = match ev.kind {
            HookKind::Idle => AgentEvent::Idle,
            HookKind::SessionStart => AgentEvent::SessionStart,
            HookKind::PromptSubmitted => AgentEvent::PromptSubmitted,
            HookKind::WaitingInput => AgentEvent::WaitingInput,
            HookKind::Stopped => AgentEvent::Stopped,
            HookKind::SessionEnd => AgentEvent::SessionEnd,
        };
        let status = match event {
            AgentEvent::SessionStart | AgentEvent::Idle => AgentStatus::Idle,
            AgentEvent::PromptSubmitted => AgentStatus::Working,
            AgentEvent::WaitingInput => AgentStatus::WaitingInput,
            AgentEvent::Stopped => AgentStatus::Review,
            AgentEvent::SessionEnd => AgentStatus::Idle,
        };
        let focused_cwd = self.focused_cwd();
        let window_active = window.is_window_active();
        let worktree_changed = self.sidebar.update(cx, |s, cx| {
            let r = s
                .model
                .apply_agent_event(&ev.agent, &ev.cwd, event, ev.timestamp);
            cx.notify();
            r
        });
        let pane = self.pane_for_event(&ev, cx);
        // Only a pane the hook named itself is sure to hold the conversation.
        if let Some(p) = ev.pane.and_then(|raw| self.ws.pane_by_raw(raw))
            && (!ev.session_id.is_empty() || event == AgentEvent::SessionEnd)
        {
            let conversation = (event != AgentEvent::SessionEnd).then(|| AgentSessionRef {
                agent: ev.agent.clone(),
                session: ev.session_id.clone(),
            });
            if self.ws.set_agent_session(p, conversation) {
                self.save_session();
            }
        }
        let pane_changed = pane.is_some_and(|p| {
            if continuing_session {
                return false;
            }
            if event == AgentEvent::SessionEnd {
                self.ws.end_agent(p)
            } else {
                let prior = self.ws.pane(p).and_then(|i| i.agent.as_ref());
                // A delayed idle notification must not dismiss unread output or
                // reset its known completion time.
                if event == AgentEvent::Idle
                    && prior.is_some_and(|a| {
                        a.agent == ev.agent
                            && matches!(a.status, AgentStatus::Review | AgentStatus::Idle)
                    })
                {
                    let info = self.ws.pane_mut(p).unwrap();
                    let changed = ev.pane.is_some() && !info.agent_live;
                    info.agent_live |= ev.pane.is_some();
                    return changed;
                }
                let mut changed = self.ws.set_agent_status(
                    p,
                    &ev.agent,
                    status,
                    if status == AgentStatus::Idle {
                        0
                    } else {
                        ev.timestamp
                    },
                );
                if let Some(info) = self.ws.pane_mut(p) {
                    changed |= info.agent_live != ev.pane.is_some();
                    info.agent_live = ev.pane.is_some();
                }
                changed
            }
        });
        if worktree_changed.is_none() && !pane_changed {
            return;
        }
        let worktree = worktree_changed
            .as_ref()
            .map(|(w, _)| w.clone())
            .unwrap_or_else(|| ev.cwd.clone());
        let looking = window_active
            && match pane {
                Some(p) => self.ws.focused_pane() == Some(p),
                None => focused_cwd
                    .as_ref()
                    .is_some_and(|c| c.starts_with(&worktree)),
            };
        let name = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(&worktree)
            .and_then(|(_, w)| w.branch.clone())
            .or_else(|| {
                worktree
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let agent = self.agent_name(&ev.agent);
        let target = platform::NotificationTarget {
            pane: pane.map(PaneId::raw),
            worktree: worktree.clone(),
        };
        if matches!(status, AgentStatus::WaitingInput | AgentStatus::Review) {
            let (severity, message) = match status {
                AgentStatus::WaitingInput => {
                    (Severity::Warning, format!("{agent} is waiting in {name}"))
                }
                _ => (Severity::Success, format!("{agent} finished in {name}")),
            };
            let mut context = self.notification_pane_context(pane);
            context.repository = Some(worktree.clone());
            let key = format!(
                "agent:{}:{}:{:?}:{}:{status:?}",
                ev.agent, ev.session_id, ev.pane, ev.timestamp
            );
            self.notifications
                .record_once(key, ev.timestamp, severity, message, context);
        }
        match status {
            AgentStatus::WaitingInput if self.config.notifications => {
                self.env.system.notify(
                    &format!("{agent} is waiting"),
                    &format!("in {name}"),
                    &target,
                );
            }
            AgentStatus::Review if self.config.notifications && !looking => {
                self.env.system.notify(
                    &format!("{agent} finished"),
                    &format!("in {name}"),
                    &target,
                );
            }
            _ => {}
        }
        if looking {
            self.reviewed_focused(cx);
        }
        if ev.pane.is_some() {
            self.sync_worktree_agent_status(&ev.agent, &ev.cwd, cx);
        }
        if worktree_changed.is_some()
            && matches!(event, AgentEvent::Stopped | AgentEvent::SessionEnd)
        {
            self.refresh_repo_of(&worktree, cx);
            self.refresh_sessions(cx);
        }
        self.sync_attention(cx);
    }

    /// Same-agent panes in one branch must not clear each other's dot.
    fn sync_worktree_agent_status(&mut self, agent: &str, cwd: &Path, cx: &mut Context<Self>) {
        let model = &self.sidebar.read(cx).model;
        let Some((_, worktree)) = model.worktree_for_path(cwd) else {
            return;
        };
        let root = worktree.path.clone();
        let status = self
            .ws
            .tabs()
            .iter()
            .flat_map(|t| t.panes())
            .filter_map(|p| self.ws.pane(p))
            .filter(|p| {
                p.cwd
                    .as_ref()
                    .and_then(|cwd| model.worktree_for_path(cwd))
                    .is_some_and(|(_, w)| w.path == root)
            })
            .filter_map(|p| p.agent.as_ref().filter(|a| a.agent == agent))
            .map(|a| a.status)
            .max_by_key(|s| s.urgency())
            .unwrap_or_default();
        self.sidebar.update(cx, |s, cx| {
            if let Some(w) = s.model.worktree_mut(&root) {
                w.agents.insert(agent.into(), status);
            }
            cx.notify();
        });
    }

    /// The pane an agent event belongs to: the one its hook named through
    /// `CHDA_PANE_ID`, else (agents started outside a chda shell) the most
    /// recently active pane in the event's worktree.
    fn pane_for_event(&self, ev: &HookEvent, cx: &App) -> Option<PaneId> {
        if let Some(p) = ev.pane.and_then(|raw| self.ws.pane_by_raw(raw)) {
            return Some(p);
        }
        let root = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(&ev.cwd)
            .map(|(_, w)| w.path.clone())
            .unwrap_or_else(|| ev.cwd.clone());
        self.ws
            .tabs()
            .iter()
            .flat_map(|t| t.panes())
            .filter_map(|p| Some((p, self.ws.pane(p)?)))
            .filter(|(_, i)| i.cwd.as_ref().is_some_and(|c| c.starts_with(&root)))
            .max_by_key(|(_, i)| i.last_activity)
            .map(|(p, _)| p)
    }

    /// The adapters of the agents `config.toml` lists, in its order.
    fn configured_adapters(&self) -> impl Iterator<Item = &dyn AgentAdapter> {
        self.config.agents.iter().filter_map(|id| {
            self.adapters
                .iter()
                .find(|a| a.id().as_str() == id)
                .map(|a| a.as_ref())
        })
    }

    /// Display name of an agent id, e.g. "Claude Code" for `claude`.
    fn agent_name(&self, id: &str) -> String {
        self.adapters
            .iter()
            .find(|a| Some(a.id()) == AgentId::parse(id))
            .map(|a| a.display_name().to_owned())
            .unwrap_or_else(|| id.to_owned())
    }

    /// After agent status or focus changes: refresh the activity list, the
    /// Dock badge and the spinner timer.
    fn sync_attention(&mut self, cx: &mut Context<Self>) {
        self.sync_panes(cx);
        self.env.system.set_badge(self.ws.unseen_waiting());
        self.ensure_spin(cx);
        cx.notify();
    }

    fn any_working(&self) -> bool {
        self.ws
            .tabs()
            .iter()
            .flat_map(|t| t.panes())
            .filter_map(|p| self.ws.pane(p)?.agent.as_ref())
            .any(|a| a.status == AgentStatus::Working)
    }

    /// Run the spinner timer while some agent works; it stops by itself.
    /// The tab bar and the sidebar redraw on each frame, nothing else does.
    fn ensure_spin(&mut self, cx: &mut Context<Self>) {
        if self.spinning || cx.reduce_motion() || !self.any_working() {
            return;
        }
        self.spinning = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(crate::status_icon::SPIN_STEP)
                    .await;
                let keep = this.update(cx, |view, cx| {
                    let working = view.any_working() && !cx.reduce_motion();
                    view.spin = if working {
                        view.spin.wrapping_add(1)
                    } else {
                        0
                    };
                    let (spin, visible) = (view.spin, view.sidebar_visible);
                    view.sidebar.update(cx, |s, cx| {
                        s.spin = spin;
                        if visible {
                            cx.notify();
                        }
                    });
                    cx.notify();
                    view.spinning = working;
                    working
                });
                if !keep.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Cycle through panes whose agent waits for input, then those with a
    /// finished turn, newest first; scroll each to its last prompt.
    fn go_to_waiting_agent(
        &mut self,
        _: &GoToWaitingAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panes = self.ws.attention_panes();
        if panes.is_empty() {
            self.notify("No agent is waiting".into());
            cx.notify();
            return;
        }
        let next = match self
            .last_jump
            .and_then(|last| panes.iter().position(|p| *p == last))
        {
            Some(i) => panes[(i + 1) % panes.len()],
            None => panes[0],
        };
        self.last_jump = Some(next);
        self.jump_to_pane(next, window, cx);
    }

    /// Focus a pane (activating its tab) and scroll it to its last prompt.
    fn jump_to_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ws.focus_pane(pane) {
            return;
        }
        if let Some((view, _)) = self.panes.get(&pane) {
            view.read(cx).jump_to_last_prompt();
        }
        self.focus_active(window, cx);
    }

    /// The sidebar's status dot for a worktree: jump to the pane that needs
    /// attention there, else any of its panes, else open one.
    fn jump_to_worktree_agent(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let panes: Vec<PaneId> = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(path)
            .map(|(_, w)| w.panes.clone())
            .unwrap_or_default();
        let pane = self
            .ws
            .attention_panes()
            .into_iter()
            .find(|p| panes.contains(p))
            .or_else(|| panes.first().copied());
        match pane {
            Some(p) => self.jump_to_pane(p, window, cx),
            None => self.open_worktree(path, window, cx),
        }
    }

    /// Bring the window forward when the user clicks one of our
    /// notifications, and show the agent's pane, or its worktree when the
    /// pane is gone.
    fn start_notification_clicks(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, mut rx) = unbounded::<platform::NotificationTarget>();
        self.env
            .system
            .on_notification_click(Box::new(move |target| {
                let _ = tx.unbounded_send(target);
            }));
        cx.spawn_in(window, async move |this, cx| {
            while let Some(target) = rx.next().await {
                if this
                    .update_in(cx, |view, window, cx| {
                        view.on_notification_click(target, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn on_notification_click(
        &mut self,
        target: platform::NotificationTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.activate(true);
        self.env.system.restore_windows();
        window.activate_window();
        match target.pane.and_then(|raw| self.ws.pane_by_raw(raw)) {
            Some(pane) => self.jump_to_pane(pane, window, cx),
            None => {
                if !self.sidebar_visible {
                    self.toggle_sidebar(&ToggleSidebar, window, cx);
                }
                self.sidebar
                    .update(cx, |s, cx| s.select(&target.worktree, cx));
            }
        }
    }

    fn focused_cwd(&self) -> Option<PathBuf> {
        self.ws
            .focused_pane()
            .and_then(|p| self.ws.pane(p))
            .and_then(|i| i.cwd.clone())
    }

    /// Repaint ages without syncing panes, writing sessions or polling git.
    fn schedule_telemetry(cx: &mut Context<Self>) {
        let samplers: Arc<Mutex<HashMap<PaneId, platform::resources::Sampler>>> =
            Arc::new(Mutex::new(HashMap::new()));
        cx.spawn(async move |this, cx| {
            let mut tick = 0u64;
            loop {
                let selection = this.update(cx, |view, cx| {
                    let pane = view.ws.focused_pane();
                    let pid = pane.and_then(|p| view.panes.get(&p)?.0.read(cx).child_pid());
                    (
                        pane,
                        pid,
                        view.panes.keys().copied().collect::<HashSet<_>>(),
                    )
                });
                let Ok((pane, pid, live_panes)) = selection else {
                    break;
                };
                let samplers = samplers.clone();
                let task = cx.background_spawn(async move {
                    let mut samplers = samplers
                        .lock()
                        .map_err(|_| "Resource sampler is unavailable".to_owned())?;
                    samplers.retain(|pane, _| live_panes.contains(pane));
                    let pane = pane.ok_or_else(|| "No active terminal pane".to_owned())?;
                    let pid = pid.ok_or_else(|| "No active PTY process".to_owned())?;
                    // Keep each pane's original process identity across focus
                    // changes. Closed panes release their sampler above.
                    samplers
                        .entry(pane)
                        .or_default()
                        .sample(pid, now_ms(), tick.is_multiple_of(3))
                });
                let result = task.await;
                if this
                    .update(cx, |view, cx| {
                        if pane != view.ws.focused_pane() {
                            return;
                        }
                        view.status_bar.resource_pane = pane;
                        match result {
                            Ok(resources) => {
                                view.status_bar.resources = Some(resources);
                                view.status_bar.resource_error = None;
                            }
                            Err(e) => {
                                view.status_bar.resources = None;
                                view.status_bar.resource_error = Some(e);
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                tick += 1;
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();
    }

    pub(crate) fn start_codex_quota(&self, cx: &mut Context<Self>) {
        let env = self.env.clone();
        {
            let mut registry = env.windows.borrow_mut();
            if registry.codex_polling {
                return;
            }
            registry.codex_polling = true;
        }
        // The task owns app state, not the first window. Closing that window
        // must not stop refreshes in the remaining windows.
        cx.spawn(async move |_, cx| {
            let mut failures = 0usize;
            loop {
                if env.windows.borrow().quitting || env.windows.borrow().entries.is_empty() {
                    break;
                }
                let executable = chda_core::agents::which("codex", &env.search_path());
                let task = cx.background_spawn(async move {
                    executable
                        .ok_or(chda_core::agents::quota::CodexQuotaError::NotInstalled)
                        .and_then(|path| chda_core::agents::quota::read_codex(&path, now_ms()))
                });
                let result = task.await;
                let delay = match &result {
                    Ok(_) => {
                        failures = 0;
                        Duration::from_secs(60)
                    }
                    Err(error) => {
                        failures = if matches!(
                            error,
                            chda_core::agents::quota::CodexQuotaError::Retry { .. }
                        ) {
                            failures.saturating_add(1)
                        } else {
                            0
                        };
                        error.retry_after(failures)
                    }
                };
                env.windows
                    .borrow()
                    .quotas
                    .borrow_mut()
                    .codex_result(result);
                let entries = env.windows.borrow().entries.clone();
                cx.update(|cx| {
                    for entry in entries {
                        let _ = entry.view.update(cx, |_, cx| cx.notify());
                    }
                });
                cx.background_executor().timer(delay).await;
            }
            env.windows.borrow_mut().codex_polling = false;
        })
        .detach();
    }

    fn schedule_activity_ages(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update(cx, |view, cx| {
                        if view.env.collect_telemetry {
                            cx.notify();
                        }
                        if view.sidebar_visible {
                            view.sidebar.update(cx, |s, cx| {
                                if !s.model.active_tabs.is_empty() {
                                    cx.notify();
                                }
                            });
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn schedule_refreshes(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let mut ticks: u64 = 0;
            loop {
                cx.background_executor().timer(STATUS_REFRESH).await;
                ticks += 1;
                let alive = this.update(cx, |view, cx| {
                    view.poll_children(cx);
                    if view.sidebar_visible {
                        view.refresh_all(cx);
                        if ticks
                            .is_multiple_of(SESSION_REFRESH.as_secs() / STATUS_REFRESH.as_secs())
                        {
                            view.refresh_sessions(cx);
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Look for a newer release now and every hour; GitHub is asked at
    /// most once a day (`release-check.json` remembers when), and never
    /// with `update-check = false`.
    fn schedule_release_checks(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let Ok(Some((file, fetch))) = this.update(cx, |view, cx| {
                    let file = view
                        .release_check_file()
                        .filter(|_| view.config.update_check);
                    let Some(file) = file else {
                        view.release_notice = None;
                        return None;
                    };
                    let check = ReleaseCheck::load(&file);
                    view.set_release_notice(&check, cx);
                    check
                        .due(now_ms())
                        .then(|| (file, view.env.latest_release.clone()))
                }) else {
                    if this.upgrade().is_none() {
                        break;
                    }
                    cx.background_executor().timer(RELEASE_CHECK).await;
                    continue;
                };
                let latest = cx
                    .background_spawn(async move { fetch().as_deref().and_then(parse_latest) })
                    .await;
                if let Some(latest) = latest {
                    let mut check = ReleaseCheck::load(&file);
                    check.record(now_ms(), latest);
                    let _ = check.save(&file);
                    let alive = this.update(cx, |view, cx| view.set_release_notice(&check, cx));
                    if alive.is_err() {
                        break;
                    }
                }
                cx.background_executor().timer(RELEASE_CHECK).await;
            }
        })
        .detach();
    }

    fn release_check_file(&self) -> Option<PathBuf> {
        self.env
            .data_dir
            .as_ref()
            .map(|d| d.join("release-check.json"))
    }

    fn set_release_notice(&mut self, check: &ReleaseCheck, cx: &mut Context<Self>) {
        let notice = check.notice(env!("CARGO_PKG_VERSION")).cloned();
        if notice != self.release_notice {
            self.release_notice = notice;
            cx.notify();
        }
    }

    fn refresh_all(&mut self, cx: &mut Context<Self>) {
        let repos: Vec<PathBuf> = self
            .sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .map(|r| r.path.clone())
            .collect();
        for repo in repos {
            self.refresh_repo(repo, cx);
        }
    }

    fn refresh_repo_of(&mut self, worktree: &Path, cx: &mut Context<Self>) {
        let repo = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(worktree)
            .map(|(r, _)| r.path.clone());
        if let Some(repo) = repo {
            self.refresh_repo(repo, cx);
        }
    }

    /// One refresh per repository at a time; a request during a refresh
    /// queues exactly one more.
    pub(crate) fn refresh_repo(&mut self, repo: PathBuf, cx: &mut Context<Self>) {
        if self.refreshing.contains(&repo) {
            self.refresh_again.insert(repo);
            return;
        }
        self.refreshing.insert(repo.clone());
        // Fetch the default branch at most once a minute so merges made
        // elsewhere (and squash merges) show up without a manual pull.
        let now = std::time::Instant::now();
        let fetch = self
            .base_fetched
            .get(&repo)
            .is_none_or(|t| now.duration_since(*t) > Duration::from_secs(60));
        if fetch {
            self.base_fetched.insert(repo.clone(), now);
        }
        let task = cx.background_spawn({
            let repo = repo.clone();
            async move { chda_core::list_registered(&repo, fetch) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.refreshing.remove(&repo);
                let before = view.worktree_paths(cx);
                if matches!(result, Ok(Listing::Repository(_))) {
                    // A plain folder that became a repository has nothing
                    // watched yet; watching twice is a no-op.
                    view.watch_repo(&repo);
                }
                view.sidebar.update(cx, |s, cx| {
                    match result {
                        Ok(Listing::Repository(worktrees)) => {
                            s.model.set_worktrees(&repo, worktrees)
                        }
                        Ok(Listing::Folder) => s.model.set_folder(&repo),
                        Err(e) => {
                            if let Some(r) = s.model.repo_mut(&repo) {
                                r.error = Some(e.to_string());
                            }
                        }
                    }
                    cx.notify();
                });
                view.sync_panes(cx);
                // New worktrees can have sessions the last index skipped.
                if view.worktree_paths(cx) != before {
                    view.refresh_sessions(cx);
                }
                if view.refresh_again.remove(&repo) {
                    view.refresh_repo(repo, cx);
                } else {
                    view.refresh_prs(false, cx);
                }
            });
        })
        .detach();
    }

    /// Fetch pull request state for every non-main worktree through the
    /// forge CLI of the repository's host, at most once a minute per
    /// repository unless `force`. A host no CLI is logged in to gets a hint
    /// on the repository row instead of badges.
    fn refresh_prs(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(forges) = self.forges.clone() else {
            return;
        };
        let now = std::time::Instant::now();
        let repos: Vec<(PathBuf, Vec<(PathBuf, String)>)> = self
            .sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .filter(|r| {
                force
                    || self
                        .pr_fetched
                        .get(&r.path)
                        .is_none_or(|t| now.duration_since(*t) > Duration::from_secs(60))
            })
            .map(|r| {
                (
                    r.path.clone(),
                    r.worktrees
                        .iter()
                        .filter(|w| !w.is_main)
                        .filter_map(|w| w.branch.clone().map(|b| (w.path.clone(), b)))
                        .collect(),
                )
            })
            .collect();
        for (repo, branches) in repos {
            if branches.is_empty() {
                continue;
            }
            self.pr_fetched.insert(repo.clone(), now);
            let host_override = self.config.repo_hosts.get(&repo).cloned();
            let forges = Arc::clone(&forges);
            let task = cx.background_spawn({
                let repo = repo.clone();
                async move {
                    let remote = chda_core::repo_remote(&repo, host_override.as_deref())?;
                    Some(forges.pull_requests(&remote, &branches))
                }
            });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |view, cx| {
                    view.sidebar.update(cx, |s, cx| {
                        let mut changed = false;
                        let hint = match result {
                            Some(Ok(results)) => {
                                for (path, pr) in results {
                                    changed |= s.model.set_pr(&path, pr);
                                }
                                None
                            }
                            Some(Err(hint)) => hint,
                            None => None,
                        };
                        if let Some(r) = s.model.repo_mut(&repo)
                            && r.pr_hint != hint
                        {
                            r.pr_hint = hint;
                            changed = true;
                        }
                        if changed {
                            cx.notify();
                        }
                    });
                });
            })
            .detach();
        }
    }

    /// Re-index agent sessions for the registered repositories' worktrees.
    fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        let adapters = Arc::clone(&self.adapters);
        let cache = Arc::clone(&self.session_cache);
        let worktrees = self.worktree_paths(cx);
        let data_dir = self.env.data_dir.clone();
        let task = cx.background_spawn(async move {
            chda_core::index_sessions(&adapters, &cache, &worktrees, data_dir.as_deref())
        });
        cx.spawn(async move |this, cx| {
            let sessions = task.await;
            let _ = this.update(cx, |view, cx| {
                view.sidebar.update(cx, |s, cx| {
                    s.model.set_sessions(&sessions);
                    s.sessions_loaded = true;
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// Every registered repository and its worktrees.
    fn worktree_paths(&self, cx: &App) -> Vec<PathBuf> {
        let model = &self.sidebar.read(cx).model;
        let mut paths: Vec<PathBuf> = model
            .repos
            .iter()
            .flat_map(|r| {
                std::iter::once(r.path.clone()).chain(r.worktrees.iter().map(|w| w.path.clone()))
            })
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// Tell the sidebar which panes live in which worktree, and remember each
    /// pane's branch and repository for tab titles and grouping.
    /// Rebuild the saved window's tabs with a shell in each saved directory,
    /// or the agent conversation a pane had open.
    fn restore_session(
        &mut self,
        saved: &SavedWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (panes, report) = self.ws.restore(saved);
        let mut notes: Vec<String> = report
            .missing
            .iter()
            .map(|(dir, to)| match to {
                Some(repo) => format!("{} is missing (opened {})", dir.display(), repo.display()),
                None => format!("{} is missing (opened the home directory)", dir.display()),
            })
            .collect();
        for p in panes {
            if let Some(file) = self
                .ws
                .pane(p.pane)
                .and_then(|info| info.previous_run.clone())
            {
                self.owned_outputs.borrow_mut().insert(p.pane, file);
            }
            let adopting = self.env.windows.borrow().update.adopting;
            if p.run.as_ref().is_some_and(chda_core::ManagedRun::stopped) {
                self.ws.end_agent(p.pane);
                continue;
            }
            if !adopting && p.launch.is_some() && !self.config.restore_agents {
                if let Some(info) = self.ws.pane_mut(p.pane) {
                    info.agent_launch = None;
                    info.managed_run = None;
                }
            } else if !adopting && let Some(context) = &p.launch {
                let result = self.captured_resume_argv(context);
                match result {
                    Ok(command) => {
                        self.initializing_panes.insert(p.pane);
                        self.ws.pane_mut(p.pane).unwrap().managed_run =
                            Some(chda_core::ManagedRun::Starting {
                                started_at: now_ms(),
                            });
                        self.open_pane(
                            p.pane,
                            Some(context.cwd.clone()),
                            Some(command),
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        let info = self.ws.pane_mut(p.pane).unwrap();
                        info.cwd = Some(context.cwd.clone());
                        info.managed_run = Some(chda_core::ManagedRun::failed(error));
                        self.ws.end_agent(p.pane);
                    }
                }
                continue;
            }
            self.initializing_panes.insert(p.pane);
            let command = if adopting {
                None
            } else {
                match &p.agent {
                    Some(conversation) if self.config.restore_agents => {
                        let cwd = p.cwd.clone().unwrap_or_default();
                        let argv = self.resume_argv(&cwd, conversation);
                        match argv {
                            Ok(argv) => Some(argv),
                            Err(why) => {
                                notes.push(why);
                                None
                            }
                        }
                    }
                    _ => None,
                }
            };
            if adopting {
                if let Some((meta, _)) = self
                    .env
                    .windows
                    .borrow()
                    .update
                    .inherited
                    .get(&p.pane.raw())
                    && let Some(info) = self.ws.pane_mut(p.pane)
                {
                    info.title = meta.title.clone();
                    info.agent = meta.agent.clone();
                    info.agent_live = meta.agent_live;
                }
                self.initializing_panes.remove(&p.pane);
            } else if command.is_none() {
                self.ws.set_agent_session(p.pane, None);
            }
            self.open_pane(p.pane, p.cwd, command, window, cx);
        }
        let graphs: Vec<_> = self
            .ws
            .tabs()
            .iter()
            .filter_map(|tab| tab.graph_repo().map(|repo| (tab.id, repo.to_path_buf())))
            .collect();
        for (id, repo) in graphs {
            self.create_graph_view(id, repo, cx);
        }
        let reviews: Vec<_> = self
            .ws
            .tabs()
            .iter()
            .filter_map(|tab| match &tab.content {
                chda_core::TabContent::DiffReview { worktree, base, .. } => {
                    Some((tab.id, worktree.clone(), base.clone()))
                }
                _ => None,
            })
            .collect();
        for (id, worktree, base) in reviews {
            self.create_review_view(id, worktree, base, cx);
        }
        if self.ws.is_empty() {
            self.new_tab(&NewTab, window, cx);
            return;
        }
        if !notes.is_empty() {
            self.notify(format!("Restored the last session; {}", notes.join("; ")));
        }
        self.focus_active(window, cx);
    }

    /// The command that reopens a saved agent conversation in `cwd`, or why
    /// it cannot be reopened.
    fn resume_argv(
        &self,
        cwd: &Path,
        conversation: &AgentSessionRef,
    ) -> Result<Vec<String>, String> {
        let adapter = AgentId::parse(&conversation.agent)
            .and_then(|id| self.adapters.iter().find(|a| a.id() == id));
        let Some(adapter) = adapter else {
            return Err(format!(
                "unknown agent {}, opened a shell",
                conversation.agent
            ));
        };
        let name = adapter.display_name();
        if !adapter
            .executable(&self.env.home(), &self.env.search_path())
            .is_some()
        {
            return Err(format!(
                "{name} is not on PATH, opened a shell in {}",
                cwd.display()
            ));
        }
        let session = SessionId(conversation.session.clone());
        if !adapter.has_session(&session) {
            return Err(format!(
                "{name} session {} is gone, opened a shell in {}",
                session.0,
                cwd.display()
            ));
        }
        self.agent_argv(cwd, adapter.id(), Some(&session))
            .ok_or_else(|| {
                format!(
                    "could not start {name}, opened a shell in {}",
                    cwd.display()
                )
            })
    }

    /// The window's origin and content size: a window opens with the size
    /// of its content, while its frame also counts the title bar.
    fn note_bounds(&mut self, window: &Window) {
        let origin = window.window_bounds().get_bounds().origin;
        let size = window.viewport_size();
        self.bounds = Some(SavedBounds {
            x: f32::from(origin.x),
            y: f32::from(origin.y),
            width: f32::from(size.width),
            height: f32::from(size.height),
        });
    }

    /// Save tabs, splits and directories for the next launch. Cheap when
    /// nothing changed: the file is only written when its content differs.
    fn save_session(&self) {
        if self.quitting {
            return;
        }
        let mut saved = self.ws.snapshot();
        saved.bounds = self.bounds;
        let mut registry = self.env.windows.borrow_mut();
        if let Some(entry) = registry
            .entries
            .iter_mut()
            .find(|e| e.view == self.self_weak)
        {
            entry.saved = saved;
            entry.sessions = self
                .ws
                .tabs()
                .iter()
                .flat_map(|t| t.panes())
                .filter_map(|p| {
                    let info = self.ws.pane(p)?;
                    let session = info
                        .agent_session
                        .as_ref()
                        .map(|s| (s.agent.clone(), s.session.clone()))
                        .or_else(|| {
                            let launch = info.agent_launch.as_ref()?;
                            Some((
                                launch.agent.as_str().to_owned(),
                                launch.session.as_ref()?.0.clone(),
                            ))
                        })?;
                    Some((p, (session.0, session.1, info.agent_live)))
                })
                .collect();
            entry.panes =
                self.ws
                    .tabs()
                    .iter()
                    .flat_map(|t| t.panes())
                    .filter_map(|p| {
                        self.ws.pane(p)?.cwd.clone().map(|cwd| {
                            (p, (cwd, self.pane_navigation.get(&p).copied().unwrap_or(0)))
                        })
                    })
                    .collect();
        }
        if self.config.restore_session
            && !registry.restoring
            && let Some(dir) = self.env.data_dir.as_deref()
        {
            let snapshot = registry.snapshot();
            if snapshot.save(dir).is_ok() {
                self.prune_owned_outputs(&snapshot, dir);
            }
        }
    }

    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        self.previous_runs
            .retain(|pane, _| self.ws.pane(*pane).is_some());
        self.previous_open
            .retain(|pane| self.ws.pane(*pane).is_some());
        self.initializing_panes
            .retain(|pane| self.ws.pane(*pane).is_some());
        let panes: Vec<(PaneId, PathBuf)> = self
            .ws
            .tabs()
            .iter()
            .flat_map(|t| t.panes())
            .filter_map(|p| self.ws.pane(p).and_then(|i| i.cwd.clone()).map(|c| (p, c)))
            .collect();
        let located: Vec<(PaneId, Option<String>, Option<PathBuf>)> = {
            let model = &self.sidebar.read(cx).model;
            panes
                .iter()
                .map(|(p, cwd)| match model.worktree_for_path(cwd) {
                    Some((repo, wt)) => (*p, wt.branch.clone(), Some(repo.path.clone())),
                    None => (*p, None, None),
                })
                .collect()
        };
        for (p, branch, repo) in located {
            if let Some(info) = self.ws.pane_mut(p) {
                info.branch = branch;
                info.repo = repo;
            }
        }
        if let Some(active) = self.ws.active_tab() {
            self.tab_group = self.ws.tab_repo(active);
        }
        let active_tabs: Vec<ActiveTab> = {
            let model = &self.sidebar.read(cx).model;
            self.ws
                .tabs_with_activity()
                .into_iter()
                .map(|(tab, title, repo, last_activity)| {
                    let tab_ref = self.ws.tabs().iter().find(|t| t.id == tab);
                    let pane = tab_ref
                        .and_then(|t| t.focused_pane())
                        .and_then(|p| self.ws.pane(p));
                    let branch = pane.and_then(|p| p.branch.clone());
                    let alias = pane
                        .and_then(|p| p.cwd.as_ref())
                        .and_then(|cwd| model.worktree_for_path(cwd))
                        .and_then(|(_, wt)| wt.note_title());
                    let title = match self.config.active_label {
                        chda_config::ActiveLabel::Alias => alias
                            .map(str::to_owned)
                            .or_else(|| branch.clone())
                            .unwrap_or(title),
                        chda_config::ActiveLabel::Branch => branch.clone().unwrap_or(title),
                    };
                    ActiveTab {
                        plain_terminal: tab_ref.is_some_and(|t| {
                            t.terminal().is_some()
                                && t.panes().iter().all(|p| {
                                    self.ws
                                        .pane(*p)
                                        .is_none_or(|i| !i.agent_live && i.agent_launch.is_none())
                                })
                        }),
                        agent_live: tab_ref.is_some_and(|t| {
                            t.panes()
                                .iter()
                                .any(|p| self.ws.pane(*p).is_some_and(|i| i.agent_live))
                        }),
                        branch,
                        status: tab_ref
                            .and_then(|t| self.ws.tab_agent(t))
                            .map(|a| a.status)
                            .unwrap_or_default(),
                        tab,
                        title,
                        repo: repo.and_then(|r| {
                            model
                                .repos
                                .iter()
                                .find(|e| e.path == r)
                                .map(|e| e.name.clone())
                        }),
                        previous_activity: tab_ref.and_then(|t| {
                            t.panes()
                                .iter()
                                .filter_map(|p| self.ws.pane(*p)?.previous_activity)
                                .max()
                        }),
                        last_activity,
                    }
                })
                .collect()
        };
        let idle_agents = self
            .ws
            .tabs()
            .iter()
            .flat_map(|tab| {
                tab.panes()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, pane)| {
                        let info = self.ws.pane(pane)?;
                        let agent = info.agent.as_ref()?;
                        if !info.agent_live || agent.status != AgentStatus::Idle {
                            return None;
                        }
                        let context = info
                            .cwd
                            .as_ref()
                            .and_then(|cwd| self.sidebar.read(cx).model.worktree_for_path(cwd));
                        let location = context
                            .map(|(repo, wt)| {
                                format!(
                                    "{} / {}",
                                    repo.name,
                                    wt.branch.as_deref().unwrap_or("detached HEAD")
                                )
                            })
                            .unwrap_or_else(|| {
                                info.cwd
                                    .as_ref()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "Directory unavailable".into())
                            });
                        Some(IdleAgent {
                            pane,
                            agent: agent.agent.clone(),
                            tab: self.ws.tab_title(tab),
                            pane_index: index + 1,
                            location,
                            cwd: info.cwd.clone(),
                            since: agent.since,
                        })
                    })
            })
            .collect();
        self.sidebar.update(cx, |s, cx| {
            s.model.set_panes(&panes);
            s.model.active_tabs = active_tabs;
            s.model.idle_agents = idle_agents;
            s.active_label = self.config.active_label;
            s.active_collapsed = self.config.active_collapsed;
            s.idle_agents_collapsed = self.config.idle_agents_collapsed;
            s.project_collapsed = self.config.project_collapsed;
            cx.notify();
        });
        self.sync_sidebar_selection(false, cx);
        self.sync_children(cx);
        self.save_session();
        // The title bar also depends on refreshed worktree metadata.
        cx.notify();
    }

    fn save_config(&self) {
        if let Some(path) = &self.env.config_path {
            let _ = self.config.save(path);
        }
    }

    /// Directory new panes start in: the focused pane's.
    fn inherited_cwd(&self) -> Option<PathBuf> {
        self.focused_cwd()
            .or_else(|| self.ws.active_tab()?.directory().map(Path::to_path_buf))
    }

    fn open_pane(
        &mut self,
        pane: PaneId,
        cwd: Option<PathBuf>,
        command: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(info) = self.ws.pane_mut(pane) {
            info.cwd = cwd.clone();
        }
        let settings = self.settings.clone();
        let managed = self
            .ws
            .pane(pane)
            .and_then(|p| p.agent_launch.as_ref())
            .is_some_and(|c| c.managed);
        let result = TerminalView::plan(
            &settings,
            &self.env,
            pane.raw(),
            cwd.clone(),
            command,
            managed,
        )
        .and_then(TerminalView::prepare);
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(e) => {
                self.initializing_panes.remove(&pane);
                if self.env.windows.borrow().update.adopting {
                    self.env.windows.borrow_mut().update.adoption_failed = true;
                }
                if let Some(info) = self.ws.pane_mut(pane) {
                    info.managed_run = Some(chda_core::ManagedRun::failed(format!(
                        "Could not start process: {e}"
                    )));
                    info.agent_live = false;
                }
                self.notify_error(format!("Could not start the pane: {e}"));
                self.sync_panes(cx);
                return;
            }
        };
        self.attach_terminal(pane, prepared, settings, cwd, window, cx);
    }

    fn attach_terminal(
        &mut self,
        pane: PaneId,
        prepared: crate::terminal_view::PreparedTerminal,
        settings: Settings,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.env.windows.borrow().update.adopting
            && let Some(context) = self
                .ws
                .pane(pane)
                .and_then(|p| p.agent_launch.clone())
                .filter(|c| c.managed)
        {
            self.ws
                .set_agent_status(pane, context.agent.as_str(), AgentStatus::Idle, now_ms());
            self.ws.pane_mut(pane).unwrap().agent_live = true;
            self.initializing_panes.remove(&pane);
            if let Some(session) = context.session {
                self.ws.set_agent_session(
                    pane,
                    Some(AgentSessionRef {
                        agent: context.agent.as_str().into(),
                        session: session.0,
                    }),
                );
            }
        }
        if let Some(info) = self.ws.pane_mut(pane)
            && info.agent_launch.as_ref().is_some_and(|c| c.managed)
        {
            let started_at = info
                .managed_run
                .as_ref()
                .and_then(chda_core::ManagedRun::started_at)
                .unwrap_or_else(now_ms);
            info.managed_run = Some(chda_core::ManagedRun::Running { started_at });
        }
        let view = cx.new(|cx| TerminalView::new(prepared, settings, cwd, window, cx));
        let sub = cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
            this.on_pane_event(pane, event, window, cx)
        });
        self.panes.insert(pane, (view, sub));
        self.sync_panes(cx);
    }

    /// Codex reports its runtime state in the OSC title chda
    /// requests. Route it through the same path as hooks, tied to this pane.
    fn apply_codex_terminal_event(
        &mut self,
        pane: PaneId,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(info) = self.ws.pane(pane) else {
            return;
        };
        let previous = parse_codex_title(&info.title);
        let ending = matches!(event, TerminalEvent::Prompt | TerminalEvent::Exited { .. })
            || matches!(event, TerminalEvent::Title(t) if t.is_empty());
        let kind = if ending {
            if previous.is_none()
                && !info.agent.as_ref().is_some_and(|a| a.agent == "codex")
                && !info
                    .agent_session
                    .as_ref()
                    .is_some_and(|s| s.agent == "codex")
            {
                return;
            }
            HookKind::SessionEnd
        } else if let TerminalEvent::Title(title) = event {
            let Some(state) = parse_codex_title(title) else {
                return;
            };
            if previous == Some(state) {
                return;
            }
            match state {
                CodexRunState::Working => HookKind::PromptSubmitted,
                CodexRunState::WaitingInput => HookKind::WaitingInput,
                CodexRunState::Ready => {
                    if !previous.is_some_and(|p| p != CodexRunState::Ready)
                        || !info.agent.as_ref().is_some_and(|a| {
                            a.agent == "codex"
                                && matches!(
                                    a.status,
                                    AgentStatus::Working | AgentStatus::WaitingInput
                                )
                        })
                    {
                        self.ws
                            .set_agent_status(pane, "codex", AgentStatus::Idle, now_ms());
                        if let Some(info) = self.ws.pane_mut(pane) {
                            info.agent_live = true;
                        }
                        self.sync_attention(cx);
                        return;
                    }
                    HookKind::Stopped
                }
            }
        } else {
            return;
        };
        let Some(cwd) = self.ws.pane(pane).and_then(|p| p.cwd.clone()) else {
            return;
        };
        self.apply_hook_event(
            HookEvent {
                child: None,
                agent: "codex".into(),
                session_id: String::new(),
                cwd,
                kind,
                timestamp: now_ms(),
                pane: Some(pane.raw()),
            },
            window,
            cx,
        );
    }

    fn on_pane_event(
        &mut self,
        pane: PaneId,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_codex_terminal_event(pane, event, window, cx);
        match event {
            TerminalEvent::Exited { status, output } => {
                if self
                    .ws
                    .pane(pane)
                    .and_then(|p| p.agent_launch.as_ref())
                    .is_some_and(|c| c.managed)
                {
                    self.managed_exit(pane, status.as_ref(), output.as_ref(), window, cx);
                    return;
                }
                self.ws.close_pane(pane);
                self.panes.remove(&pane);
                if self.ws.is_empty() {
                    // Nothing left to restore: this removes the saved session.
                    self.save_session();
                    window.remove_window();
                    return;
                }
                self.focus_active(window, cx);
                self.sync_panes(cx);
            }
            TerminalEvent::Title(title) => {
                // Agents such as Codex animate their title while working. A
                // pane title only shows as its tab's fallback label (and the
                // title bar's, for the active tab); otherwise nothing redraws.
                let before = self.pane_tab_title(pane);
                if let Some(info) = self.ws.pane_mut(pane) {
                    info.title = title.clone();
                }
                if self.pane_tab_title(pane) == before {
                    return;
                }
            }
            TerminalEvent::Cwd(cwd) => {
                if let Some(info) = self.ws.pane_mut(pane) {
                    info.cwd = Some(cwd.clone());
                }
                self.sync_panes(cx);
                self.refresh_repo_of(cwd, cx);
            }
            TerminalEvent::Bell => {
                if self.ws.focused_pane() != Some(pane)
                    && let Some(info) = self.ws.pane_mut(pane)
                {
                    info.bell = true;
                }
                self.env.system.beep();
            }
            TerminalEvent::Focused => {
                self.ws.focus_pane(pane);
                self.context_menu = None;
                self.focus_active(window, cx);
            }
            TerminalEvent::Prompt => {
                self.initializing_panes.remove(&pane);
                // A shell prompt after an agent ran means the agent exited.
                let agent = self
                    .ws
                    .pane(pane)
                    .and_then(|i| i.agent.as_ref())
                    .map(|a| a.agent.clone());
                if self.ws.end_agent(pane) {
                    if let Some(cwd) = self.ws.pane(pane).and_then(|i| i.cwd.clone())
                        && let Some(agent) = agent
                    {
                        self.sync_worktree_agent_status(&agent, &cwd, cx);
                    }
                    self.sync_attention(cx);
                }
                if self.ws.set_agent_session(pane, None) {
                    self.save_session();
                }
                if self.sidebar_visible
                    && let Some(cwd) = self.ws.pane(pane).and_then(|i| i.cwd.clone())
                {
                    self.refresh_repo_of(&cwd, cx);
                }
            }
            TerminalEvent::OpenPath { path, line, column } => {
                self.open_path(path, *line, *column, cx);
            }
            TerminalEvent::ViewDiagram(source) => self.view_diagram(source.as_deref(), cx),
            TerminalEvent::SelectionRead { request, text } => {
                self.receive_share_selection(pane, *request, text.clone(), window, cx)
            }
            TerminalEvent::SelectionMenu {
                position,
                link,
                at_prompt,
            } => {
                let mut items = vec![
                    ("Copy".into(), MenuAction::CopySelection(pane)),
                    (
                        "Copy for sharing…".into(),
                        MenuAction::ShareSelection {
                            pane,
                            identity: self.share_identity(pane),
                        },
                    ),
                ];
                if let Some(link) = link {
                    let cwd = self.ws.pane(pane).and_then(|p| p.cwd.as_deref());
                    items.extend(crate::link_menu::items(link, pane, cwd, *at_prompt));
                }
                self.context_menu = Some(ContextMenu {
                    position: *position,
                    items,
                });
            }
            TerminalEvent::LinkMenu {
                link,
                position,
                at_prompt,
            } => {
                let cwd = self.ws.pane(pane).and_then(|i| i.cwd.clone());
                self.context_menu = Some(ContextMenu {
                    position: *position,
                    items: crate::link_menu::items(link, pane, cwd.as_deref(), *at_prompt),
                });
            }
            TerminalEvent::Activity(at) => {
                if self.initializing_panes.contains(&pane) {
                    return;
                }
                if let Some(info) = self.ws.pane_mut(pane) {
                    info.last_activity = *at;
                }
                // Only ACTIVE's ages show this, and their once-a-second timer
                // redraws the sidebar; output alone redraws nothing else.
                let activity = self.ws.tabs_with_activity();
                self.sidebar.update(cx, |s, _| {
                    for tab in &mut s.model.active_tabs {
                        if let Some((.., at)) = activity.iter().find(|(id, ..)| *id == tab.tab) {
                            tab.last_activity = *at;
                        }
                    }
                });
                // The next launch shows this as the previous working time.
                // Output saves at most once per delay, off the hot path;
                // quitting saves at once.
                if !self.activity_save_pending {
                    self.activity_save_pending = true;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(ACTIVITY_SAVE_DELAY).await;
                        let _ = this.update(cx, |view, _| {
                            view.activity_save_pending = false;
                            view.save_session();
                        });
                    })
                    .detach();
                }
                return;
            }
        }
        self.sync_title(window, cx);
        cx.notify();
    }

    /// Render a Mermaid diagram in the browser from a local page.
    fn view_diagram(&mut self, source: Option<&str>, cx: &mut Context<Self>) {
        let Some(source) = source else {
            self.notify("No Mermaid diagram in this pane's output".into());
            return;
        };
        let Some(dir) = self.env.data_dir.as_ref().map(|d| d.join("diagrams")) else {
            return;
        };
        match crate::diagram::write_page(&dir, source) {
            Ok(page) => self.env.system.open_file(&page, cx),
            Err(e) => self.notify_error(format!("diagram: {e}")),
        }
    }

    /// The folder the title bar opens: the root of the worktree the focused
    /// pane is in, else that pane's directory.
    pub(crate) fn open_in_folder(&self, cx: &App) -> Option<PathBuf> {
        if let Some(repo) = self.ws.active_tab().and_then(|tab| tab.directory()) {
            return Some(repo.to_path_buf());
        }
        let cwd = self.focused_cwd()?;
        let root = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(&cwd)
            .map(|(_, w)| w.path.clone());
        Some(root.unwrap_or(cwd))
    }

    /// The title bar's app: the one picked last, else the first installed.
    pub(crate) fn folder_app(&self) -> Option<&crate::platform::FolderApp> {
        self.config
            .open_in
            .as_deref()
            .and_then(|id| self.folder_apps.iter().find(|a| a.id == id))
            .or_else(|| self.folder_apps.first())
    }

    /// Open the focused worktree in the app with `id` and remember the app.
    pub(crate) fn open_folder_in(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(folder) = self.open_in_folder(cx) else {
            self.notify("No folder to open: the focused pane has no directory".into());
            cx.notify();
            return;
        };
        if let Err(e) = self.env.system.open_folder_in(id, &folder) {
            self.notify_error(format!("Could not open {}: {e}", folder.display()));
        }
        if self.config.open_in.as_deref() != Some(id) {
            self.config.open_in = Some(id.to_owned());
            self.save_config();
        }
        cx.notify();
    }

    /// The window's own title bar: drag area, title, and the "open in" app
    /// picker and button.
    /// The grab area on the sidebar's right border: drag to resize,
    /// double-click for the default width.
    fn render_sidebar_grip(&self, divider: Hsla, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("sidebar-grip")
            .debug_selector(|| "sidebar-grip".into())
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(SIDEBAR_GRIP))
            .cursor_col_resize()
            .when(self.sidebar_drag, |d| d.bg(divider))
            .hover(|s| s.bg(divider))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if e.click_count == 2 {
                        this.sidebar_drag = false;
                        this.config.sidebar_width = ChdaConfig::default().sidebar_width;
                        this.save_config();
                    } else {
                        this.sidebar_drag = true;
                    }
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    /// Repository and worktree task context remain visible independently of
    /// a custom name assigned to the tab.
    pub(crate) fn title_bar_text(&self, cx: &App) -> (String, String) {
        let Some(tab) = self.ws.active_tab() else {
            return ("chda".into(), "chda".into());
        };
        let fallback = self.ws.tab_title(tab);
        let worktree = tab
            .focused_pane()
            .and_then(|pane| self.ws.pane(pane))
            .and_then(|info| info.cwd.as_deref())
            .and_then(|cwd| self.sidebar.read(cx).model.worktree_for_path(cwd));
        let Some((repo, worktree)) = worktree else {
            return (fallback.clone(), fallback);
        };
        let branch = worktree.branch.as_deref().unwrap_or("detached HEAD");
        let task = worktree.note_title().unwrap_or(branch);
        let title = if repo.folder {
            repo.name.clone()
        } else {
            format!("{} - {task}", repo.name)
        };
        let tooltip = match worktree
            .note
            .as_deref()
            .filter(|note| !note.trim().is_empty())
        {
            Some(note) => format!(
                "Repository: {}\n{note}\nBranch: {branch}\n{}",
                repo.name,
                worktree.path.display()
            ),
            None => format!("{title}\nBranch: {branch}\n{}", worktree.path.display()),
        };
        (title, tooltip)
    }

    fn render_update_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let progress = self.env.windows.borrow().update.progress.clone();
        if self.release_notice.is_none() && !progress.busy() {
            return None;
        }
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let stop = |_: &MouseDownEvent, _: &mut Window, cx: &mut App| cx.stop_propagation();
        let mut bar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .min_w_0()
            .text_sm()
            .text_color(fg);
        if let Some(release) = &self.release_notice {
            let url = release.url.clone();
            let progress = self.env.windows.borrow().update.progress.clone();
            let label = if progress.busy() {
                progress.label()
            } else {
                format!("Update to {}", release.version)
            };
            bar = bar.child(
                div().id("update-notice").debug_selector(|| "update-notice".into())
                    .h(px(22.0)).px_2().flex().items_center().rounded_md()
                    .bg(fg.opacity(0.1)).text_color(fg).cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.18))).on_mouse_down(MouseButton::Left, stop)
                    .tooltip(crate::tooltip::text("Download, verify and install the update. Running sessions reconnect when the window reopens."))
                    .on_click(cx.listener(|this, _, window, cx| this.start_update(window, cx)))
                    .child(optical(label)),
            ).child(
                div().id("update-menu").debug_selector(|| "update-menu".into())
                    .h(px(22.0)).px_2().cursor_pointer().on_mouse_down(MouseButton::Left, stop)
                    .tooltip(crate::tooltip::text("Release notes and update options"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.context_menu = Some(ContextMenu {
                            position: point((window.viewport_size().width - px(330.0)).max(px(0.0)), px(TITLE_BAR_HEIGHT)),
                            items: vec![
                                ("Release notes".into(), MenuAction::OpenUrl(url.clone())),
                                ("Dismiss until the next release".into(), MenuAction::DismissUpdate),
                            ],
                        });
                        cx.notify();
                    })).child(optical("⋯")),
            );
        }
        let progress = self.env.windows.borrow().update.progress.clone();
        if progress.busy() {
            if self.release_notice.is_none() {
                bar = bar.child(div().h(px(22.0)).px_2().child(optical(progress.label())));
            }
            if self.env.windows.borrow().update.job.is_some() {
                bar = bar.child(
                    div()
                        .id("cancel-update")
                        .debug_selector(|| "cancel-update".into())
                        .h(px(22.0))
                        .px_2()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, stop)
                        .on_click(cx.listener(|this, _, _, _| {
                            if let Some(job) = &this.env.windows.borrow().update.job {
                                let _ = job.signal("cancel");
                            }
                        }))
                        .child(optical("Cancel")),
                );
            }
        }
        Some(bar.into_any_element())
    }

    fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let (title, tooltip) = self.title_bar_text(cx);
        let title_selector = format!("title-text:{title}");
        let inset = if window.is_fullscreen() {
            8.0
        } else {
            crate::platform::TITLE_BAR_INSET
        };
        let stop = |_: &MouseDownEvent, _: &mut Window, cx: &mut App| cx.stop_propagation();
        let mut bar = div()
            .id("title-bar")
            .window_control_area(gpui::WindowControlArea::Drag)
            .flex_shrink_0()
            .h(px(TITLE_BAR_HEIGHT))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .pl(px(inset))
            .pr_2()
            .gap_2()
            .bg(blend(bg, fg, 0.05))
            .border_b_1()
            .border_color(fg.opacity(0.12))
            .text_sm()
            .text_color(fg.opacity(0.75))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.title_drag = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.title_drag = false),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.title_drag {
                    this.title_drag = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(
                div()
                    .id("sidebar-toggle")
                    .debug_selector(|| "sidebar-toggle".into())
                    .h(px(22.0))
                    .px_1()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .text_color(fg.opacity(if self.sidebar_visible { 0.8 } else { 0.55 }))
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.1)))
                    .on_mouse_down(MouseButton::Left, stop)
                    .tooltip(crate::tooltip::text(if self.sidebar_visible {
                        "Hide the sidebar (cmd-b)"
                    } else {
                        "Show the sidebar (cmd-b)"
                    }))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_sidebar(&ToggleSidebar, window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .font_family(crate::fonts::SYMBOLS_FAMILY)
                            .text_base()
                            .child(optical(ICON_SIDEBAR)),
                    ),
            )
            .child(
                div()
                    .id("active-title")
                    .debug_selector(|| "active-title".into())
                    .flex_1()
                    .min_w_0()
                    .text_center()
                    .tooltip(crate::tooltip::text(tooltip))
                    .child(
                        optical(title)
                            .debug_selector(move || title_selector.clone())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis(),
                    ),
            );
        if window.viewport_size().width >= px(700.0) {
            bar = bar.children(self.render_update_controls(cx));
        }
        bar = bar.child(self.render_notification_button(cx));
        if let Some(app) = self.folder_app() {
            let folder = self.open_in_folder(cx);
            let folder_name = folder
                .as_ref()
                .and_then(|f| f.file_name())
                .map(|n| n.to_string_lossy().into_owned());
            let (id, name, icon) = (app.id.clone(), app.name.clone(), app.icon.clone());
            let button = |id: &'static str| {
                div()
                    .id(id)
                    .h(px(22.0))
                    .px_2()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(fg.opacity(0.18))
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.1)))
                    .on_mouse_down(MouseButton::Left, stop)
            };
            let picker = button("open-in-pick")
                .debug_selector(|| "open-in-pick".into())
                .tooltip(crate::tooltip::text(
                    "Choose the app the button opens the worktree in",
                ))
                .on_click(cx.listener(|this, _, window, cx| {
                    let items = this
                        .folder_apps
                        .iter()
                        .map(|a| (a.name.clone(), MenuAction::PickFolderApp(a.id.clone())))
                        .collect();
                    let width = window.viewport_size().width;
                    this.context_menu = Some(ContextMenu {
                        position: point(width - px(230.0), px(TITLE_BAR_HEIGHT)),
                        items,
                    });
                    cx.notify();
                }))
                .children(icon.map(|i| img(i).size(px(16.0)).flex_shrink_0()))
                .child(optical(name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(fg.opacity(0.6))
                        .child("\u{25be}"),
                );
            let tip = match &folder_name {
                Some(f) => format!("Open {f} in {name}"),
                None => format!("Open the focused worktree in {name}"),
            };
            let go = button("open-in-go")
                .debug_selector(|| "open-in-go".into())
                .tooltip(crate::tooltip::text(tip))
                .when(folder.is_none(), |d| d.opacity(0.5))
                .on_click(cx.listener(move |this, _, _, cx| this.open_folder_in(&id, cx)))
                .child(optical("\u{25b6}"));
            bar = bar.child(div().flex().flex_row().gap_1().child(picker).child(go));
        }
        bar.into_any_element()
    }

    /// Render a Markdown file to a local page and open it in the browser.
    fn preview_markdown(&mut self, file: &Path, cx: &mut Context<Self>) {
        let Some(dir) = self.env.data_dir.as_ref().map(|d| d.join("diagrams")) else {
            return;
        };
        match crate::markdown_preview::write_page(&dir, file) {
            Ok(page) => self.env.system.open_file(&page, cx),
            Err(e) => self.notify_error(format!("Markdown preview: {e}")),
        }
        cx.notify();
    }

    /// Open a cmd-clicked file with the configured editor, else the system's
    /// default application; folders always go to the file manager.
    fn open_path(
        &mut self,
        path: &Path,
        line: Option<u32>,
        column: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let template = self.config.editor.as_deref().filter(|_| !path.is_dir());
        let Some(template) = template else {
            self.env.system.open_file(path, cx);
            return;
        };
        let argv = chda_config::editor_command(template, path, line, column);
        let Some((program, args)) = argv.split_first() else {
            return;
        };
        if let Err(e) = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            self.notify_error(format!("editor `{program}`: {e}"));
        }
    }

    /// Looking at a pane clears "review" for it, and marks
    /// a waiting agent there as seen (which clears the Dock badge).
    fn reviewed_focused(&mut self, cx: &mut Context<Self>) {
        if self
            .ws
            .focused_pane()
            .and_then(|p| self.ws.pane(p))
            .is_none_or(|i| i.agent.is_none())
            && let Some(cwd) = self.focused_cwd()
        {
            self.sidebar.update(cx, |s, cx| {
                if s.model.mark_reviewed(&cwd) {
                    cx.notify();
                }
            });
        }
        if let Some(pane) = self.ws.focused_pane()
            && self.ws.mark_seen(pane)
        {
            if let Some(info) = self.ws.pane(pane)
                && let (Some(agent), Some(cwd)) = (info.agent.as_ref(), info.cwd.as_ref())
            {
                let agent = agent.agent.clone();
                let cwd = cwd.clone();
                self.sync_worktree_agent_status(&agent, &cwd, cx);
            }
            self.sync_attention(cx);
        }
    }

    fn sync_sidebar_selection(&mut self, navigation: bool, cx: &mut Context<Self>) {
        let tab = self.ws.active_tab().map(|t| t.id);
        let cwd = self
            .focused_cwd()
            .or_else(|| self.ws.active_tab()?.directory().map(Path::to_path_buf));
        if navigation
            && self.config.project_collapsed
            && cwd
                .as_ref()
                .is_some_and(|cwd| self.sidebar.read(cx).model.worktree_for_path(cwd).is_some())
        {
            self.config.project_collapsed = false;
            self.save_config();
            self.sidebar.update(cx, |s, _| s.project_collapsed = false);
        }
        self.sidebar.update(cx, |s, cx| {
            s.select_context(tab, cwd.as_deref(), navigation, cx)
        });
    }

    /// Give keyboard focus to the workspace's focused pane.
    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .confirm
            .as_ref()
            .is_some_and(|confirm| matches!(confirm.action, MenuAction::StopAndClose { .. }))
        {
            window.focus(&self.focus_handle, cx);
        } else if let Some(pane) = self.ws.focused_pane()
            && let Some((view, _)) = self.panes.get(&pane)
        {
            let handle = view.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
        if let Some(active) = self.ws.active_tab() {
            self.tab_group = self.ws.tab_repo(active);
        }
        let order = {
            let mut registry = self.env.windows.borrow_mut();
            registry.navigation += 1;
            registry.active = Some(window.window_handle().window_id());
            registry.navigation
        };
        if let Some(pane) = self.ws.focused_pane() {
            self.pane_navigation.insert(pane, order);
        }
        if !self.env.windows.borrow().update.adopting {
            self.reviewed_focused(cx);
        }
        // ACTIVE labels depend on split focus, even when no agent status or
        // terminal output changed. Keep each row in sync with its click target.
        self.sync_panes(cx);
        self.sync_sidebar_selection(true, cx);
        self.sync_title(window, cx);
        cx.notify();
    }

    /// The label of the tab that holds `pane`.
    fn pane_tab_title(&self, pane: PaneId) -> Option<String> {
        let tab = self.ws.tabs().iter().find(|t| t.panes().contains(&pane))?;
        Some(self.ws.tab_title(tab))
    }

    /// Setting the macOS window title relayouts the title bar, so only a
    /// changed title is set.
    fn sync_title(&mut self, window: &mut Window, cx: &App) {
        let (title, _) = self.title_bar_text(cx);
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active(window, cx);
    }

    fn refocus_terminal(&self, window: &mut Window, cx: &mut App) {
        if let Some(pane) = self.ws.focused_pane()
            && let Some((terminal, _)) = self.panes.get(&pane)
        {
            window.focus(&terminal.read(cx).focus_handle(cx), cx);
        }
    }

    /// Change the font size of every pane in the window. Each terminal
    /// re-measures its cells on the next layout and resizes its PTY.
    fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let size = px(size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX));
        if size == self.settings.font_size {
            return;
        }
        self.settings.font_size = size;
        for (view, _) in self.panes.values() {
            view.update(cx, |view, cx| {
                view.settings.font_size = size;
                cx.notify();
            });
        }
        cx.notify();
    }

    fn increase_font_size(&mut self, _: &IncreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        let size = f32::from(self.settings.font_size) + FONT_SIZE_STEP;
        self.set_font_size(size, cx);
    }

    fn decrease_font_size(&mut self, _: &DecreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        let size = f32::from(self.settings.font_size) - FONT_SIZE_STEP;
        self.set_font_size(size, cx);
    }

    fn reset_font_size(&mut self, _: &ResetFontSize, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(f32::from(self.configured_font_size), cx);
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.inherited_cwd();
        self.open_tab_at(cwd, None, window, cx);
    }

    fn open_tab_at(
        &mut self,
        cwd: Option<PathBuf>,
        command: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (_, pane) = self.ws.new_tab();
        self.open_pane(pane, cwd, command, window, cx);
        self.focus_active(window, cx);
    }

    fn close_surface(&mut self, _: &CloseSurface, window: &mut Window, cx: &mut Context<Self>) {
        if self.share_sheet.is_some() {
            self.close_share_sheet(window, cx);
            return;
        }
        if self.confirm.is_some() || self.sheet.as_ref().is_some_and(|sheet| sheet.busy) {
            return;
        }
        if self.sheet.take().is_some()
            || self.note_sheet.take().is_some()
            || self.context_menu.take().is_some()
        {
            self.focus_active(window, cx);
            return;
        }
        let target = if let Some(pane) = self.ws.focused_pane() {
            CloseTarget::Pane(pane)
        } else if let Some(tab) = self.ws.active_tab() {
            CloseTarget::Tab(tab.id)
        } else {
            return;
        };
        self.request_close(target, window, cx);
    }

    pub(crate) fn request_close(
        &mut self,
        target: CloseTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.confirm.is_some() {
            return;
        }
        let Some(tab) = self.ws.tabs().iter().find(|tab| match target {
            CloseTarget::Tab(id) => tab.id == id,
            CloseTarget::Pane(id) => tab.panes().contains(&id),
        }) else {
            return;
        };
        let title = self.ws.tab_title(tab);
        let panes = match target {
            CloseTarget::Tab(_) => tab.panes(),
            CloseTarget::Pane(pane) => vec![pane],
        };
        let lines: Vec<_> = panes
            .iter()
            .filter_map(|pane| {
                let info = self.ws.pane(*pane)?;
                let agent = info.agent.as_ref()?;
                let status = match agent.status {
                    AgentStatus::Working => "working",
                    AgentStatus::WaitingInput => "waiting for input",
                    AgentStatus::Review | AgentStatus::Idle => return None,
                };
                let session = info
                    .agent_session
                    .as_ref()
                    .map(|session| session.session.as_str())
                    .unwrap_or("session unavailable");
                Some(format!(
                    "{} · {status} · pane {} · {session}",
                    self.agent_name(&agent.agent),
                    pane.raw()
                ))
            })
            .collect();
        if lines.is_empty() {
            self.finish_close(target, window, cx);
            return;
        }
        let Some(active) = self.ws.active_tab() else {
            return;
        };
        self.confirm = Some(ConfirmSheet {
            title: format!("Stop sessions and close “{title}”?"),
            lines,
            action: MenuAction::StopAndClose {
                target,
                focus: CloseFocus {
                    tab: active.id,
                    pane: active.focused_pane(),
                },
            },
        });
        // The confirmation owns keyboard focus: Esc must never reach an agent.
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn finish_close(&mut self, target: CloseTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            CloseTarget::Tab(id) => {
                let Some(tab) = self.ws.tabs().iter().find(|tab| tab.id == id) else {
                    return;
                };
                let panes = tab.panes();
                self.ws.close_tab(id);
                self.graphs.remove(&id);
                self.reviews.remove(&id);
                for pane in panes {
                    self.panes.remove(&pane);
                }
            }
            CloseTarget::Pane(pane) => {
                if self.ws.pane(pane).is_none() {
                    return;
                }
                self.ws.close_pane(pane);
                self.panes.remove(&pane);
            }
        }
        if self
            .renaming
            .as_ref()
            .is_some_and(|(id, _, _)| !self.ws.tabs().iter().any(|tab| tab.id == *id))
        {
            self.renaming = None;
        }
        if self.ws.is_empty() {
            self.save_session();
            window.remove_window();
            return;
        }
        self.focus_active(window, cx);
        self.sync_attention(cx);
    }

    fn restore_close_focus(
        &mut self,
        focus: CloseFocus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = focus.pane {
            if !self.ws.focus_pane(pane) {
                self.ws.activate_tab_id(focus.tab);
            }
        } else {
            self.ws.activate_tab_id(focus.tab);
        }
        self.focus_active(window, cx);
    }

    fn cancel_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ConfirmSheet {
            action: MenuAction::StopAndClose { focus, .. },
            ..
        }) = self.confirm.take()
        {
            self.restore_close_focus(focus, window, cx);
        } else {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    /// Close `panes`; an emptied window gets a fresh tab instead of quitting.
    fn close_panes(&mut self, panes: &[PaneId], window: &mut Window, cx: &mut Context<Self>) {
        if panes.is_empty() {
            return;
        }
        for pane in panes {
            self.ws.close_pane(*pane);
            self.panes.remove(pane);
        }
        if self.ws.is_empty() {
            self.open_tab_at(None, None, window, cx);
        } else {
            self.focus_active(window, cx);
        }
        self.sync_panes(cx);
    }

    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.inherited_cwd();
        if let Some(pane) = self.ws.split(axis) {
            self.open_pane(pane, cwd, None, window, cx);
            self.focus_active(window, cx);
        }
    }

    fn goto_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let group = self.tab_group.clone();
        if self.ws.activate_tab_in(group.as_deref(), index) {
            self.focus_active(window, cx);
        }
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.config.sidebar_visible = self.sidebar_visible;
        self.save_config();
        if self.sidebar_visible {
            self.refresh_all(cx);
        }
        self.focus_active(window, cx);
    }

    /// The sidebar's width in this window.
    fn sidebar_width(&self, window: &Window) -> f32 {
        clamp_sidebar(
            self.config.sidebar_width as f32,
            f32::from(window.viewport_size().width),
        )
    }

    /// Follow the pointer while the sidebar's border is dragged.
    fn drag_sidebar(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sidebar_drag {
            return;
        }
        if e.pressed_button != Some(MouseButton::Left) {
            // Released outside the window.
            self.end_sidebar_drag(cx);
            return;
        }
        let width = clamp_sidebar(
            f32::from(e.position.x),
            f32::from(window.viewport_size().width),
        );
        let width = width.round() as u32;
        if width != self.config.sidebar_width {
            self.config.sidebar_width = width;
            cx.notify();
        }
    }

    fn end_sidebar_drag(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.sidebar_drag) {
            self.save_config();
            cx.notify();
        }
    }

    fn add_repo(&mut self, _: &AddRepo, window: &mut Window, cx: &mut Context<Self>) {
        let operation = self.env.windows.borrow().update.operation();
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add repository".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let _operation = operation;
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let _ = this.update_in(cx, |view, window, cx| {
                for path in paths {
                    view.register_repo(path, window, cx);
                }
            });
        })
        .detach();
    }

    /// Add a folder the user picked or dropped: its repository, or, when it
    /// is in none, ask whether to run `git init` or add it as it is.
    fn register_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match chda_core::repo_of(&path) {
            Ok(repo) => self.add_to_sidebar(repo, cx),
            Err(_) if path.is_dir() => self.ask_about_folder(path, window, cx),
            Err(e) => {
                self.notify_error(format!("{}: {e}", path.display()));
                cx.notify();
            }
        }
    }

    fn ask_about_folder(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let operation = self.env.windows.borrow().update.operation();
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("{name} is not a git repository."),
            Some(
                "Initialize git to get worktrees, branches and badges, \
                 or add it as a plain folder.",
            ),
            &[INIT_GIT, ADD_AS_FOLDER, "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let _operation = operation;
            let Ok(choice) = answer.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| match choice {
                0 => view.init_git(path, cx),
                1 => view.add_to_sidebar(path, cx),
                _ => {}
            });
        })
        .detach();
    }

    /// `git init` in a folder, then show it as a repository: added to the
    /// sidebar if it is not there yet, refreshed if it was a plain folder.
    fn init_git(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn({
            let path = path.clone();
            async move { chda_core::init_repository(&path) }
        });
        cx.spawn(async move |this, cx| {
            let _operation = operation;
            let result = task.await;
            let _ = this.update(cx, |view, cx| match result {
                Ok(()) => {
                    if view.sidebar.read(cx).model.is_folder(&path) {
                        view.refresh_repo(path, cx);
                    } else {
                        view.add_to_sidebar(path, cx);
                    }
                }
                Err(e) => {
                    view.notify_repo(
                        format!("git init in {}: {e}", path.display()),
                        Severity::Error,
                        &path,
                        None,
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Register a repository's main worktree, or a plain folder.
    fn add_to_sidebar(&mut self, repo: PathBuf, cx: &mut Context<Self>) {
        let added = self.sidebar.update(cx, |s, cx| {
            let added = s.model.add_repo(repo.clone());
            cx.notify();
            added
        });
        if added {
            self.config.repos.push(repo.clone());
            self.save_config();
            self.watch_repo(&repo);
            self.refresh_repo(repo, cx);
            self.refresh_sessions(cx);
        }
    }

    pub(crate) fn on_sidebar_event(
        &mut self,
        event: SidebarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = None;
        match event {
            SidebarEvent::ChildDetails(child) => self.open_child_details(child, window, cx),
            SidebarEvent::RefocusTerminal => self.refocus_terminal(window, cx),
            SidebarEvent::OpenWorktree(path) => self.open_worktree(&path, window, cx),
            SidebarEvent::WorktreeMenu(path, position) => {
                let repo = self
                    .sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&path)
                    .map(|(r, w)| (r.path.clone(), w.is_main, w.missing, w.clone()));
                let Some((repo, is_main, missing, entry)) = repo else {
                    return;
                };
                if missing {
                    self.context_menu = Some(ContextMenu {
                        position,
                        items: vec![
                            (
                                "Remove missing worktree (keep branch)".into(),
                                MenuAction::PruneMissing {
                                    repo: repo.clone(),
                                    worktree: path.clone(),
                                },
                            ),
                            (
                                "Remove missing worktree and branch...".into(),
                                MenuAction::DeleteWorktreeAndBranch {
                                    repo,
                                    worktree: path,
                                    force: false,
                                },
                            ),
                        ],
                    });
                    cx.notify();
                    return;
                }
                let mut items = vec![(
                    "Open terminal".to_owned(),
                    MenuAction::OpenTerminal(path.clone()),
                )];
                if self.sidebar.read(cx).model.is_folder(&repo) {
                    items.extend(self.agent_launch_items(&path));
                    items.push((INIT_GIT.into(), MenuAction::InitGit(repo.clone())));
                    items.push(("Remove from sidebar".into(), MenuAction::RemoveRepo(repo)));
                    self.context_menu = Some(ContextMenu { position, items });
                    cx.notify();
                    return;
                }
                if let Some(base) = self
                    .sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&path)
                    .and_then(|(_, w)| w.diff.as_ref())
                    .map(|d| d.base.clone())
                {
                    items.push((
                        format!("View diff against {base}"),
                        MenuAction::ViewDiff(path.clone()),
                    ));
                    items.push(("Review diff…".into(), MenuAction::ReviewDiff(path.clone())));
                }
                items.extend(self.agent_launch_items(&path));
                items.push((
                    "New worktree...".into(),
                    MenuAction::NewWorktree(repo.clone()),
                ));
                if let Some(branch) = entry.branch.clone() {
                    items.push((
                        "New worktree from this branch...".into(),
                        MenuAction::NewWorktreeFrom {
                            repo: repo.clone(),
                            base: branch.clone(),
                        },
                    ));
                    items.push((
                        "Edit note...".into(),
                        MenuAction::EditNote {
                            repo: repo.clone(),
                            branch: branch.clone(),
                        },
                    ));
                    let starred = self.config.is_starred(&repo, &branch);
                    items.push((
                        if starred {
                            "Remove from Starred"
                        } else {
                            "Add to Starred"
                        }
                        .into(),
                        MenuAction::Star {
                            repo: repo.clone(),
                            branch,
                            starred: !starred,
                        },
                    ));
                }
                items.push(match chda_core::update_blocker(&entry) {
                    None => (
                        match entry.badges.behind.filter(|b| *b > 0) {
                            Some(n) => format!("Update branch (\u{2193}{n})"),
                            None => "Update branch".into(),
                        },
                        MenuAction::UpdateBranch {
                            repo: repo.clone(),
                            worktree: path.clone(),
                            strategy: None,
                        },
                    ),
                    Some(blocker) => (
                        "Update branch".into(),
                        MenuAction::Unavailable(blocker.describe(|id| self.agent_name(id))),
                    ),
                });
                if !is_main {
                    items.push((
                        "Merge into main and clean up".into(),
                        MenuAction::MergeAndClean {
                            repo: repo.clone(),
                            worktree: path.clone(),
                            force: false,
                        },
                    ));
                    items.push((
                        "Delete worktree...".into(),
                        MenuAction::DeleteWorktreeAndBranch {
                            repo,
                            worktree: path,
                            force: false,
                        },
                    ));
                }
                self.context_menu = Some(ContextMenu { position, items });
            }
            SidebarEvent::RepoMenu(repo, position) => {
                if self.sidebar.read(cx).model.is_folder(&repo) {
                    self.context_menu = Some(ContextMenu {
                        position,
                        items: vec![
                            (INIT_GIT.into(), MenuAction::InitGit(repo.clone())),
                            ("Refresh".into(), MenuAction::RefreshRepo(repo.clone())),
                            ("Remove from sidebar".into(), MenuAction::RemoveRepo(repo)),
                        ],
                    });
                    cx.notify();
                    return;
                }
                self.context_menu = Some(ContextMenu {
                    position,
                    items: vec![
                        (
                            "New worktree...".into(),
                            MenuAction::NewWorktree(repo.clone()),
                        ),
                        (
                            "New worktree from branch...".into(),
                            MenuAction::PickBranch(repo.clone()),
                        ),
                        (
                            "Clean up merged worktrees...".into(),
                            MenuAction::CleanStale(repo.clone()),
                        ),
                        (
                            "View Git tree".into(),
                            MenuAction::ViewGitTree(repo.clone()),
                        ),
                        ("Refresh".into(), MenuAction::RefreshRepo(repo.clone())),
                        ("Remove from sidebar".into(), MenuAction::RemoveRepo(repo)),
                    ],
                });
            }
            SidebarEvent::NewWorktree(repo) => self.open_sheet(repo, window, cx),
            SidebarEvent::ResumeSessions(picks) => self.resume_sessions(picks, window, cx),
            SidebarEvent::AddRepo => self.add_repo(&AddRepo, window, cx),
            SidebarEvent::AddRepos(paths) => {
                for path in paths {
                    self.register_repo(path, window, cx);
                }
            }
            SidebarEvent::OpenUrl(url) => cx.open_url(&url),
            SidebarEvent::JumpToAgent(path) => self.jump_to_worktree_agent(&path, window, cx),
            SidebarEvent::OpenDiff(path) => self.open_diff(&path, window, cx),
            SidebarEvent::OpenStarred { repo, branch } => {
                let target = self
                    .sidebar
                    .read(cx)
                    .model
                    .branch_worktree(&repo, &branch)
                    .map(|(_, w)| w.path.clone());
                match target {
                    Some(path) => self.open_worktree(&path, window, cx),
                    None => {
                        let name = repo
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| repo.display().to_string());
                        self.notify_repo(format!(
                            "No worktree has {branch} checked out in {name}. Right-click {name} for \"New worktree from branch...\"."
                        ), Severity::Info, &repo, None);
                    }
                }
            }
            SidebarEvent::Unstar { repo, branch } => self.set_starred(&repo, &branch, false, cx),
            SidebarEvent::MoveRepo { from, to } => {
                let order = self.sidebar.update(cx, |s, cx| {
                    let moved = s.model.move_repo(&from, &to);
                    cx.notify();
                    moved.then(|| s.model.repos.iter().map(|r| r.path.clone()).collect())
                });
                if let Some(order) = order {
                    self.config.repos = order;
                    self.save_config();
                }
            }
            SidebarEvent::ToggleActiveLabel => {
                self.config.active_label = match self.config.active_label {
                    chda_config::ActiveLabel::Alias => chda_config::ActiveLabel::Branch,
                    chda_config::ActiveLabel::Branch => chda_config::ActiveLabel::Alias,
                };
                self.save_config();
                self.sync_panes(cx);
            }
            SidebarEvent::ToggleActive => {
                self.config.active_collapsed = !self.config.active_collapsed;
                self.save_config();
                self.sync_panes(cx);
            }
            SidebarEvent::ToggleIdleAgents => {
                self.config.idle_agents_collapsed = !self.config.idle_agents_collapsed;
                self.save_config();
                self.sync_panes(cx);
            }
            SidebarEvent::ToggleProject => {
                self.config.project_collapsed = !self.config.project_collapsed;
                self.save_config();
                self.sync_panes(cx);
                self.refocus_terminal(window, cx);
            }
            SidebarEvent::CloseTab(tab) => self.request_close(CloseTarget::Tab(tab), window, cx),
            SidebarEvent::FocusTab(tab) => {
                if self.ws.activate_tab_id(tab) {
                    self.focus_active(window, cx);
                }
            }
            SidebarEvent::FocusPane(pane) => self.jump_to_pane(pane, window, cx),
            SidebarEvent::ClosePane(pane) => {
                self.request_close(CloseTarget::Pane(pane), window, cx)
            }
        }
        cx.notify();
    }

    /// "Run <agent>" and "Run <preset>" for a folder.
    fn agent_launch_items(&self, path: &Path) -> Vec<(String, MenuAction)> {
        let agents = self.configured_adapters().map(|a| {
            (
                format!("Run {}", a.display_name()),
                MenuAction::RunAgent(path.to_path_buf(), a.id()),
            )
        });
        let presets = self.presets().map(|p| {
            (
                format!("Run {}", p.name),
                MenuAction::RunPreset(path.to_path_buf(), p.name.clone()),
            )
        });
        agents.chain(presets).collect()
    }

    /// Focus a pane already in the worktree, else open a tab there.
    pub(crate) fn open_worktree(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let missing = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(path)
            .is_some_and(|(_, w)| w.missing && w.path == path);
        if missing {
            self.notify_repo(
                format!(
                    "{} no longer exists. Right-click the worktree to remove it.",
                    path.display()
                ),
                Severity::Warning,
                path,
                None,
            );
            cx.notify();
            return;
        }
        let wanted = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(path)
            .map(|(_, w)| w.path.clone());
        let existing = self
            .ws
            .tabs()
            .iter()
            .flat_map(|t| t.panes())
            .filter(|p| {
                self.ws
                    .pane(*p)
                    .and_then(|i| i.cwd.as_ref())
                    .and_then(|cwd| self.sidebar.read(cx).model.worktree_for_path(cwd))
                    .is_some_and(|(_, w)| Some(&w.path) == wanted.as_ref())
            })
            .max_by_key(|p| self.pane_navigation.get(p).copied().unwrap_or(0));
        let local_order = existing
            .and_then(|p| self.pane_navigation.get(&p).copied())
            .unwrap_or(0);
        let other = {
            let registry = self.env.windows.borrow();
            registry
                .entries
                .iter()
                .filter(|e| e.window != window.window_handle())
                .flat_map(|e| {
                    e.panes.iter().filter_map(|(pane, (cwd, order))| {
                        self.sidebar
                            .read(cx)
                            .model
                            .worktree_for_path(cwd)
                            .filter(|(_, w)| Some(&w.path) == wanted.as_ref())
                            .map(|_| (e.clone(), *pane, *order))
                    })
                })
                .max_by_key(|(_, _, order)| *order)
        };
        if let Some((entry, pane, order)) = other
            && (existing.is_none() || order > local_order)
        {
            let focused = entry
                .window
                .update(cx, |_, other_window, cx| {
                    entry
                        .view
                        .update(cx, |view, cx| {
                            if !view.ws.focus_pane(pane) {
                                return false;
                            }
                            other_window.activate_window();
                            view.focus_active(other_window, cx);
                            true
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if focused {
                return;
            }
        }
        if let Some(pane) = existing
            && self.ws.focus_pane(pane)
        {
            self.focus_active(window, cx);
            return;
        }
        self.open_default_action(path, window, cx);
    }

    /// Fetch and bring in the worktree's upstream. A diverged branch is not
    /// touched: the sheet offers a rebase when no local commit was pushed,
    /// a merge otherwise.
    fn update_branch(
        &mut self,
        repo: PathBuf,
        worktree: PathBuf,
        strategy: Option<PullStrategy>,
        cx: &mut Context<Self>,
    ) {
        let strategy = strategy.unwrap_or(self.config.pull);
        let branch = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(&worktree)
            .and_then(|(_, w)| w.branch.clone())
            .unwrap_or_else(|| "the branch".into());
        let notification =
            self.begin_repo_notification(format!("Updating {branch}\u{2026}"), &worktree);
        cx.notify();
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn({
            let worktree = worktree.clone();
            async move { chda_core::update_branch(&worktree, strategy) }
        });
        cx.spawn(async move |this, cx| {
            let _operation = operation;
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                use chda_core::{PullMode, UpdateOutcome};
                let severity = match &result {
                    Ok(UpdateOutcome::UpToDate | UpdateOutcome::Updated { .. }) => Severity::Success,
                    Err(_) => Severity::Error,
                    _ => Severity::Warning,
                };
                let message = match result {
                    Ok(UpdateOutcome::UpToDate) => format!("{branch} is up to date with its upstream"),
                    Ok(UpdateOutcome::Updated { mode, commits }) => match mode {
                        PullMode::FastForward => {
                            format!("Fast-forwarded {branch} by {commits} commit(s)")
                        }
                        PullMode::Rebase => {
                            format!("Rebased {branch} on {commits} new upstream commit(s)")
                        }
                        PullMode::Merge => {
                            format!("Merged {commits} upstream commit(s) into {branch}")
                        }
                    },
                    Ok(UpdateOutcome::Conflicted(mode)) => format!(
                        "{} conflicts; {branch} was left as it was",
                        if mode == PullMode::Rebase {
                            "Rebasing"
                        } else {
                            "Merging"
                        }
                    ),
                    Ok(UpdateOutcome::Diverged {
                        ahead,
                        behind,
                        pushed,
                    }) => {
                        let (line, strategy) = if pushed {
                            (
                                "Some local commits are already on a remote, so they are not rewritten: merge the upstream into the branch.",
                                PullStrategy::Merge,
                            )
                        } else {
                            (
                                "None of the local commits are on a remote: replay them on top of the upstream (rebase).",
                                PullStrategy::Rebase,
                            )
                        };
                        view.confirm = Some(ConfirmSheet {
                            title: format!("{branch} has diverged from its upstream"),
                            lines: vec![
                                format!("{ahead} local commit(s), {behind} new upstream commit(s)."),
                                line.into(),
                            ],
                            action: MenuAction::UpdateBranch {
                                repo: repo.clone(),
                                worktree: worktree.clone(),
                                strategy: Some(strategy),
                            },
                        });
                        format!("{branch} has diverged from its upstream")
                    }
                    Err(e) => format!("Updating {branch}: {e}"),
                };
                view.notify_repo(message, severity, &worktree, Some(notification));
                view.refresh_repo(repo, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn agent_argv(
        &self,
        cwd: &Path,
        agent: AgentId,
        resume: Option<&SessionId>,
    ) -> Option<Vec<String>> {
        let adapter = self.adapters.iter().find(|a| a.id() == agent)?;
        let executable = adapter.executable(&self.env.home(), &self.env.search_path())?;
        self.agent_argv_for_executable(cwd, agent, resume, &executable)
    }

    fn agent_argv_for_executable(
        &self,
        cwd: &Path,
        agent: AgentId,
        resume: Option<&SessionId>,
        executable: &Path,
    ) -> Option<Vec<String>> {
        let adapter = self.adapters.iter().find(|a| a.id() == agent)?;
        let original = adapter.launch_command(cwd, resume, &Self::hook_bin());
        let mut args: Vec<std::ffi::OsString> = original.get_args().map(Into::into).collect();
        if agent == AgentId::Claude
            && let Some(dir) = &self.env.data_dir
            && let Ok(settings) = chda_core::agents::statusline::launch_settings(
                &self.env.home(),
                cwd,
                dir,
                &Self::hook_bin(),
            )
        {
            if let Some(index) = args.iter().position(|arg| arg == "--settings") {
                args.drain(index..(index + 2).min(args.len()));
            }
            args.push("--settings".into());
            args.push(settings.into_os_string());
        }
        let mut cmd = std::process::Command::new(executable);
        cmd.args(args)
            .envs(original.get_envs().filter_map(|(k, v)| v.map(|v| (k, v))))
            .current_dir(cwd);
        Some(chda_core::agents::command_argv(&cmd))
    }

    fn open_default_action(&mut self, cwd: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(agent) = self.config.default_action.agent().and_then(AgentId::parse) {
            self.run_agent(cwd, agent, None, window, cx);
        } else {
            self.open_tab_at(Some(cwd.to_path_buf()), None, window, cx);
        }
    }

    fn run_agent(
        &mut self,
        cwd: &Path,
        agent: AgentId,
        resume: Option<SessionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(adapter) = self.adapters.iter().find(|a| a.id() == agent) else {
            return;
        };
        if !adapter
            .executable(&self.env.home(), &self.env.search_path())
            .is_some()
        {
            self.notify_error(format!("{} is not on PATH", adapter.display_name()));
            cx.notify();
            return;
        }
        if matches!(agent, AgentId::Claude | AgentId::Codex) {
            self.show_agent_launch(cwd, agent, Vec::new(), resume, window, cx);
        } else {
            self.open_managed_launch(cwd, agent, Vec::new(), resume, window, cx);
        }
    }

    /// Start the `agent-presets` entry called `name` in a new tab at `cwd`.
    fn run_preset(&mut self, cwd: &Path, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preset) = self.config.agent_presets.iter().find(|p| p.name == name) else {
            return;
        };
        let adapter = AgentId::parse(&preset.agent)
            .and_then(|id| self.adapters.iter().find(|a| a.id() == id));
        let Some(adapter) = adapter else {
            self.notify_error(format!("Preset {name}: unknown agent {:?}", preset.agent));
            cx.notify();
            return;
        };
        if !adapter
            .executable(&self.env.home(), &self.env.search_path())
            .is_some()
        {
            self.notify_error(format!("{} is not on PATH", adapter.display_name()));
            cx.notify();
            return;
        }
        let agent = adapter.id();
        let options = preset.args.clone();
        if matches!(agent, AgentId::Claude | AgentId::Codex) {
            self.show_agent_launch(cwd, agent, options, None, window, cx);
            return;
        }
        self.open_managed_launch(cwd, agent, options, None, window, cx);
    }

    /// The presets that can run: those naming an agent chda knows.
    fn presets(&self) -> impl Iterator<Item = &chda_config::AgentPreset> {
        self.config.agent_presets.iter().filter(|p| {
            AgentId::parse(&p.agent).is_some_and(|id| self.adapters.iter().any(|a| a.id() == id))
        })
    }

    /// Resume past sessions in one new tab: the first fills it, each next
    /// one splits the largest pane along its longer side.
    pub(crate) fn resume_sessions(
        &mut self,
        picks: Vec<SessionPick>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launch_sheet.is_some() {
            self.notify_error("Finish or cancel the current agent launch first".into());
            cx.notify();
            return;
        }
        if picks.iter().any(|pick| {
            matches!(
                AgentId::parse(&pick.agent),
                Some(AgentId::Claude | AgentId::Codex)
            )
        }) {
            let items = picks
                .into_iter()
                .filter_map(|pick| AgentId::parse(&pick.agent).map(|agent| (agent, pick)))
                .map(|(agent, pick)| {
                    self.launch_item(
                        &pick.worktree,
                        agent,
                        Vec::new(),
                        Some(SessionId(pick.session)),
                    )
                })
                .collect::<Result<Vec<_>, _>>();
            match items {
                Ok(items) if !items.is_empty() => {
                    self.launch_sheet = Some(agent_launch::LaunchSheet { items, error: None });
                    self.notifications_open = false;
                    self.notification_focus = None;
                    window.focus(&self.focus_handle, cx);
                }
                Err(e) => self.notify_error(e),
                _ => {}
            }
            cx.notify();
            return;
        }
        let mut launches = Vec::new();
        let mut missing = Vec::new();
        for pick in picks {
            let adapter = AgentId::parse(&pick.agent)
                .and_then(|id| self.adapters.iter().find(|a| a.id() == id));
            let Some(adapter) = adapter else {
                continue;
            };
            if !adapter
                .executable(&self.env.home(), &self.env.search_path())
                .is_some()
            {
                missing.push(adapter.display_name().to_owned());
                continue;
            }
            match self
                .launch_item(
                    &pick.worktree,
                    adapter.id(),
                    Vec::new(),
                    Some(SessionId(pick.session)),
                )
                .and_then(|item| {
                    self.captured_resume_argv(&item.context)
                        .map(|argv| (item.context, argv))
                }) {
                Ok(launch) => launches.push(launch),
                Err(e) => self.notify_error(e),
            }
        }
        missing.dedup();
        if !missing.is_empty() {
            self.notify_error(format!("{} is not on PATH", missing.join(", ")));
        }
        if launches.is_empty() {
            cx.notify();
            return;
        }
        let aspect = self.tab_aspect(window);
        for (i, (context, argv)) in launches.into_iter().enumerate() {
            let pane = if i == 0 {
                Some(self.ws.new_tab().1)
            } else {
                self.ws.split_largest(aspect)
            };
            if let Some(pane) = pane {
                let cwd = context.cwd.clone();
                self.ws.pane_mut(pane).unwrap().agent_launch = Some(context);
                self.open_pane(pane, Some(cwd), Some(argv), window, cx);
            }
        }
        self.focus_active(window, cx);
    }

    /// Width over height of the area tabs are drawn in.
    fn tab_aspect(&self, window: &Window) -> f32 {
        let size = window.viewport_size();
        let sidebar = if self.sidebar_visible {
            self.sidebar_width(window)
        } else {
            0.0
        };
        let width = (f32::from(size.width) - sidebar).max(1.0);
        width / f32::from(size.height).max(1.0)
    }

    /// Select the existing graph for this repository or open one without a PTY.
    pub(crate) fn open_git_graph(
        &mut self,
        repo: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = repo.canonicalize().unwrap_or(repo);
        let id = self.ws.open_graph(repo.clone());
        if !self.graphs.contains_key(&id) {
            self.create_graph_view(id, repo, cx);
        }
        self.focus_active(window, cx);
        self.sync_panes(cx);
    }

    fn create_graph_view(&mut self, id: TabId, repo: PathBuf, cx: &mut Context<Self>) {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let view = cx.new(|cx| crate::git_graph::GitGraphView::new(repo, bg, fg, cx));
        self.graphs.insert(id, view);
    }

    /// Page the worktree's diff against its base, including uncommitted edits.
    pub(crate) fn open_diff(
        &mut self,
        worktree: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(diff) = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(worktree)
            .filter(|(_, w)| w.path == worktree)
            .and_then(|(_, w)| w.diff.clone())
        else {
            return;
        };
        let command = chda_core::diff_command(&diff.merge_base);
        self.open_tab_at(Some(worktree.to_path_buf()), Some(command), window, cx);
    }

    pub(crate) fn run_menu_action(
        &mut self,
        action: MenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = None;
        match action {
            MenuAction::CopySelection(pane) => {
                if let Some((term, _)) = self.panes.get(&pane) {
                    term.read(cx).copy_selection();
                }
            }
            MenuAction::ShareSelection { pane, identity } => {
                if self.share_identity(pane) == identity {
                    self.open_share_sheet(pane, window, cx);
                } else {
                    self.notify_error(
                        "The selected conversation changed. Select its text again before copying."
                            .into(),
                    );
                }
            }
            MenuAction::StopAndClose { target, focus } => {
                // Restore the original selection if the target disappeared meanwhile.
                self.restore_close_focus(focus, window, cx);
                self.finish_close(target, window, cx);
            }
            MenuAction::OpenTerminal(path) => self.open_tab_at(Some(path), None, window, cx),
            MenuAction::ViewDiff(path) => self.open_diff(&path, window, cx),
            MenuAction::ReviewDiff(path) => self.open_review(&path, window, cx),
            MenuAction::ViewGitTree(repo) => self.open_git_graph(repo, window, cx),
            MenuAction::RunAgent(path, agent) => self.run_agent(&path, agent, None, window, cx),
            MenuAction::RunPreset(path, name) => self.run_preset(&path, &name, window, cx),
            MenuAction::NewWorktree(repo) => self.open_sheet(repo, window, cx),
            MenuAction::NewWorktreeFrom { repo, base } => {
                self.open_sheet_from(repo, Some(base), window, cx)
            }
            MenuAction::EditNote { repo, branch } => self.open_note_sheet(repo, branch, window, cx),
            MenuAction::MergeAndClean {
                repo,
                worktree,
                force,
            } => {
                let entry = self
                    .sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&worktree)
                    .map(|(_, w)| w.clone())
                    .filter(|w| w.path == worktree);
                let Some(entry) = entry else {
                    return;
                };
                if let Some(busy) = &entry.busy {
                    self.notify_repo(
                        format!("{}: already {busy}", entry.path.display()),
                        Severity::Info,
                        &entry.path,
                        None,
                    );
                    cx.notify();
                    return;
                }
                let blockers = chda_core::blockers(&entry);
                if !force && !blockers.is_empty() {
                    let name = entry.branch.clone().unwrap_or_default();
                    self.confirm = Some(ConfirmSheet {
                        title: format!("{name} is not ready to clean up"),
                        lines: blockers
                            .iter()
                            .map(|b| b.to_string())
                            .chain(std::iter::once(
                                "Force: merge anyway, discard changes and delete the branch."
                                    .into(),
                            ))
                            .collect(),
                        action: MenuAction::MergeAndClean {
                            repo,
                            worktree,
                            force: true,
                        },
                    });
                    cx.notify();
                    return;
                }
                let operation = self.env.windows.borrow().update.operation();
                let task = cx.background_spawn({
                    let repo = repo.clone();
                    async move { chda_core::merge_and_clean(&repo, &entry, force) }
                });
                cx.spawn(async move |this, cx| {
                    let _operation = operation;
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        let severity = if result.is_ok() {
                            Severity::Success
                        } else {
                            Severity::Error
                        };
                        let message = match result {
                            Ok(r) => {
                                format!("Merged {} and removed {}", r.branch, r.removed.display())
                            }
                            Err(e) => e.to_string(),
                        };
                        view.notify_repo(message, severity, &worktree, None);
                        view.refresh_repo(repo, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            MenuAction::CleanStale(repo) => {
                let worktrees: Vec<chda_core::WorktreeEntry> = self
                    .sidebar
                    .read(cx)
                    .model
                    .repos
                    .iter()
                    .find(|r| r.path == repo)
                    .map(|r| r.worktrees.clone())
                    .unwrap_or_default();
                let stale: Vec<chda_core::WorktreeEntry> = chda_core::stale_worktrees(&worktrees)
                    .into_iter()
                    .cloned()
                    .collect();
                if stale.is_empty() {
                    self.notify_repo(
                        "No merged, clean worktrees to remove".into(),
                        Severity::Info,
                        &repo,
                        None,
                    );
                    cx.notify();
                    return;
                }
                if self
                    .confirm
                    .as_ref()
                    .is_none_or(|c| !matches!(c.action, MenuAction::CleanStale(_)))
                {
                    self.confirm = Some(ConfirmSheet {
                        title: format!("Remove {} merged worktree(s)?", stale.len()),
                        lines: stale
                            .iter()
                            .map(|w| {
                                format!(
                                    "{}  {}",
                                    w.branch.clone().unwrap_or_default(),
                                    w.path.display()
                                )
                            })
                            .collect(),
                        action: MenuAction::CleanStale(repo),
                    });
                    cx.notify();
                    return;
                }
                let operation = self.env.windows.borrow().update.operation();
                let task = cx.background_spawn({
                    let repo = repo.clone();
                    async move {
                        stale
                            .iter()
                            .map(|w| chda_core::remove_stale(&repo, w).map(|_| w.path.clone()))
                            .collect::<Vec<_>>()
                    }
                });
                cx.spawn(async move |this, cx| {
                    let _operation = operation;
                    let results = task.await;
                    let _ = this.update(cx, |view, cx| {
                        let ok = results.iter().filter(|r| r.is_ok()).count();
                        let errors: Vec<String> = results
                            .iter()
                            .filter_map(|r| r.as_ref().err().map(|e| e.to_string()))
                            .collect();
                        let severity = if errors.is_empty() {
                            Severity::Success
                        } else {
                            Severity::Error
                        };
                        let message = if errors.is_empty() {
                            format!("Removed {ok} worktree(s)")
                        } else {
                            format!("Removed {ok}; failed: {}", errors.join("; "))
                        };
                        view.notify_repo(message, severity, &repo, None);
                        view.refresh_repo(repo, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            MenuAction::DeleteWorktreeAndBranch {
                repo,
                worktree,
                force,
            } => {
                let entry = self
                    .sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&worktree)
                    .map(|(_, w)| w.clone())
                    .filter(|w| w.path == worktree);
                let Some(entry) = entry else {
                    return;
                };
                if let Some(busy) = &entry.busy {
                    self.notify_repo(
                        format!("{}: already {busy}", entry.path.display()),
                        Severity::Info,
                        &entry.path,
                        None,
                    );
                    cx.notify();
                    return;
                }
                if !force {
                    let name = entry.branch.clone().unwrap_or_default();
                    let (title, mut lines) = if entry.safe_to_delete() {
                        (
                            format!("Delete {name}?"),
                            vec![
                                "Already merged into the default branch with nothing uncommitted or unpushed: safe to delete.".to_owned(),
                                format!("Removes {} and the branch.", entry.path.display()),
                            ],
                        )
                    } else {
                        let mut lines: Vec<String> = chda_core::blockers(&entry)
                            .iter()
                            .map(|b| b.to_string())
                            .collect();
                        if !entry.is_merged() {
                            lines.insert(0, "Not merged into the default branch.".into());
                        }
                        lines.push("Force delete discards uncommitted changes and deletes the branch anyway.".into());
                        (format!("Force delete {name}?"), lines)
                    };
                    if entry.missing {
                        lines.insert(0, "The worktree folder is already gone.".into());
                    }
                    match entry.panes.len() {
                        0 => {}
                        1 => lines.push("Closes the terminal open in it.".into()),
                        n => lines.push(format!("Closes the {n} terminals open in it.")),
                    }
                    self.confirm = Some(ConfirmSheet {
                        title,
                        lines,
                        action: MenuAction::DeleteWorktreeAndBranch {
                            repo,
                            worktree,
                            force: true,
                        },
                    });
                    cx.notify();
                    return;
                }
                // A shell left in the folder would keep working in a deleted
                // directory (and can recreate files while it goes).
                self.close_panes(&entry.panes, window, cx);
                self.sidebar.update(cx, |s, cx| {
                    if let Some(w) = s.model.worktree_mut(&worktree) {
                        w.busy = Some("deleting".into());
                        w.error = None;
                    }
                    cx.notify();
                });
                let operation = self.env.windows.borrow().update.operation();
                let task = cx.background_spawn({
                    let repo = repo.clone();
                    async move {
                        let start = std::time::Instant::now();
                        let result = chda_core::delete_worktree_and_branch(&repo, &entry);
                        if let Err(e) = &result {
                            eprintln!(
                                "chda: deleting {} failed after {:?}: {e}",
                                entry.path.display(),
                                start.elapsed()
                            );
                        }
                        result
                    }
                });
                cx.spawn(async move |this, cx| {
                    let _operation = operation;
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        view.sidebar.update(cx, |s, cx| {
                            match &result {
                                Ok(()) => s.model.remove_worktree(&worktree),
                                Err(e) => {
                                    if let Some(w) = s.model.worktree_mut(&worktree) {
                                        w.busy = None;
                                        w.error = Some(e.to_string());
                                    }
                                }
                            }
                            cx.notify();
                        });
                        let severity = if result.is_ok() {
                            Severity::Success
                        } else {
                            Severity::Error
                        };
                        view.notify_repo(
                            match &result {
                                Ok(()) => format!("Deleted {}", worktree.display()),
                                Err(e) => e.to_string(),
                            },
                            severity,
                            &worktree,
                            None,
                        );
                        if result.is_ok() {
                            let repo = repo.clone();
                            let operation = view.env.windows.borrow().update.operation();
                            cx.background_spawn(async move {
                                let _operation = operation;
                                if let Err(e) = chda_core::purge_trash(&repo) {
                                    eprintln!(
                                        "chda: emptying {}'s chda-trash: {e}",
                                        repo.display()
                                    );
                                }
                            })
                            .detach();
                        }
                        view.refresh_repo(repo, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            MenuAction::PickBranch(repo) => {
                let worktrees: Vec<chda_core::WorktreeEntry> = self
                    .sidebar
                    .read(cx)
                    .model
                    .repos
                    .iter()
                    .find(|r| r.path == repo)
                    .map(|r| r.worktrees.clone())
                    .unwrap_or_default();
                let branches = chda_core::unchecked_branches(&repo, &worktrees).unwrap_or_default();
                if branches.is_empty() {
                    self.notify_repo(
                        "Every local branch already has a worktree".into(),
                        Severity::Info,
                        &repo,
                        None,
                    );
                    cx.notify();
                    return;
                }
                let items: Vec<PaletteItem> = branches
                    .into_iter()
                    .map(|b| PaletteItem {
                        label: b.clone(),
                        detail: self
                            .config
                            .worktree_path(&repo, &b)
                            .to_string_lossy()
                            .into_owned(),
                        command: PaletteCommand::CheckoutBranch(repo.clone(), b),
                    })
                    .collect();
                self.open_palette(items, 0, "Check out a branch", window, cx);
            }
            MenuAction::PruneMissing { repo, worktree } => {
                let operation = self.env.windows.borrow().update.operation();
                let task =
                    cx.background_spawn({
                        let repo = repo.clone();
                        async move {
                            chda_core::delete_worktree(&repo, &worktree, false).map(|_| worktree)
                        }
                    });
                cx.spawn(async move |this, cx| {
                    let _operation = operation;
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        let severity = if result.is_ok() {
                            Severity::Success
                        } else {
                            Severity::Error
                        };
                        view.notify_repo(
                            match result {
                                Ok(p) => format!("Removed the record of {}", p.display()),
                                Err(e) => e.to_string(),
                            },
                            severity,
                            &repo,
                            None,
                        );
                        view.refresh_repo(repo, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            MenuAction::Star {
                repo,
                branch,
                starred,
            } => self.set_starred(&repo, &branch, starred, cx),
            MenuAction::RemoveRepo(repo) => {
                if let Some(w) = &mut self.watcher {
                    w.unwatch(&repo);
                }
                self.sidebar.update(cx, |s, cx| {
                    s.model.remove_repo(&repo);
                    cx.notify();
                });
                self.config.repos.retain(|r| *r != repo);
                self.save_config();
            }
            MenuAction::Unavailable(_) => {}
            MenuAction::UpdateBranch {
                repo,
                worktree,
                strategy,
            } => self.update_branch(repo, worktree, strategy, cx),
            MenuAction::RefreshRepo(repo) => {
                self.refresh_repo(repo, cx);
                self.refresh_sessions(cx);
                self.refresh_prs(true, cx);
            }
            MenuAction::OpenPath { path, line, column } => {
                self.open_path(&path, line, column, cx);
            }
            MenuAction::RevealPath(path) => self.env.system.reveal_path(&path, cx),
            MenuAction::PreviewMarkdown(path) => self.preview_markdown(&path, cx),
            MenuAction::InitGit(path) => self.init_git(path, cx),
            MenuAction::DismissUpdate => {
                if let Some(file) = self.release_check_file() {
                    let mut check = ReleaseCheck::load(&file);
                    check.dismiss();
                    if let Err(e) = check.save(&file) {
                        self.notify_error(format!("Could not save the dismissal: {e}"));
                    }
                }
                self.release_notice = None;
                cx.notify();
            }
            MenuAction::PickFolderApp(id) => {
                if self.config.open_in.as_deref() != Some(id.as_str()) {
                    self.config.open_in = Some(id);
                    self.save_config();
                }
                cx.notify();
            }
            MenuAction::OpenWithSystem(path) => self.env.system.open_file(&path, cx),
            MenuAction::CdHere { pane, dir } => {
                if let Some((view, _)) = self.panes.get(&pane) {
                    view.update(cx, |v, _| v.cd(&dir));
                }
            }
            MenuAction::OpenUrl(url) => cx.open_url(&url),
            MenuAction::Copy(text) => cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)),
        }
        cx.notify();
    }

    pub(crate) fn open_sheet(
        &mut self,
        repo: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sheet_from(repo, None, window, cx);
    }

    /// The new worktree sheet; with `base`, the new branch starts there.
    pub(crate) fn open_sheet_from(
        &mut self,
        repo: PathBuf,
        base: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sheet.as_ref().is_some_and(|sheet| sheet.busy) {
            return;
        }
        if self.sidebar.read(cx).model.is_folder(&repo) {
            self.notify_repo(format!(
                "{} is a plain folder. Right-click it and pick \"{INIT_GIT}\" to make worktrees.",
                repo.display()
            ), Severity::Info, &repo, None);
            cx.notify();
            return;
        }
        let suggestion = chda_core::new_branch_name(&repo, |b| self.config.worktree_path(&repo, b));
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let input = cx.new(|cx| TextInput::new("branch name", fg, blend(bg, fg, 0.12), cx));
        let note = cx.new(|cx| {
            let mut note = TextInput::new(
                "note: what is this branch for? (optional)",
                fg,
                blend(bg, fg, 0.12),
                cx,
            );
            note.multiline = true;
            note
        });
        let on_event = |this: &mut Self,
                        event: &TextInputEvent,
                        window: &mut Window,
                        cx: &mut Context<Self>| match event {
            TextInputEvent::Submit(_) => this.create_worktree(window, cx),
            TextInputEvent::Cancel => {
                if this.sheet.as_ref().is_some_and(|s| s.busy) {
                    return;
                }
                this.sheet = None;
                this.focus_active(window, cx);
            }
        };
        let subs = [
            cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
                on_event(this, event, window, cx)
            }),
            cx.subscribe_in(&note, window, move |this, _, event, window, cx| {
                on_event(this, event, window, cx)
            }),
        ];
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.sheet = Some(NewWorktreeSheet {
            repo,
            base,
            suggestion,
            input,
            note,
            busy: false,
            _subs: subs,
            error: None,
        });
        cx.notify();
    }

    /// Tab moves between the branch name and the note.
    fn sheet_tab(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sheet) = &self.sheet else {
            return;
        };
        if event.keystroke.key != "tab" {
            return;
        }
        let on_name = sheet.input.read(cx).focus_handle(cx).is_focused(window);
        let next = if on_name { &sheet.note } else { &sheet.input };
        let handle = next.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.stop_propagation();
    }

    /// The sheet for a branch's note, prefilled with the current one.
    pub(crate) fn open_note_sheet(
        &mut self,
        repo: PathBuf,
        branch: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .filter(|r| r.path == repo)
            .flat_map(|r| r.worktrees.iter())
            .find(|w| w.branch.as_deref() == Some(branch.as_str()))
            .and_then(|w| w.note.clone())
            .unwrap_or_default();
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let input = cx.new(|cx| {
            let mut input = TextInput::new("what is this branch for?", fg, blend(bg, fg, 0.12), cx);
            input.multiline = true;
            input.set_text(&current, cx);
            input
        });
        let sub = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            TextInputEvent::Submit(text) => this.save_note(text.clone(), window, cx),
            TextInputEvent::Cancel => {
                this.note_sheet = None;
                this.focus_active(window, cx);
            }
        });
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.note_sheet = Some(NoteSheet {
            repo,
            branch,
            input,
            _sub: sub,
            error: None,
        });
        cx.notify();
    }

    fn save_note(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = &mut self.note_sheet else {
            return;
        };
        match chda_core::set_note(&sheet.repo, &sheet.branch, &text) {
            Ok(()) => {
                let (repo, branch) = (sheet.repo.clone(), sheet.branch.clone());
                self.note_sheet = None;
                self.show_note(&repo, &branch, &text, cx);
                self.refresh_repo(repo, cx);
                self.focus_active(window, cx);
            }
            Err(e) => sheet.error = Some(e.to_string()),
        }
        cx.notify();
    }

    /// Show a saved note right away, before the refresh reads it back.
    fn show_note(&mut self, repo: &Path, branch: &str, text: &str, cx: &mut Context<Self>) {
        let text = text.trim();
        self.sidebar.update(cx, |s, cx| {
            if let Some(r) = s.model.repo_mut(repo) {
                for w in &mut r.worktrees {
                    if w.branch.as_deref() == Some(branch) {
                        w.note = (!text.is_empty()).then(|| text.to_owned());
                    }
                }
            }
            cx.notify();
        });
    }

    #[cfg(test)]
    pub(crate) fn sheet_error(&self) -> Option<&str> {
        self.sheet.as_ref()?.error.as_deref()
    }

    fn new_worktree_action(
        &mut self,
        _: &NewWorktree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self
            .focused_cwd()
            .and_then(|c| {
                self.sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&c)
                    .map(|(r, _)| r.path.clone())
            })
            .or_else(|| {
                self.sidebar
                    .read(cx)
                    .model
                    .repos
                    .iter()
                    .find(|r| !r.folder)
                    .map(|r| r.path.clone())
            });
        if let Some(repo) = repo {
            self.open_sheet(repo, window, cx);
        }
    }

    fn create_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = &mut self.sheet else {
            return;
        };
        if sheet.busy {
            return;
        }
        sheet.busy = true;
        sheet.error = None;
        let branch = match sheet.input.read(cx).text().trim() {
            "" => sheet.suggestion.clone(),
            typed => typed.to_owned(),
        };
        let note = sheet.note.read(cx).text().trim().to_owned();
        let repo = sheet.repo.clone();
        let base = sheet.base.clone();
        let path = self.config.worktree_path(&repo, &branch);
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn({
            let repo = repo.clone();
            let branch = branch.clone();
            let path = path.clone();
            let note = note.clone();
            async move {
                chda_core::create_worktree(&repo, &branch, &path, base.as_deref()).map(|_| {
                    if note.is_empty() {
                        None
                    } else {
                        chda_core::set_note(&repo, &branch, &note)
                            .err()
                            .map(|e| e.to_string())
                    }
                })
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let _operation = operation;
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                match result {
                    Ok(note_error) => {
                        view.sheet = None;
                        if let Some(e) = note_error {
                            view.notify_repo(
                                format!("Created {branch}, but saving its note failed: {e}"),
                                Severity::Warning,
                                &path,
                                None,
                            );
                        }
                        view.refresh_repo(repo, cx);
                        view.open_default_action(&path, window, cx);
                    }
                    Err(e) => {
                        if let Some(sheet) = &mut view.sheet {
                            sheet.error = Some(e.to_string());
                            sheet.busy = false;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Start editing a tab's title in place.
    fn start_rename(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let input = cx.new(|cx| TextInput::new("tab name", fg, blend(bg, fg, 0.12), cx));
        let sub = cx.subscribe_in(&input, window, move |this, input, event, window, cx| {
            match event {
                TextInputEvent::Submit(title) => {
                    this.ws.rename_tab(tab, title);
                }
                TextInputEvent::Cancel => {}
            }
            let _ = input;
            this.renaming = None;
            this.focus_active(window, cx);
        });
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.renaming = Some((tab, input, sub));
        cx.notify();
    }

    pub(crate) fn palette_items(&self, cx: &App) -> Vec<PaletteItem> {
        let mut items: Vec<PaletteItem> = [
            ("New tab", "cmd-t", "new_tab"),
            ("Close pane", "cmd-w", "close"),
            ("Split right", "cmd-d", "split_right"),
            ("Split down", "cmd-shift-d", "split_down"),
            ("Toggle zoom", "cmd-shift-enter", "zoom"),
            ("Equalize splits", "cmd-ctrl-=", "equalize"),
            ("Toggle sidebar", "cmd-b", "sidebar"),
            ("Add repository...", "cmd-shift-o", "add_repo"),
            ("Rename tab", "double-click the tab", "rename_tab"),
            ("Increase font size", "cmd-=", "font_bigger"),
            ("Decrease font size", "cmd--", "font_smaller"),
            ("Reset font size", "cmd-0", "font_reset"),
            ("Select theme...", "preview with the arrow keys", "theme"),
            ("Go to waiting agent", "cmd-shift-a", "waiting_agent"),
            (
                "View last diagram",
                "Mermaid in this pane's output, rendered offline",
                "view_diagram",
            ),
        ]
        .into_iter()
        .map(|(label, detail, action)| PaletteItem {
            label: label.into(),
            detail: detail.into(),
            command: PaletteCommand::Action(action),
        })
        .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if let Some(folder) = self.open_in_folder(cx) {
            for app in &self.folder_apps {
                items.push(PaletteItem {
                    label: format!("Open in {}", app.name),
                    detail: folder.to_string_lossy().into_owned(),
                    command: PaletteCommand::OpenFolderIn(app.id.clone()),
                });
            }
        }
        if let Some(cwd) = self.focused_cwd() {
            for file in markdown_files(&cwd) {
                items.push(PaletteItem {
                    label: format!(
                        "Preview Markdown: {}",
                        file.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    detail: file.to_string_lossy().into_owned(),
                    command: PaletteCommand::PreviewMarkdown(file),
                });
            }
        }
        let sidebar = &self.sidebar.read(cx).model;
        for repo in &sidebar.repos {
            if !repo.folder {
                items.push(PaletteItem {
                    label: format!("New worktree in {}", repo.name),
                    detail: repo.path.to_string_lossy().into_owned(),
                    command: PaletteCommand::NewWorktree(repo.path.clone()),
                });
            }
            for wt in &repo.worktrees {
                // Where the item acts: `repo/branch`, or a folder's name.
                let place = match (&wt.branch, repo.folder) {
                    (_, true) => repo.name.clone(),
                    (Some(branch), false) => format!("{}/{branch}", repo.name),
                    (None, false) => format!("{}/(detached)", repo.name),
                };
                items.push(PaletteItem {
                    label: format!("Go to {place}"),
                    detail: match &wt.note {
                        // The whole note, so any of its words finds the worktree.
                        Some(note) => format!(
                            "{}  {}",
                            note.split_whitespace().collect::<Vec<_>>().join(" "),
                            wt.path.display()
                        ),
                        None => wt.path.to_string_lossy().into_owned(),
                    },
                    command: PaletteCommand::GoToWorktree(wt.path.clone()),
                });
                for a in self.configured_adapters() {
                    items.push(PaletteItem {
                        label: format!("Run {} in {place}", a.display_name()),
                        detail: wt.path.to_string_lossy().into_owned(),
                        command: PaletteCommand::RunAgent(wt.path.clone(), a.id().as_str().into()),
                    });
                }
                for p in self.presets() {
                    items.push(PaletteItem {
                        label: format!("Run {} in {place}", p.name),
                        detail: std::iter::once(p.agent.as_str())
                            .chain(p.args.iter().map(String::as_str))
                            .collect::<Vec<_>>()
                            .join(" "),
                        command: PaletteCommand::RunPreset(wt.path.clone(), p.name.clone()),
                    });
                }
                for s in wt.sessions.iter().take(5) {
                    items.push(PaletteItem {
                        label: format!(
                            "Resume {} in {place} ({})",
                            self.agent_name(&s.agent),
                            chda_core::relative_age(now, s.last_active_at)
                        ),
                        detail: s.snippet.clone(),
                        command: PaletteCommand::ResumeSession {
                            worktree: wt.path.clone(),
                            agent: s.agent.clone(),
                            session: s.id.clone(),
                        },
                    });
                }
            }
        }
        items
    }

    fn toggle_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            self.restore_theme(cx);
            self.focus_active(window, cx);
            return;
        }
        let items = self.palette_items(cx);
        self.open_palette(items, 0, "Type a command, worktree or session", window, cx);
    }

    /// Pick a theme from the bundled ones and the Ghostty theme directories,
    /// showing each as the selection moves.
    fn select_theme(&mut self, _: &SelectTheme, window: &mut Window, cx: &mut Context<Self>) {
        let ghostty_theme = chda_config::load(&self.ghostty_paths, None).theme;
        let mut items = vec![PaletteItem {
            label: "Follow Ghostty config".into(),
            detail: ghostty_theme.unwrap_or_else(|| "default colors".into()),
            command: PaletteCommand::SetTheme(None),
        }];
        items.extend(
            chda_config::theme_names(&self.ghostty_paths)
                .into_iter()
                .map(|name| PaletteItem {
                    label: name.clone(),
                    detail: String::new(),
                    command: PaletteCommand::SetTheme(Some(name)),
                }),
        );
        let current = PaletteCommand::SetTheme(self.config.theme.clone());
        let selected = items.iter().position(|i| i.command == current).unwrap_or(0);
        self.open_palette(items, selected, "Select a theme", window, cx);
    }

    /// Open `config.toml`, writing the current values first if it does not
    /// exist yet.
    fn open_config(&mut self, _: &OpenConfig, _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.env.config_path.clone() else {
            return;
        };
        if !path.exists() {
            self.save_config();
        }
        self.open_path(&path, None, None, cx);
    }

    /// Open the Ghostty config file Ghostty itself would edit, creating an
    /// empty `config.ghostty` if there is none.
    fn open_ghostty_config(
        &mut self,
        _: &OpenGhosttyConfig,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .ghostty_paths
            .preferred_config_file()
            .map(Path::to_path_buf)
        else {
            return;
        };
        if !path.exists()
            && let Err(e) = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, ""))
        {
            self.notify_error(format!("{}: {e}", path.display()));
            cx.notify();
            return;
        }
        self.open_path(&path, None, None, cx);
    }

    /// Run an agent in the focused pane's worktree, or its directory when it
    /// is in none.
    fn run_agent_here(&mut self, agent: AgentId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cwd) = self.focused_cwd() else {
            return;
        };
        let dir = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(&cwd)
            .map(|(_, w)| w.path.clone())
            .unwrap_or(cwd);
        self.run_agent(&dir, agent, None, window, cx);
    }

    /// The palette's "Resume" entries alone.
    fn resume_session(&mut self, _: &ResumeSession, window: &mut Window, cx: &mut Context<Self>) {
        let items: Vec<PaletteItem> = self
            .palette_items(cx)
            .into_iter()
            .filter(|i| matches!(i.command, PaletteCommand::ResumeSession { .. }))
            .collect();
        if items.is_empty() {
            self.notify("No agent sessions in the sidebar's worktrees yet".into());
            cx.notify();
            return;
        }
        self.open_palette(items, 0, "Resume an agent session", window, cx);
    }

    /// Show `theme` (`None`: the Ghostty config's) without saving it.
    fn show_theme(&mut self, theme: Option<&str>, cx: &mut Context<Self>) {
        let ghostty = chda_config::load(&self.ghostty_paths, theme);
        if !ghostty.problems.is_empty() {
            return;
        }
        let mut settings = Settings::from_ghostty(&ghostty);
        settings.font_size = self.settings.font_size;
        if settings != self.settings {
            self.apply_settings(settings, cx);
        }
        cx.notify();
    }

    fn open_palette(
        &mut self,
        items: Vec<PaletteItem>,
        selected: usize,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let palette = cx.new(|cx| {
            Palette::new(
                items,
                selected,
                placeholder,
                fg,
                blend(bg, fg, 0.08),
                window,
                cx,
            )
        });
        let sub = cx.subscribe_in(&palette, window, |this, _, event, window, cx| {
            match event {
                PaletteEvent::Highlighted(PaletteCommand::SetTheme(theme)) => {
                    this.previewing_theme = true;
                    this.show_theme(theme.as_deref(), cx);
                    return;
                }
                PaletteEvent::Highlighted(_) => return,
                PaletteEvent::Chosen(command) => {
                    this.palette = None;
                    this.run_palette_command(command.clone(), window, cx)
                }
                PaletteEvent::Dismissed => {
                    this.palette = None;
                    this.restore_theme(cx);
                    this.focus_active(window, cx);
                }
            }
            cx.notify();
        });
        self.palette = Some((palette, sub));
        cx.notify();
    }

    /// Undo a theme preview the palette was closed on.
    fn restore_theme(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.previewing_theme) {
            let theme = self.config.theme.clone();
            self.show_theme(theme.as_deref(), cx);
        }
    }

    fn run_palette_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::Action(name) => match name {
                "new_tab" => self.new_tab(&NewTab, window, cx),
                "close" => self.close_surface(&CloseSurface, window, cx),
                "split_right" => self.split(Axis::Horizontal, window, cx),
                "split_down" => self.split(Axis::Vertical, window, cx),
                "zoom" => {
                    self.ws.toggle_zoom();
                    self.focus_active(window, cx);
                }
                "equalize" => {
                    self.ws.equalize();
                    self.focus_active(window, cx);
                }
                "sidebar" => self.toggle_sidebar(&ToggleSidebar, window, cx),
                "add_repo" => self.add_repo(&AddRepo, window, cx),
                "font_bigger" => self.increase_font_size(&IncreaseFontSize, window, cx),
                "font_smaller" => self.decrease_font_size(&DecreaseFontSize, window, cx),
                "font_reset" => self.reset_font_size(&ResetFontSize, window, cx),
                "waiting_agent" => self.go_to_waiting_agent(&GoToWaitingAgent, window, cx),
                "theme" => self.select_theme(&SelectTheme, window, cx),
                "view_diagram" => {
                    if let Some(pane) = self.ws.focused_pane()
                        && let Some((view, _)) = self.panes.get(&pane)
                    {
                        view.read(cx).view_last_diagram();
                    }
                }
                "rename_tab" => {
                    if let Some(tab) = self.ws.active_tab().map(|t| t.id) {
                        self.start_rename(tab, window, cx);
                    }
                }
                _ => {}
            },
            PaletteCommand::GoToWorktree(path) => self.open_worktree(&path, window, cx),
            PaletteCommand::RunAgent(path, agent) => {
                if let Some(agent) = AgentId::parse(&agent) {
                    self.run_agent(&path, agent, None, window, cx);
                }
            }
            PaletteCommand::RunPreset(path, name) => self.run_preset(&path, &name, window, cx),
            PaletteCommand::ResumeSession {
                worktree,
                agent,
                session,
            } => {
                if let Some(agent) = AgentId::parse(&agent) {
                    self.run_agent(&worktree, agent, Some(SessionId(session)), window, cx);
                }
            }
            PaletteCommand::NewWorktree(repo) => self.open_sheet(repo, window, cx),
            PaletteCommand::OpenFolderIn(id) => {
                self.open_folder_in(&id, cx);
                self.focus_active(window, cx);
            }
            PaletteCommand::PreviewMarkdown(file) => {
                self.preview_markdown(&file, cx);
                self.focus_active(window, cx);
            }
            PaletteCommand::SetTheme(theme) => {
                self.previewing_theme = false;
                self.show_theme(theme.as_deref(), cx);
                self.config.theme = theme;
                self.save_config();
                self.focus_active(window, cx);
            }
            PaletteCommand::CheckoutBranch(repo, branch) => {
                let path = self.config.worktree_path(&repo, &branch);
                match chda_core::create_worktree(&repo, &branch, &path, None) {
                    Ok(()) => {
                        self.refresh_repo(repo, cx);
                        self.open_tab_at(Some(path), None, window, cx);
                    }
                    Err(e) => {
                        self.notify_error(e.to_string());
                        cx.notify();
                    }
                }
            }
        }
    }

    fn render_palette(&self) -> Option<AnyElement> {
        let (palette, _) = self.palette.as_ref()?;
        Some(
            deferred(
                div()
                    .absolute()
                    .size_full()
                    .top_0()
                    .left_0()
                    .flex()
                    .items_start()
                    .justify_center()
                    .pt_12()
                    .occlude()
                    .child(palette.clone()),
            )
            .into_any_element(),
        )
    }

    pub(crate) fn confirm_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(confirm) = self.confirm.take() {
            // CleanStale runs when the confirm sheet is still set; re-arm it
            // so the action sees the confirmation.
            if matches!(confirm.action, MenuAction::CleanStale(_)) {
                let action = confirm.action.clone();
                self.confirm = Some(confirm);
                self.run_menu_action(action, window, cx);
                self.confirm = None;
            } else {
                self.run_menu_action(confirm.action, window, cx);
            }
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.share_sheet.is_some() {
            self.close_share_sheet(window, cx);
            return;
        }
        if self.child_detail.is_some() {
            self.close_child_details(window, cx);
            return;
        }
        if self.notifications_open {
            self.close_notifications(window, cx);
            return;
        }
        if self.sheet.as_ref().is_some_and(|sheet| sheet.busy) {
            return;
        }
        if self.confirm.is_some() {
            self.cancel_confirm(window, cx);
            return;
        }
        if self.sheet.take().is_some()
            || self.launch_sheet.take().is_some()
            || self.note_sheet.take().is_some()
            || self.palette.take().is_some()
            || self.context_menu.take().is_some()
        {
            self.restore_theme(cx);
            self.focus_active(window, cx);
            cx.notify();
        } else {
            // No overlay handled Esc: let the focused terminal encode and
            // deliver it to the running application (for example, cancel a turn).
            cx.propagate();
        }
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tabs = self.ws.tabs_in(self.tab_group.as_deref());
        if tabs.is_empty() {
            return None;
        }
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let active = self.ws.active_index_in(self.tab_group.as_deref());
        let bar = div()
            .flex()
            .flex_row()
            .w_full()
            .id("tab-bar")
            .overflow_x_scroll()
            .flex_shrink_0()
            .bg(blend(bg, fg, 0.06))
            .text_sm()
            .text_color(fg)
            .children(tabs.into_iter().enumerate().map(|(i, tab)| {
                let title = self.ws.tab_title(tab);
                let bell = tab
                    .focused_pane()
                    .and_then(|pane| self.ws.pane(pane))
                    .is_some_and(|p| p.bell);
                let is_active = active == Some(i);
                let tab_id = tab.id;
                let close = div()
                    .id(("tab-close", i))
                    .debug_selector(move || format!("tab-close-{i}"))
                    .size(px(20.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(fg.opacity(0.15)))
                    .tooltip(crate::tooltip::text(format!("Close {title}")))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.request_close(CloseTarget::Tab(tab_id), window, cx);
                    }))
                    .child("×");
                if let Some((id, input, _)) = &self.renaming
                    && *id == tab.id
                {
                    return div()
                        .id(("tab", i))
                        .px_2()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap_1()
                        .bg(bg)
                        .child(div().flex_1().min_w_0().child(input.clone()))
                        .child(close);
                }
                let agent = self.ws.tab_agent(tab).cloned();
                let tip = agent.as_ref().map(|a| {
                    let name = self.agent_name(&a.agent);
                    match a.status {
                        AgentStatus::Working => format!("{name} is working"),
                        AgentStatus::WaitingInput => format!("{name} is waiting for input"),
                        AgentStatus::Review => format!("{name} finished; not looked at yet"),
                        AgentStatus::Idle => format!("{name} is idle; ready for another task"),
                    }
                });
                let dot = agent.map(|a| crate::status_icon::status_icon(Some(a.status), self.spin));
                div()
                    .id(("tab", i))
                    .debug_selector(move || format!("tab-{i}"))
                    .min_w(px(104.0))
                    .flex_shrink_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .max_w(px(260.0))
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when_some(tip, |d, tip| d.tooltip(crate::tooltip::text(tip)))
                    .when(is_active, |d| d.bg(bg))
                    .when(!is_active, |d| d.text_color(fg.opacity(0.6)))
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        if e.click_count() >= 2 {
                            this.start_rename(tab_id, window, cx);
                        } else {
                            this.goto_tab(i, window, cx);
                        }
                    }))
                    .children(dot)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(format!(
                                "{}{}  {}",
                                if bell { "\u{1f514} " } else { "" },
                                i + 1,
                                title
                            )),
                    )
                    .child(close)
            }));
        Some(bar.into_any_element())
    }

    fn render_node(&self, node: &Node, divider: Hsla, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Leaf(pane)
                if self
                    .ws
                    .pane(*pane)
                    .is_some_and(|info| info.managed_run.is_some()) =>
            {
                self.render_managed_pane(*pane, cx)
            }
            Node::Leaf(pane) => match self.panes.get(pane) {
                // Cached: redrawing the window (tab bar spinner, another
                // pane's output) reuses a pane's last frame unless it changed.
                Some((view, _)) => div()
                    .size_full()
                    .child(
                        AnyView::from(view.clone()).cached(StyleRefinement::default().size_full()),
                    )
                    .into_any_element(),
                None => div().size_full().into_any_element(),
            },
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.render_node(first, divider, cx);
                let second = self.render_node(second, divider, cx);
                let (container, first_box, line) = match axis {
                    Axis::Horizontal => (
                        div().flex_row(),
                        div().h_full().w(relative(*ratio)),
                        div().h_full().w(px(1.0)),
                    ),
                    Axis::Vertical => (
                        div().flex_col(),
                        div().w_full().h(relative(*ratio)),
                        div().w_full().h(px(1.0)),
                    ),
                };
                container
                    .flex()
                    .size_full()
                    .child(first_box.min_w_0().min_h_0().overflow_hidden().child(first))
                    .child(line.flex_shrink_0().bg(divider))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(second),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let items = menu.items.clone();
        let list = div()
            .flex()
            .flex_col()
            .min_w(px(180.0))
            .py_1()
            .rounded_md()
            .bg(blend(bg, fg, 0.1))
            .border_1()
            .border_color(fg.opacity(0.2))
            .text_sm()
            .text_color(fg)
            .shadow_md()
            .occlude()
            .children(items.into_iter().enumerate().map(|(i, (label, action))| {
                let item = div().id(("menu", i)).px_3().py_1();
                match action {
                    MenuAction::Unavailable(reason) => item
                        .text_color(fg.opacity(0.45))
                        .child(label)
                        .child(div().text_xs().child(reason)),
                    MenuAction::PickFolderApp(ref id) => {
                        let app = self.folder_apps.iter().find(|a| &a.id == id);
                        let current = self.folder_app().is_some_and(|a| &a.id == id);
                        item.flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .hover(|s| s.bg(fg.opacity(0.12)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_menu_action(action.clone(), window, cx)
                            }))
                            .children(
                                app.and_then(|a| a.icon.clone())
                                    .map(|i| img(i).size(px(16.0)).flex_shrink_0()),
                            )
                            .child(div().flex_1().child(label))
                            .when(current, |d| d.child("\u{2713}"))
                    }
                    action => item
                        .cursor_pointer()
                        .hover(|s| s.bg(fg.opacity(0.12)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_menu_action(action.clone(), window, cx)
                        }))
                        .child(label),
                }
            }));
        Some(
            deferred(
                // Blocks what is under the menu, so hovered rows show no
                // tooltip over it and a click outside only dismisses it.
                div()
                    .absolute()
                    .size_full()
                    .top_0()
                    .left_0()
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.context_menu = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| {
                            this.context_menu = None;
                            cx.notify();
                        }),
                    )
                    .child(
                        anchored()
                            .position(menu.position)
                            .snap_to_window()
                            .child(list),
                    ),
            )
            .into_any_element(),
        )
    }

    fn render_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.confirm.as_ref()?;
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let destructive = matches!(
            confirm.action,
            MenuAction::StopAndClose { .. } | MenuAction::DeleteWorktreeAndBranch { .. }
        );
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .id("confirm-sessions")
                    .max_h(px(200.0))
                    .overflow_y_scroll()
                    .children(confirm.lines.iter().map(|line| {
                        div()
                            .text_sm()
                            .text_color(fg.opacity(0.8))
                            .child(line.clone())
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        div()
                            .id("confirm-cancel")
                            .debug_selector(|| "confirm-cancel".into())
                            .px_3()
                            .py_2()
                            .rounded_sm()
                            .border_1()
                            .border_color(fg.opacity(0.2))
                            .cursor_pointer()
                            .hover(|s| s.bg(fg.opacity(0.1)))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel_confirm(window, cx)),
                            )
                            .child("Cancel"),
                    )
                    .child(
                        div()
                            .id("confirm-ok")
                            .debug_selector(|| "confirm-ok".into())
                            .px_3()
                            .py_2()
                            .rounded_sm()
                            .bg(if destructive {
                                gpui::rgb(0xf38ba8).into()
                            } else {
                                fg
                            })
                            .text_color(hsla(self.settings.colors.background.unwrap_or_default()))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.confirm_action(window, cx)),
                            )
                            .child(
                                if matches!(confirm.action, MenuAction::StopAndClose { .. }) {
                                    "Stop and close"
                                } else if destructive {
                                    "Delete worktree"
                                } else {
                                    "Proceed"
                                },
                            ),
                    ),
            );
        Some(self.sheet_frame(confirm.title.clone(), body, None, "Esc cancel"))
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.sheet.as_ref()?;
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let typed = sheet.input.read(cx).text().trim();
        let branch = if typed.is_empty() {
            &sheet.suggestion
        } else {
            typed
        };
        let path = self.config.worktree_path(&sheet.repo, branch);
        let repo_name = sheet.repo.file_name().unwrap_or_default().to_string_lossy();
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(cx.listener(Self::sheet_tab))
            .child(div().text_sm().text_color(fg.opacity(0.65)).child(format!(
                "{repo_name}  ·  from {}",
                sheet.base.as_deref().unwrap_or("HEAD")
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("Branch name"),
                    )
                    .child(sheet.input.clone()),
            )
            .when(typed.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "branch-hint".into())
                        .text_xs()
                        .text_color(fg.opacity(0.6))
                        .child(format!("Leave blank to use {}", sheet.suggestion)),
                )
            })
            .children(sheet.error.clone().map(|e| {
                div()
                    .id("branch-error")
                    .text_xs()
                    .text_color(gpui::rgb(0xf38ba8))
                    .child(format!("! {e}"))
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child("Note · optional")
                    .child(div().min_h(px(72.0)).child(sheet.note.clone())),
            )
            .child(
                div()
                    .p_2()
                    .rounded_sm()
                    .bg(fg.opacity(0.05))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(fg.opacity(0.6))
                            .child("DESTINATION"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(path.display().to_string().replace('/', "/\u{200b}")),
                    ),
            )
            .child(self.form_actions(false, sheet.busy, cx));
        Some(self.sheet_frame(
            "New worktree".into(),
            body,
            None,
            "↵ Create   ·   Esc Cancel   ·   Tab Next field   ·   Shift ↵ New line",
        ))
    }

    fn render_note_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.note_sheet.as_ref()?;
        Some(
            self.sheet_frame(
                "Edit branch note".into(),
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_sm().child(format!(
                        "{}  ·  {}",
                        sheet.repo.file_name().unwrap_or_default().to_string_lossy(),
                        sheet.branch
                    )))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child("Note")
                            .child(div().min_h(px(100.0)).child(sheet.input.clone())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .child("The first nonempty line names this task."),
                    )
                    .child(self.form_actions(true, false, cx)),
                sheet.error.clone(),
                "↵ Save   ·   Esc Cancel   ·   Shift ↵ New line",
            ),
        )
    }

    fn form_actions(&self, note: bool, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        div()
            .flex()
            .gap_2()
            .justify_end()
            .pt_2()
            .child(
                div()
                    .id("form-cancel")
                    .debug_selector(|| "form-cancel".into())
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .bg(fg.opacity(0.08))
                    .cursor_pointer()
                    .when(busy, |d| d.opacity(0.4))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !busy && this.sheet.as_ref().is_none_or(|sheet| !sheet.busy) {
                            if note {
                                this.note_sheet = None;
                            } else {
                                this.sheet = None;
                            }
                            this.focus_active(window, cx);
                        }
                    }))
                    .child("Cancel"),
            )
            .child(
                div()
                    .id("form-submit")
                    .debug_selector(|| "form-submit".into())
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(fg.opacity(0.45))
                    .bg(fg.opacity(0.12))
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.2)))
                    .when(busy, |d| d.opacity(0.5))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !busy && this.sheet.as_ref().is_none_or(|sheet| !sheet.busy) {
                            if note {
                                if let Some(sheet) = &this.note_sheet {
                                    let text = sheet.input.read(cx).text().to_owned();
                                    this.save_note(text, window, cx);
                                }
                            } else {
                                this.create_worktree(window, cx);
                            }
                        }
                    }))
                    .child(if busy {
                        "Creating…"
                    } else if note {
                        "Save note"
                    } else {
                        "Create worktree"
                    }),
            )
            .into_any_element()
    }

    /// A modal panel near the top of the window.
    fn sheet_frame(
        &self,
        title: String,
        body: gpui::Div,
        error: Option<String>,
        hint: &'static str,
    ) -> AnyElement {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        deferred(
            div()
                .absolute()
                .size_full()
                .top_0()
                .left_0()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .bg(gpui::black().opacity(0.3))
                .occlude()
                .child(
                    div()
                        .id("form-panel")
                        .debug_selector(|| "form-panel".into())
                        .w(px(480.0))
                        .max_w(relative(1.0))
                        .max_h(relative(0.9))
                        .overflow_y_scroll()
                        .p_4()
                        .rounded_md()
                        .bg(blend(bg, fg, 0.08))
                        .border_1()
                        .border_color(fg.opacity(0.2))
                        .shadow_lg()
                        .text_sm()
                        .text_color(fg)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(title),
                        )
                        .child(body)
                        .children(
                            error.map(|e| div().text_xs().text_color(gpui::rgb(0xf38ba8)).child(e)),
                        )
                        .child(div().text_xs().text_color(fg.opacity(0.5)).child(hint)),
                ),
        )
        .into_any_element()
    }
}

/// Mix `a` towards `b` by `t`.
/// Text in the title bar, nudged down so its ink, not its line box, sits on
/// the window buttons' center line: the UI font's ascent is taller than its
/// descent, which leaves centered text about a point high.
fn optical(text: impl IntoElement) -> gpui::Div {
    div().relative().top(px(1.0)).child(text)
}

/// Markdown files directly in `dir`, sorted, at most 20: the palette offers
/// to preview them.
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && crate::markdown_preview::is_markdown(p))
        .collect();
    files.sort();
    files.truncate(20);
    files
}

pub fn blend(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    gpui::Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
    .into()
}

#[cfg(test)]
impl WorkspaceView {
    /// Shell startup has reached the workspace, beyond the visible VT text.
    pub(crate) fn focused_prompt_ready(&self, cx: &App) -> bool {
        self.ws
            .focused_pane()
            .is_some_and(|p| !self.initializing_panes.contains(&p))
            && self.focused_text(cx).contains("test%")
    }

    /// The focused pane's visible text, rows joined with newlines.
    pub(crate) fn focused_text(&self, cx: &App) -> String {
        self.ws
            .focused_pane()
            .map(|p| self.pane_text(p, cx))
            .unwrap_or_default()
    }

    /// A pane's visible text, rows joined with newlines.
    pub(crate) fn pane_text(&self, pane: PaneId, cx: &App) -> String {
        let Some((view, _)) = self.panes.get(&pane) else {
            return String::new();
        };
        let frame = view.read(cx).frame();
        (0..frame.size.rows)
            .map(|y| frame.row_text(y))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.env.windows.borrow().update.frozen {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_center()
                .bg(hsla(self.settings.colors.background.unwrap_or_default()))
                .text_color(hsla(self.settings.colors.foreground.unwrap_or_default()))
                .child(match &self.env.windows.borrow().update.progress {
                    chda_core::self_update::UpdateProgress::Failed { message } => message.clone(),
                    progress => progress.label(),
                })
                .into_any_element();
        }
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let divider = blend(bg, fg, 0.2);
        let content = match self.ws.active_tab() {
            Some(tab) => match &tab.content {
                chda_core::TabContent::Terminal(terminal) => match terminal.zoomed {
                    Some(pane) => self.render_node(&Node::Leaf(pane), divider, cx),
                    None => self.render_node(&terminal.root, divider, cx),
                },
                chda_core::TabContent::GitGraph { .. } => self
                    .graphs
                    .get(&tab.id)
                    .map(|graph| graph.clone().into_any_element())
                    .unwrap_or_else(|| div().into_any_element()),
                chda_core::TabContent::DiffReview { .. } => self
                    .reviews
                    .get(&tab.id)
                    .map(|(view, _)| view.clone().into_any_element())
                    .unwrap_or_else(|| div().into_any_element()),
            },
            None => div().into_any_element(),
        };
        let sidebar_width = px(self.sidebar_width(window));
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .children(self.render_tab_bar(cx))
            .child(div().flex_1().min_h_0().w_full().child(content));
        let title_bar = self.render_title_bar(window, cx);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(bg)
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.launch_sheet.is_some()
                    && event.keystroke.key == "enter"
                    && !event.keystroke.modifiers.modified()
                {
                    this.start_agent_launch(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(|this, _: &NewWindow, _, cx| {
                let ghostty = chda_config::load(&this.env.ghostty, this.config.theme.as_deref());
                crate::open_workspace_window(ghostty, None, this.env.clone(), cx);
            }))
            .on_action(cx.listener(Self::close_surface))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::add_repo))
            .on_action(cx.listener(Self::new_worktree_action))
            .on_action(cx.listener(Self::toggle_palette))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::increase_font_size))
            .on_action(cx.listener(Self::decrease_font_size))
            .on_action(cx.listener(Self::reset_font_size))
            .on_action(cx.listener(Self::go_to_waiting_agent))
            .on_action(cx.listener(Self::select_theme))
            .on_action(cx.listener(Self::open_config))
            .on_action(cx.listener(Self::open_ghostty_config))
            .on_action(cx.listener(Self::resume_session))
            .on_action(
                cx.listener(|this, _: &ReloadConfig, window, cx| this.reload_config(window, cx)),
            )
            .on_action(cx.listener(|this, _: &RunClaude, window, cx| {
                this.run_agent_here(AgentId::Claude, window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunCodex, window, cx| {
                this.run_agent_here(AgentId::Codex, window, cx)
            }))
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &ZoomWindow, window, _| window.zoom_window())
            .on_action(cx.listener(|this, _: &NextTab, w, cx| {
                this.ws.cycle_tab_in_group(true);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, w, cx| {
                this.ws.cycle_tab_in_group(false);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &GotoTab1, w, cx| this.goto_tab(0, w, cx)))
            .on_action(cx.listener(|this, _: &GotoTab2, w, cx| this.goto_tab(1, w, cx)))
            .on_action(cx.listener(|this, _: &GotoTab3, w, cx| this.goto_tab(2, w, cx)))
            .on_action(cx.listener(|this, _: &GotoTab4, w, cx| this.goto_tab(3, w, cx)))
            .on_action(cx.listener(|this, _: &GotoTab5, w, cx| this.goto_tab(4, w, cx)))
            .on_action(cx.listener(|this, _: &GotoTab6, w, cx| this.goto_tab(5, w, cx)))
            .on_action(cx.listener(|this, _: &GotoTab7, w, cx| this.goto_tab(6, w, cx)))
            .on_action(cx.listener(|this, _: &GotoTab8, w, cx| this.goto_tab(7, w, cx)))
            .on_action(cx.listener(|this, _: &LastTab, w, cx| {
                let group = this.tab_group.clone();
                let last = this.ws.tabs_in(group.as_deref()).len().saturating_sub(1);
                this.goto_tab(last, w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SplitRight, w, cx| this.split(Axis::Horizontal, w, cx)),
            )
            .on_action(cx.listener(|this, _: &SplitDown, w, cx| this.split(Axis::Vertical, w, cx)))
            .on_action(cx.listener(|this, _: &FocusLeft, w, cx| {
                this.ws.focus_direction(Direction::Left);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusRight, w, cx| {
                this.ws.focus_direction(Direction::Right);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusUp, w, cx| {
                this.ws.focus_direction(Direction::Up);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusDown, w, cx| {
                this.ws.focus_direction(Direction::Down);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &NextSplit, w, cx| {
                this.ws.cycle_pane(true);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevSplit, w, cx| {
                this.ws.cycle_pane(false);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &ResizeLeft, _, cx| {
                this.ws.resize(Direction::Left, RESIZE_STEP);
                this.save_session();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ResizeRight, _, cx| {
                this.ws.resize(Direction::Right, RESIZE_STEP);
                this.save_session();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ResizeUp, _, cx| {
                this.ws.resize(Direction::Up, RESIZE_STEP);
                this.save_session();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ResizeDown, _, cx| {
                this.ws.resize(Direction::Down, RESIZE_STEP);
                this.save_session();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &EqualizeSplits, _, cx| {
                this.ws.equalize();
                this.save_session();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleZoom, w, cx| {
                this.ws.toggle_zoom();
                this.focus_active(w, cx);
            }))
            .child(title_bar)
            .when(window.viewport_size().width < px(700.0), |d| {
                d.children(self.render_update_controls(cx))
            })
            .when_some(
                match &self.env.windows.borrow().update.progress {
                    chda_core::self_update::UpdateProgress::Failed { message } => {
                        Some(message.clone())
                    }
                    _ => None,
                },
                |d, message| {
                    d.child(
                        div()
                            .p_2()
                            .text_color(fg)
                            .bg(fg.opacity(0.08))
                            .child(message),
                    )
                },
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_row()
                    .when(self.sidebar_visible, |d| {
                        d.child(
                            div()
                                .w(sidebar_width)
                                .h_full()
                                .flex_shrink_0()
                                .relative()
                                .flex()
                                .flex_col()
                                .border_r_1()
                                .border_color(divider)
                                .child(
                                    div().flex_1().min_h_0().child(
                                        AnyView::from(self.sidebar.clone())
                                            .cached(StyleRefinement::default().size_full()),
                                    ),
                                )
                                .child(self.render_sidebar_grip(divider, cx)),
                        )
                    })
                    .child(main),
            )
            .when(self.sidebar_drag, |d| {
                // Over everything while the border is dragged, so the panes
                // do not take the drag for a text selection.
                d.child(
                    div()
                        .id("sidebar-drag")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .cursor_col_resize()
                        .on_mouse_move(cx.listener(Self::drag_sidebar))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.end_sidebar_drag(cx)),
                        ),
                )
            })
            .children(self.render_context_menu(cx))
            .child(self.status_bar.render(self, window, cx))
            .children(self.status_bar.details(self, cx))
            .children(self.render_sheet(cx))
            .children(self.render_agent_launch(cx))
            .children(self.render_note_sheet(cx))
            .children(self.render_confirm(cx))
            .children(self.render_palette())
            .children(self.render_notifications(cx))
            .children(self.render_child_details(cx))
            .children(self.render_share_sheet(cx))
            .into_any_element()
    }
}

/// A worktree as `chda mcp`'s `list_worktrees` reports it.
fn worktree_json(repo: &chda_core::RepoEntry, w: &chda_core::WorktreeEntry) -> serde_json::Value {
    let pr = w.pr.as_ref().map(|p| {
        serde_json::json!({
            "number": p.number,
            "url": p.url,
            "state": format!("{:?}", p.state).to_lowercase(),
        })
    });
    let agents: serde_json::Map<String, serde_json::Value> = w
        .agents
        .iter()
        .map(|(id, status)| (id.clone(), format!("{status:?}").to_lowercase().into()))
        .collect();
    serde_json::json!({
        "repo": repo.path,
        "branch": w.branch,
        "path": w.path,
        "main": w.is_main,
        "note": w.note,
        "uncommitted": w.badges.dirty_count(),
        "ahead": w.badges.ahead,
        "behind": w.badges.behind,
        "merged": w.is_merged(),
        "pull_request": pr,
        "agents": agents,
    })
}
