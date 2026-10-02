//! The window's root view: sidebar, tab bar and the split tree of panes.
//! It also owns the agent hook receiver and the sidebar refresh schedule.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chda_config::{ChdaConfig, GhosttyConfig, Paths, PullStrategy, TabTitle};

use crate::environment::Environment;
use chda_core::agents::{AgentAdapter, AgentId, HookEvent, HookKind, SessionCache, SessionId, ipc};
use chda_core::{
    ActiveTab, AgentEvent, AgentSessionRef, AgentStatus, Axis, Direction, FileWatcher, Node,
    PaneId, RepoWatcher, SavedBounds, SavedWindow, TabId, TitleMode, Workspace,
};
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, MouseButton, PathPromptOptions,
    Pixels, Point, Render, Subscription, Window, actions, anchored, deferred, div, prelude::*, px,
    relative,
};

use crate::palette::{Palette, PaletteCommand, PaletteEvent, PaletteItem};
use crate::platform;
use crate::settings::Settings;
use crate::sidebar_view::{AgentLabel, SessionPick, SidebarEvent, SidebarView};
use crate::terminal_element::hsla;
use crate::terminal_view::{TerminalEvent, TerminalView};
use crate::text_input::{TextInput, TextInputEvent};

actions!(
    workspace,
    [
        NewTab,
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
/// Font size bounds and step for the runtime font size actions (Ghostty's).
const FONT_SIZE_STEP: f32 = 1.0;
const FONT_SIZE_MIN: f32 = 4.0;
const FONT_SIZE_MAX: f32 = 255.0;
/// Safety-net refresh while the window is active; the real triggers are
/// shell prompts, `.git` changes and agent events.
const STATUS_REFRESH: Duration = Duration::from_secs(60);
/// How often session transcripts are re-indexed while the window is active.
const SESSION_REFRESH: Duration = Duration::from_secs(120);
/// Step of the "working" dot pulse; the timer only runs while an agent works.
const PULSE_STEP: Duration = Duration::from_millis(250);
const PULSE_STEPS: u8 = 8;

/// A popup menu anchored at a window position.
pub(crate) struct ContextMenu {
    position: Point<Pixels>,
    pub(crate) items: Vec<(String, MenuAction)>,
}

#[derive(Clone, Debug)]
pub(crate) enum MenuAction {
    OpenTerminal(PathBuf),
    /// A read-only tab with the worktree's diff against its base.
    ViewDiff(PathBuf),
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
    focus_handle: FocusHandle,
    pub(crate) sidebar: Entity<SidebarView>,
    _sidebar_sub: Subscription,
    sidebar_visible: bool,
    adapters: Arc<Vec<Box<dyn AgentAdapter>>>,
    session_cache: Arc<Mutex<SessionCache>>,
    hook_events: Option<mpsc::Receiver<HookEvent>>,
    refreshing: HashSet<PathBuf>,
    refresh_again: HashSet<PathBuf>,
    watcher: Option<RepoWatcher>,
    watch_events: Option<mpsc::Receiver<PathBuf>>,
    pub(crate) context_menu: Option<ContextMenu>,
    sheet: Option<NewWorktreeSheet>,
    note_sheet: Option<NoteSheet>,
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
    /// Message shown briefly at the bottom of the sidebar.
    pub(crate) status_line: Option<String>,
    /// Where the Ghostty config lives, and the files the last load read.
    ghostty_paths: Paths,
    ghostty_sources: Vec<PathBuf>,
    /// Watches the Ghostty config files and `config.toml`.
    config_watcher: Option<FileWatcher>,
    /// The last reload found a broken config; its message is on the status line.
    config_problem: Option<String>,
    /// The theme palette shows a theme that is not the configured one yet.
    previewing_theme: bool,
    /// Phase of the "working" dot pulse, and whether its timer runs.
    pulse: u8,
    pulsing: bool,
    /// The pane "go to waiting agent" jumped to last, to cycle onwards.
    last_jump: Option<PaneId>,
    /// Window frame, kept for session restore.
    bounds: Option<SavedBounds>,
    /// The app is quitting: panes going away must not shrink the saved
    /// session.
    quitting: bool,
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
        let hook_events = env
            .data_dir
            .clone()
            .and_then(|d| Self::start_hook_receiver(d, window, cx));
        let (watcher, watch_events) = Self::start_watcher(window, cx);

        let mut ws = Workspace::new();
        ws.title_mode = match config.tab_title {
            TabTitle::Branch => TitleMode::Branch,
            TabTitle::Path => TitleMode::Path,
        };
        let mut this = Self {
            sidebar_visible: config.sidebar_visible,
            configured_font_size: settings.font_size,
            settings,
            config,
            ws,
            panes: HashMap::new(),
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
            hook_events,
            refreshing: HashSet::new(),
            refresh_again: HashSet::new(),
            watcher,
            watch_events,
            context_menu: None,
            sheet: None,
            note_sheet: None,
            confirm: None,
            palette: None,
            renaming: None,
            tab_group: None,
            forges: None,
            pr_fetched: HashMap::new(),
            base_fetched: HashMap::new(),
            status_line: None,
            ghostty_paths: env.ghostty.clone(),
            ghostty_sources: ghostty.sources,
            config_watcher: None,
            config_problem: None,
            previewing_theme: false,
            pulse: 0,
            pulsing: false,
            last_jump: None,
            bounds: None,
            quitting: false,
            env,
        };
        this.note_bounds(window);
        cx.observe_window_bounds(window, |this, window, _| {
            this.note_bounds(window);
            this.save_session();
        })
        .detach();
        cx.on_app_quit(|this, _| {
            this.save_session();
            this.quitting = true;
            async {}
        })
        .detach();
        this.start_notification_clicks(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.reviewed_focused(cx);
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
        this.install_hooks();
        match saved {
            Some(saved) => this.restore_session(&saved, window, cx),
            None => this.new_tab(&NewTab, window, cx),
        }
        this.refresh_all(cx);
        this.refresh_sessions(cx);
        Self::schedule_refreshes(window, cx);
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
        data_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<mpsc::Receiver<HookEvent>> {
        let (tx, rx) = mpsc::channel();
        let (wake_tx, mut wake_rx) = unbounded::<()>();
        let socket = ipc::socket_path(&data_dir);
        ipc::serve(&socket, tx.clone(), move || {
            let _ = wake_tx.unbounded_send(());
        })
        .ok()?;
        // Events logged while no app was running.
        let log = data_dir.join("events.jsonl");
        if let Ok(text) = std::fs::read_to_string(&log) {
            for line in text.lines() {
                if let Ok(ev) = serde_json::from_str::<HookEvent>(line) {
                    let _ = tx.send(ev);
                }
            }
            let _ = std::fs::remove_file(&log);
        }
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
        Some(rx)
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
        if problem.is_some() {
            self.status_line = problem.clone();
        } else if self.config_problem.is_some() && self.status_line == self.config_problem {
            self.status_line = None;
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
        if self.sidebar_visible && !sidebar_was_visible {
            self.refresh_all(cx);
        }
        self.sync_panes(cx);
        self.sync_title(window);
        self.sidebar.update(cx, |_, cx| cx.notify());
    }

    fn watch_repo(&mut self, repo: &Path) {
        if let Some(w) = &mut self.watcher {
            let _ = w.watch(repo);
        }
    }

    fn drain_hook_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rx) = &self.hook_events else {
            return;
        };
        let events: Vec<HookEvent> = rx.try_iter().collect();
        for ev in events {
            self.apply_hook_event(ev, window, cx);
        }
    }

    fn apply_hook_event(&mut self, ev: HookEvent, window: &mut Window, cx: &mut Context<Self>) {
        let event = match ev.kind {
            HookKind::SessionStart => AgentEvent::SessionStart,
            HookKind::PromptSubmitted => AgentEvent::PromptSubmitted,
            HookKind::WaitingInput => AgentEvent::WaitingInput,
            HookKind::Stopped => AgentEvent::Stopped,
            HookKind::SessionEnd => AgentEvent::SessionEnd,
        };
        let status = match event {
            AgentEvent::SessionStart | AgentEvent::PromptSubmitted => AgentStatus::Working,
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
        if let Some(p) = ev.pane.and_then(|raw| self.ws.pane_by_raw(raw)) {
            let conversation = (event != AgentEvent::SessionEnd).then(|| AgentSessionRef {
                agent: ev.agent.clone(),
                session: ev.session_id.clone(),
            });
            if self.ws.set_agent_session(p, conversation) {
                self.save_session();
            }
        }
        let pane_changed =
            pane.is_some_and(|p| self.ws.set_agent_status(p, &ev.agent, status, ev.timestamp));
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
        if worktree_changed.is_some()
            && matches!(event, AgentEvent::Stopped | AgentEvent::SessionEnd)
        {
            self.refresh_repo_of(&worktree, cx);
            self.refresh_sessions(cx);
        }
        self.sync_attention(cx);
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
    /// Dock badge and the pulse timer.
    fn sync_attention(&mut self, cx: &mut Context<Self>) {
        self.sync_panes(cx);
        self.env.system.set_badge(self.ws.unseen_waiting());
        self.ensure_pulse(cx);
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

    /// Run the pulse timer while some agent works; it stops by itself.
    fn ensure_pulse(&mut self, cx: &mut Context<Self>) {
        if self.pulsing || !self.any_working() {
            return;
        }
        self.pulsing = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PULSE_STEP).await;
                let keep = this.update(cx, |view, cx| {
                    if !view.any_working() {
                        view.pulsing = false;
                        view.pulse = 0;
                        cx.notify();
                        return false;
                    }
                    view.pulse = (view.pulse + 1) % PULSE_STEPS;
                    cx.notify();
                    true
                });
                if !keep.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Opacity of a "working" dot for the current pulse phase.
    fn pulse_opacity(&self) -> f32 {
        let t = f32::from(self.pulse) / f32::from(PULSE_STEPS) * std::f32::consts::TAU;
        0.6 + 0.4 * t.cos()
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
            self.status_line = Some("No agent is waiting".into());
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

    fn schedule_refreshes(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let mut ticks: u64 = 0;
            loop {
                cx.background_executor().timer(STATUS_REFRESH).await;
                ticks += 1;
                let alive = this.update(cx, |view, cx| {
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
    fn refresh_repo(&mut self, repo: PathBuf, cx: &mut Context<Self>) {
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
            async move { chda_core::worktrees_of(&repo, fetch) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.refreshing.remove(&repo);
                let before = view.worktree_paths(cx);
                view.sidebar.update(cx, |s, cx| {
                    match result {
                        Ok(worktrees) => s.model.set_worktrees(&repo, worktrees),
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
            let command = match &p.agent {
                Some(conversation) if self.config.restore_agents => {
                    let cwd = p.cwd.clone().unwrap_or_default();
                    match self.resume_argv(&cwd, conversation) {
                        Ok(argv) => Some(argv),
                        Err(why) => {
                            notes.push(why);
                            None
                        }
                    }
                }
                _ => None,
            };
            if command.is_none() {
                self.ws.set_agent_session(p.pane, None);
            }
            self.open_pane(p.pane, p.cwd, command, window, cx);
        }
        if self.ws.is_empty() {
            self.new_tab(&NewTab, window, cx);
            return;
        }
        if !notes.is_empty() {
            self.status_line = Some(format!("Restored the last session; {}", notes.join("; ")));
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
        if !adapter.is_installed() {
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
        if self.quitting || !self.config.restore_session {
            return;
        }
        let Some(dir) = self.env.data_dir.as_deref() else {
            return;
        };
        let mut saved = self.ws.snapshot();
        saved.bounds = self.bounds;
        let _ = saved.save(dir);
    }

    fn sync_panes(&mut self, cx: &mut Context<Self>) {
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
                .tabs_by_activity()
                .into_iter()
                .map(|(tab, title, repo, last_activity)| ActiveTab {
                    status: self
                        .ws
                        .tabs()
                        .iter()
                        .find(|t| t.id == tab)
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
                    last_activity,
                })
                .collect()
        };
        self.sidebar.update(cx, |s, cx| {
            s.model.set_panes(&panes);
            s.model.active_tabs = active_tabs;
            cx.notify();
        });
        self.save_session();
    }

    fn save_config(&self) {
        if let Some(path) = &self.env.config_path {
            let _ = self.config.save(path);
        }
    }

    /// Directory new panes start in: the focused pane's.
    fn inherited_cwd(&self) -> Option<PathBuf> {
        self.focused_cwd()
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
        let env = std::rc::Rc::clone(&self.env);
        let view =
            cx.new(|cx| TerminalView::new(settings, &env, pane.raw(), cwd, command, window, cx));
        let sub = cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
            this.on_pane_event(pane, event, window, cx)
        });
        self.panes.insert(pane, (view, sub));
        self.sync_panes(cx);
    }

    fn on_pane_event(
        &mut self,
        pane: PaneId,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalEvent::Exited => {
                self.ws.close_pane(pane);
                self.panes.remove(&pane);
                if self.ws.is_empty() {
                    // Nothing left to restore: this removes the saved session.
                    self.save_session();
                    cx.quit();
                    return;
                }
                self.focus_active(window, cx);
                self.sync_panes(cx);
            }
            TerminalEvent::Title(title) => {
                if let Some(info) = self.ws.pane_mut(pane) {
                    info.title = title.clone();
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
                self.reviewed_focused(cx);
            }
            TerminalEvent::Prompt => {
                // A shell prompt after an agent ran means the agent exited.
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
                if let Some(info) = self.ws.pane_mut(pane) {
                    info.last_activity = *at;
                }
                self.sync_panes(cx);
            }
        }
        self.sync_title(window);
        cx.notify();
    }

    /// Render a Mermaid diagram in the browser from a local page.
    fn view_diagram(&mut self, source: Option<&str>, cx: &mut Context<Self>) {
        let Some(source) = source else {
            self.status_line = Some("No Mermaid diagram in this pane's output".into());
            return;
        };
        let Some(dir) = self.env.data_dir.as_ref().map(|d| d.join("diagrams")) else {
            return;
        };
        match crate::diagram::write_page(&dir, source) {
            Ok(page) => self.env.system.open_file(&page, cx),
            Err(e) => self.status_line = Some(format!("diagram: {e}")),
        }
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
            self.status_line = Some(format!("editor `{program}`: {e}"));
        }
    }

    /// Looking at a pane clears "review" for it and its worktree, and marks
    /// a waiting agent there as seen (which clears the Dock badge).
    fn reviewed_focused(&mut self, cx: &mut Context<Self>) {
        if let Some(cwd) = self.focused_cwd() {
            self.sidebar.update(cx, |s, cx| {
                if s.model.mark_reviewed(&cwd) {
                    cx.notify();
                }
            });
        }
        if let Some(pane) = self.ws.focused_pane()
            && self.ws.mark_seen(pane)
        {
            self.sync_attention(cx);
        }
    }

    /// Give keyboard focus to the workspace's focused pane.
    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.ws.focused_pane()
            && let Some((view, _)) = self.panes.get(&pane)
        {
            let handle = view.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        if let Some(active) = self.ws.active_tab() {
            self.tab_group = self.ws.tab_repo(active);
        }
        self.reviewed_focused(cx);
        self.sync_title(window);
        self.save_session();
        cx.notify();
    }

    fn sync_title(&self, window: &mut Window) {
        let title = self
            .ws
            .active_tab()
            .map(|t| self.ws.tab_title(t))
            .unwrap_or_else(|| "chda".into());
        window.set_window_title(&title);
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active(window, cx);
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
        if self.sheet.take().is_some()
            || self.note_sheet.take().is_some()
            || self.context_menu.take().is_some()
        {
            self.focus_active(window, cx);
            return;
        }
        let Some(pane) = self.ws.focused_pane() else {
            return;
        };
        self.ws.close_pane(pane);
        self.panes.remove(&pane);
        if self.ws.is_empty() {
            // Nothing left to restore: this removes the saved session.
            self.save_session();
            cx.quit();
            return;
        }
        self.focus_active(window, cx);
        self.sync_panes(cx);
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

    fn add_repo(&mut self, _: &AddRepo, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add repository".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                for path in paths {
                    view.register_repo(path, cx);
                }
            });
        })
        .detach();
    }

    fn register_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let repo = match chda_core::repo_of(&path) {
            Ok(repo) => repo,
            Err(e) => {
                self.status_line = Some(format!("{}: {e}", path.display()));
                cx.notify();
                return;
            }
        };
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
                }
                for a in self.configured_adapters() {
                    items.push((
                        format!("Run {}", a.display_name()),
                        MenuAction::RunAgent(path.clone(), a.id()),
                    ));
                }
                for p in self.presets() {
                    items.push((
                        format!("Run {}", p.name),
                        MenuAction::RunPreset(path.clone(), p.name.clone()),
                    ));
                }
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
                            branch,
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
                    self.register_repo(path, cx);
                }
            }
            SidebarEvent::OpenUrl(url) => cx.open_url(&url),
            SidebarEvent::JumpToAgent(path) => self.jump_to_worktree_agent(&path, window, cx),
            SidebarEvent::OpenDiff(path) => self.open_diff(&path, window, cx),
            SidebarEvent::FocusTab(tab) => {
                if self.ws.activate_tab_id(tab) {
                    self.focus_active(window, cx);
                }
            }
        }
        cx.notify();
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
            self.status_line = Some(format!(
                "{} no longer exists. Right-click the worktree to remove it.",
                path.display()
            ));
            cx.notify();
            return;
        }
        let existing = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(path)
            .and_then(|(_, w)| w.panes.first().copied());
        if let Some(pane) = existing
            && self.ws.focus_pane(pane)
        {
            self.focus_active(window, cx);
            return;
        }
        let command = self.default_action_argv(path);
        self.open_tab_at(Some(path.to_path_buf()), command, window, cx);
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
        self.status_line = Some(format!("Updating {branch}\u{2026}"));
        cx.notify();
        let task = cx.background_spawn({
            let worktree = worktree.clone();
            async move { chda_core::update_branch(&worktree, strategy) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                use chda_core::{PullMode, UpdateOutcome};
                view.status_line = Some(match result {
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
                });
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
        let cmd = adapter.launch_command(cwd, resume, &Self::hook_bin());
        Some(chda_core::agents::command_argv(&cmd))
    }

    /// What `default-action` runs in a new worktree's first tab; `None` is
    /// the login shell.
    fn default_action_argv(&self, cwd: &Path) -> Option<Vec<String>> {
        let agent = AgentId::parse(self.config.default_action.agent()?)?;
        self.agent_argv(cwd, agent, None)
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
        if !adapter.is_installed() {
            self.status_line = Some(format!("{} is not on PATH", adapter.display_name()));
            cx.notify();
            return;
        }
        let argv = self.agent_argv(cwd, agent, resume.as_ref());
        self.open_tab_at(Some(cwd.to_path_buf()), argv, window, cx);
    }

    /// Start the `agent-presets` entry called `name` in a new tab at `cwd`.
    fn run_preset(&mut self, cwd: &Path, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preset) = self.config.agent_presets.iter().find(|p| p.name == name) else {
            return;
        };
        let adapter = AgentId::parse(&preset.agent)
            .and_then(|id| self.adapters.iter().find(|a| a.id() == id));
        let Some(adapter) = adapter else {
            self.status_line = Some(format!("Preset {name}: unknown agent {:?}", preset.agent));
            cx.notify();
            return;
        };
        if !adapter.is_installed() {
            self.status_line = Some(format!("{} is not on PATH", adapter.display_name()));
            cx.notify();
            return;
        }
        let argv = self.agent_argv(cwd, adapter.id(), None).map(|mut argv| {
            argv.extend(preset.args.iter().cloned());
            argv
        });
        self.open_tab_at(Some(cwd.to_path_buf()), argv, window, cx);
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
        let mut launches = Vec::new();
        let mut missing = Vec::new();
        for pick in picks {
            let adapter = AgentId::parse(&pick.agent)
                .and_then(|id| self.adapters.iter().find(|a| a.id() == id));
            let Some(adapter) = adapter else {
                continue;
            };
            if !adapter.is_installed() {
                missing.push(adapter.display_name().to_owned());
                continue;
            }
            let session = SessionId(pick.session);
            if let Some(argv) = self.agent_argv(&pick.worktree, adapter.id(), Some(&session)) {
                launches.push((pick.worktree, argv));
            }
        }
        missing.dedup();
        if !missing.is_empty() {
            self.status_line = Some(format!("{} is not on PATH", missing.join(", ")));
        }
        if launches.is_empty() {
            cx.notify();
            return;
        }
        let aspect = self.tab_aspect(window);
        for (i, (cwd, argv)) in launches.into_iter().enumerate() {
            let pane = if i == 0 {
                Some(self.ws.new_tab().1)
            } else {
                self.ws.split_largest(aspect)
            };
            if let Some(pane) = pane {
                self.open_pane(pane, Some(cwd), Some(argv), window, cx);
            }
        }
        self.focus_active(window, cx);
    }

    /// Width over height of the area tabs are drawn in.
    fn tab_aspect(&self, window: &Window) -> f32 {
        let size = window.viewport_size();
        let sidebar = if self.sidebar_visible {
            self.config.sidebar_width as f32
        } else {
            0.0
        };
        let width = (f32::from(size.width) - sidebar).max(1.0);
        width / f32::from(size.height).max(1.0)
    }

    /// Open a tab that pages the worktree's diff against the commit it
    /// branched from, uncommitted edits included. Quitting the pager closes
    /// the tab.
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
            MenuAction::OpenTerminal(path) => self.open_tab_at(Some(path), None, window, cx),
            MenuAction::ViewDiff(path) => self.open_diff(&path, window, cx),
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
                    self.status_line = Some(format!("{}: already {busy}", entry.path.display()));
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
                let task = cx.background_spawn({
                    let repo = repo.clone();
                    async move { chda_core::merge_and_clean(&repo, &entry, force) }
                });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        view.status_line = Some(match result {
                            Ok(r) => {
                                format!("Merged {} and removed {}", r.branch, r.removed.display())
                            }
                            Err(e) => e.to_string(),
                        });
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
                    self.status_line = Some("No merged, clean worktrees to remove".into());
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
                    let results = task.await;
                    let _ = this.update(cx, |view, cx| {
                        let ok = results.iter().filter(|r| r.is_ok()).count();
                        let errors: Vec<String> = results
                            .iter()
                            .filter_map(|r| r.as_ref().err().map(|e| e.to_string()))
                            .collect();
                        view.status_line = Some(if errors.is_empty() {
                            format!("Removed {ok} worktree(s)")
                        } else {
                            format!("Removed {ok}; failed: {}", errors.join("; "))
                        });
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
                    self.status_line = Some(format!("{}: already {busy}", entry.path.display()));
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
                        view.status_line = Some(match &result {
                            Ok(()) => format!("Deleted {}", worktree.display()),
                            Err(e) => e.to_string(),
                        });
                        if result.is_ok() {
                            let repo = repo.clone();
                            cx.background_spawn(async move {
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
                    self.status_line = Some("Every local branch already has a worktree".into());
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
                let task =
                    cx.background_spawn({
                        let repo = repo.clone();
                        async move {
                            chda_core::delete_worktree(&repo, &worktree, false).map(|_| worktree)
                        }
                    });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        view.status_line = Some(match result {
                            Ok(p) => format!("Removed the record of {}", p.display()),
                            Err(e) => e.to_string(),
                        });
                        view.refresh_repo(repo, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
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
                    .first()
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
        let branch = match sheet.input.read(cx).text().trim() {
            "" => sheet.suggestion.clone(),
            typed => typed.to_owned(),
        };
        let note = sheet.note.read(cx).text().trim().to_owned();
        let repo = sheet.repo.clone();
        let base = sheet.base.clone();
        let path = self.config.worktree_path(&repo, &branch);
        match chda_core::create_worktree(&repo, &branch, &path, base.as_deref()) {
            Ok(()) => {
                self.sheet = None;
                if !note.is_empty()
                    && let Err(e) = chda_core::set_note(&repo, &branch, &note)
                {
                    self.status_line =
                        Some(format!("Created {branch}, but saving its note failed: {e}"));
                }
                self.refresh_repo(repo, cx);
                let command = self.default_action_argv(&path);
                self.open_tab_at(Some(path), command, window, cx);
            }
            Err(e) => {
                if let Some(sheet) = &mut self.sheet {
                    sheet.error = Some(e.to_string());
                }
            }
        }
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
        let sidebar = &self.sidebar.read(cx).model;
        for repo in &sidebar.repos {
            items.push(PaletteItem {
                label: format!("New worktree in {}", repo.name),
                detail: repo.path.to_string_lossy().into_owned(),
                command: PaletteCommand::NewWorktree(repo.path.clone()),
            });
            for wt in &repo.worktrees {
                let branch = wt.branch.clone().unwrap_or_else(|| "(detached)".into());
                items.push(PaletteItem {
                    label: format!("Go to {}/{branch}", repo.name),
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
                        label: format!("Run {} in {}/{branch}", a.display_name(), repo.name),
                        detail: wt.path.to_string_lossy().into_owned(),
                        command: PaletteCommand::RunAgent(wt.path.clone(), a.id().as_str().into()),
                    });
                }
                for p in self.presets() {
                    items.push(PaletteItem {
                        label: format!("Run {} in {}/{branch}", p.name, repo.name),
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
                            "Resume {} in {}/{branch} ({})",
                            self.agent_name(&s.agent),
                            repo.name,
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
            self.status_line = Some(format!("{}: {e}", path.display()));
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
            self.status_line = Some("No agent sessions in the sidebar's worktrees yet".into());
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
                        self.status_line = Some(e.to_string());
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
        if self.sheet.take().is_some()
            || self.note_sheet.take().is_some()
            || self.confirm.take().is_some()
            || self.palette.take().is_some()
            || self.context_menu.take().is_some()
            || self.status_line.take().is_some()
        {
            self.restore_theme(cx);
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tabs = self.ws.tabs_in(self.tab_group.as_deref());
        if tabs.len() < 2 {
            return None;
        }
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let active = self.ws.active_index_in(self.tab_group.as_deref());
        let bar = div()
            .flex()
            .flex_row()
            .w_full()
            .flex_shrink_0()
            .bg(blend(bg, fg, 0.06))
            .text_sm()
            .text_color(fg)
            .children(tabs.into_iter().enumerate().map(|(i, tab)| {
                let title = self.ws.tab_title(tab);
                let bell = self.ws.pane(tab.focused).is_some_and(|p| p.bell);
                let is_active = active == Some(i);
                if let Some((id, input, _)) = &self.renaming
                    && *id == tab.id
                {
                    return div()
                        .id(("tab", i))
                        .px_2()
                        .min_w_0()
                        .flex_1()
                        .bg(bg)
                        .child(input.clone());
                }
                let tab_id = tab.id;
                let agent = self.ws.tab_agent(tab).cloned();
                let tip = agent.as_ref().map(|a| {
                    let name = self.agent_name(&a.agent);
                    match a.status {
                        AgentStatus::Working => format!("{name} is working"),
                        AgentStatus::WaitingInput => format!("{name} is waiting for input"),
                        AgentStatus::Review => format!("{name} finished; not looked at yet"),
                        AgentStatus::Idle => name,
                    }
                });
                let dot = agent.map(|a| {
                    let mut color = crate::sidebar_view::status_color(a.status);
                    if a.status == AgentStatus::Working {
                        color = color.opacity(self.pulse_opacity());
                    }
                    div().flex_shrink_0().text_color(color).child("\u{25cf}")
                });
                div()
                    .id(("tab", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .min_w_0()
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
            }));
        Some(bar.into_any_element())
    }

    fn render_node(&self, node: &Node, divider: Hsla) -> AnyElement {
        match node {
            Node::Leaf(pane) => match self.panes.get(pane) {
                Some((view, _)) => div().size_full().child(view.clone()).into_any_element(),
                None => div().size_full().into_any_element(),
            },
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.render_node(first, divider);
                let second = self.render_node(second, divider);
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
                div()
                    .absolute()
                    .size_full()
                    .top_0()
                    .left_0()
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
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
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
                    .pt_16()
                    .bg(gpui::black().opacity(0.3))
                    .occlude()
                    .child(
                        div()
                            .w(px(480.0))
                            .p_3()
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
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(confirm.title.clone()),
                            )
                            .children(confirm.lines.iter().map(|l| {
                                div().text_xs().text_color(fg.opacity(0.8)).child(l.clone())
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .justify_end()
                                    .child(
                                        div()
                                            .id("confirm-cancel")
                                            .px_3()
                                            .py_1()
                                            .rounded_sm()
                                            .bg(fg.opacity(0.1))
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.confirm = None;
                                                this.focus_active(window, cx);
                                            }))
                                            .child("Cancel"),
                                    )
                                    .child(
                                        div()
                                            .id("confirm-ok")
                                            .px_3()
                                            .py_1()
                                            .rounded_sm()
                                            .bg(gpui::rgb(0xf38ba8).opacity(0.6))
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.confirm_action(window, cx)
                                            }))
                                            .child("Proceed"),
                                    ),
                            ),
                    ),
            )
            .into_any_element(),
        )
    }

    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.sheet.as_ref()?;
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let repo_name = sheet
            .repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let typed = sheet.input.read(cx).text().trim();
        let branch = if typed.is_empty() {
            format!("{} (random)", sheet.suggestion)
        } else {
            typed.to_owned()
        };
        let path = self.config.worktree_path(
            &sheet.repo,
            if typed.is_empty() {
                &sheet.suggestion
            } else {
                typed
            },
        );
        let base = sheet.base.as_deref().unwrap_or("HEAD");
        let preview = [format!("{branch} from {base}"), path.display().to_string()];
        let title = match &sheet.base {
            Some(base) => format!("New worktree in {repo_name} from {base}"),
            None => format!("New worktree in {repo_name}"),
        };
        let body = div()
            .flex()
            .flex_col()
            .gap_2()
            .on_key_down(cx.listener(Self::sheet_tab))
            .child(sheet.input.clone())
            .child(sheet.note.clone())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_xs()
                    .text_color(fg.opacity(0.6))
                    .children(preview),
            );
        Some(self.sheet_frame(
            title,
            body,
            sheet.error.clone(),
            "Leave the name empty for a random one. Tab to the note, shift-enter for a new line. Enter to create, Esc to cancel",
        ))
    }

    fn render_note_sheet(&self) -> Option<AnyElement> {
        let sheet = self.note_sheet.as_ref()?;
        Some(self.sheet_frame(
            format!("Note for {}", sheet.branch),
            div().child(sheet.input.clone()),
            sheet.error.clone(),
            "First line is the title shown in the sidebar. Shift-enter for a new line, Enter to save, Esc to cancel",
        ))
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
                .items_start()
                .justify_center()
                .pt_16()
                .bg(gpui::black().opacity(0.3))
                .occlude()
                .child(
                    div()
                        .w(px(420.0))
                        .p_3()
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
                        .child(title)
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

impl WorkspaceView {
    fn render_status_line(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = self.status_line.clone()?;
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        Some(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap_1()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(fg.opacity(0.8))
                .bg(blend(bg, fg, 0.1))
                .child(div().flex_1().min_w_0().child(s))
                .child(
                    div()
                        .id("status-close")
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(fg.opacity(0.15)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.status_line = None;
                            cx.notify();
                        }))
                        .child("\u{2715}"),
                )
                .into_any_element(),
        )
    }
}

/// Mix `a` towards `b` by `t`.
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let divider = blend(bg, fg, 0.2);
        let content = match self.ws.active_tab() {
            Some(tab) => match tab.zoomed {
                Some(pane) => self.render_node(&Node::Leaf(pane), divider),
                None => self.render_node(&tab.root, divider),
            },
            None => div().into_any_element(),
        };
        let sidebar_width = px(self.config.sidebar_width as f32);
        // The status line sits under the sidebar, or under the panes when
        // the sidebar is hidden.
        let mut status = self.render_status_line(cx);
        let main_status = if self.sidebar_visible {
            None
        } else {
            status.take()
        };
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .children(self.render_tab_bar(cx))
            .child(div().flex_1().min_h_0().w_full().child(content))
            .children(main_status);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(bg)
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::new_tab))
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
            .when(self.sidebar_visible, |d| {
                d.child(
                    div()
                        .w(sidebar_width)
                        .h_full()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .border_r_1()
                        .border_color(divider)
                        .child(div().flex_1().min_h_0().child(self.sidebar.clone()))
                        .children(status),
                )
            })
            .child(main)
            .children(self.render_context_menu(cx))
            .children(self.render_sheet(cx))
            .children(self.render_note_sheet())
            .children(self.render_confirm(cx))
            .children(self.render_palette())
    }
}
