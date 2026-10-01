//! The window's root view: sidebar, tab bar and the split tree of panes.
//! It also owns the agent hook receiver and the sidebar refresh schedule.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chda_config::{ChdaConfig, DefaultAction, TabTitle};
use chda_core::agents::{
    AgentAdapter, AgentId, HookEvent, HookKind, SessionCache, SessionId, adapters, data_dir, ipc,
};
use chda_core::{
    ActiveTab, AgentEvent, AgentStatus, Axis, Direction, Node, PaneId, RepoWatcher, TabId,
    TitleMode, Workspace,
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
use crate::sidebar_view::{SidebarEvent, SidebarView};
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

/// A popup menu anchored at a window position.
struct ContextMenu {
    position: Point<Pixels>,
    items: Vec<(String, MenuAction)>,
}

#[derive(Clone, Debug)]
enum MenuAction {
    OpenTerminal(PathBuf),
    RunAgent(PathBuf, AgentId),
    NewWorktree(PathBuf),
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
    /// Pick an existing branch to check out as a new worktree.
    PickBranch(PathBuf),
    RemoveRepo(PathBuf),
    RefreshRepo(PathBuf),
}

/// A yes/no sheet before a destructive action.
struct ConfirmSheet {
    title: String,
    lines: Vec<String>,
    action: MenuAction,
}

/// The "new worktree" sheet.
struct NewWorktreeSheet {
    repo: PathBuf,
    input: Entity<TextInput>,
    _sub: Subscription,
    error: Option<String>,
}

pub struct WorkspaceView {
    /// Settings new panes start with; `font_size` follows the font size
    /// actions, while `configured_font_size` is what `cmd-0` returns to.
    settings: Settings,
    configured_font_size: Pixels,
    config: ChdaConfig,
    ws: Workspace,
    panes: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    focus_handle: FocusHandle,
    sidebar: Entity<SidebarView>,
    _sidebar_sub: Subscription,
    sidebar_visible: bool,
    adapters: Arc<Vec<Box<dyn AgentAdapter>>>,
    session_cache: Arc<Mutex<SessionCache>>,
    hook_events: Option<mpsc::Receiver<HookEvent>>,
    refreshing: HashSet<PathBuf>,
    refresh_again: HashSet<PathBuf>,
    watcher: Option<RepoWatcher>,
    watch_events: Option<mpsc::Receiver<PathBuf>>,
    context_menu: Option<ContextMenu>,
    sheet: Option<NewWorktreeSheet>,
    confirm: Option<ConfirmSheet>,
    palette: Option<(Entity<Palette>, Subscription)>,
    /// Inline editor for a tab title.
    renaming: Option<(TabId, Entity<TextInput>, Subscription)>,
    /// Repository whose tabs the tab bar shows (`None`: tabs outside repos).
    tab_group: Option<PathBuf>,
    /// `gh` is installed and authenticated; checked once.
    gh_ok: bool,
    /// Last time pull requests were fetched per repository.
    pr_fetched: HashMap<PathBuf, std::time::Instant>,
    /// Last `origin/<default>` fetch per repository.
    base_fetched: HashMap<PathBuf, std::time::Instant>,
    /// Message shown briefly at the bottom of the sidebar.
    status_line: Option<String>,
}

impl WorkspaceView {
    pub fn new(settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = ChdaConfig::default_path()
            .and_then(|p| ChdaConfig::load(&p).ok())
            .unwrap_or_default();
        let bg = hsla(settings.colors.background.unwrap_or_default());
        let fg = hsla(settings.colors.foreground.unwrap_or_default());
        let sidebar = cx.new(|cx| SidebarView::new(fg, blend(bg, fg, 0.04), cx));
        let sidebar_sub = cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| {
            this.on_sidebar_event(event.clone(), window, cx)
        });
        let adapters: Arc<Vec<Box<dyn AgentAdapter>>> = Arc::new(adapters());
        let hook_events = Self::start_hook_receiver(window, cx);
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
            session_cache: Arc::new(Mutex::new(SessionCache::new())),
            hook_events,
            refreshing: HashSet::new(),
            refresh_again: HashSet::new(),
            watcher,
            watch_events,
            context_menu: None,
            sheet: None,
            confirm: None,
            palette: None,
            renaming: None,
            tab_group: None,
            gh_ok: false,
            pr_fetched: HashMap::new(),
            base_fetched: HashMap::new(),
            status_line: None,
        };
        let gh = cx.background_spawn(async { chda_core::gh_available() });
        cx.spawn(async move |this, cx| {
            let ok = gh.await;
            let _ = this.update(cx, |view, cx| {
                view.gh_ok = ok;
                if ok {
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
        this.new_tab(&NewTab, window, cx);
        this.refresh_all(cx);
        this.refresh_sessions(cx);
        Self::schedule_refreshes(window, cx);
        this
    }

    fn hook_bin() -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("chda"))
    }

    fn install_hooks(&self) {
        let Some(data_dir) = data_dir() else {
            return;
        };
        for adapter in self.adapters.iter() {
            let _ = adapter.install_hooks(&data_dir, &Self::hook_bin());
        }
    }

    /// Listen for `chda hook` on the local socket and drain the fallback log.
    fn start_hook_receiver(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<mpsc::Receiver<HookEvent>> {
        let data_dir = data_dir()?;
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
        let focused_cwd = self.focused_cwd();
        let window_active = window.is_window_active();
        let changed = self.sidebar.update(cx, |s, cx| {
            let r = s
                .model
                .apply_agent_event(&ev.agent, &ev.cwd, event, ev.timestamp);
            cx.notify();
            r
        });
        if let Some((worktree, status)) = changed {
            let looking = window_active
                && focused_cwd
                    .as_ref()
                    .is_some_and(|c| c.starts_with(&worktree));
            let name = worktree
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let agent = self
                .adapters
                .iter()
                .find(|a| a.id() == ev.agent_id().unwrap_or(AgentId::Claude))
                .map(|a| a.display_name().to_owned())
                .unwrap_or(ev.agent.clone());
            match status {
                AgentStatus::WaitingInput if self.config.notifications => {
                    platform::notify(&format!("{agent} is waiting"), &name);
                }
                AgentStatus::Review if self.config.notifications && !looking => {
                    platform::notify(&format!("{agent} finished"), &name);
                }
                AgentStatus::Review if looking => {
                    self.sidebar.update(cx, |s, _| {
                        s.model.mark_reviewed(&worktree);
                    });
                }
                _ => {}
            }
            if matches!(event, AgentEvent::Stopped | AgentEvent::SessionEnd) {
                self.refresh_repo_of(&worktree, cx);
                self.refresh_sessions(cx);
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
                if view.refresh_again.remove(&repo) {
                    view.refresh_repo(repo, cx);
                } else {
                    view.refresh_prs(false, cx);
                }
            });
        })
        .detach();
    }

    /// Fetch pull request state for every non-main worktree through `gh`,
    /// at most once a minute per repository unless `force`.
    fn refresh_prs(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.gh_ok {
            return;
        }
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
            let task = cx.background_spawn({
                let repo = repo.clone();
                async move {
                    branches
                        .into_iter()
                        .map(|(path, branch)| (path, chda_core::pr_for_branch(&repo, &branch)))
                        .collect::<Vec<_>>()
                }
            });
            cx.spawn(async move |this, cx| {
                let results = task.await;
                let _ = this.update(cx, |view, cx| {
                    view.sidebar.update(cx, |s, cx| {
                        let mut changed = false;
                        for (path, pr) in results {
                            changed |= s.model.set_pr(&path, pr);
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

    fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        let adapters = Arc::clone(&self.adapters);
        let cache = Arc::clone(&self.session_cache);
        let task = cx.background_spawn(async move { chda_core::index_sessions(&adapters, &cache) });
        cx.spawn(async move |this, cx| {
            let sessions = task.await;
            let _ = this.update(cx, |view, cx| {
                view.sidebar.update(cx, |s, cx| {
                    s.model.set_sessions(&sessions);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// Tell the sidebar which panes live in which worktree, and remember each
    /// pane's branch and repository for tab titles and grouping.
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
    }

    fn save_config(&self) {
        if let Some(path) = ChdaConfig::default_path() {
            let _ = self.config.save(&path);
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
        let view = cx.new(|cx| TerminalView::new(settings, cwd, command, window, cx));
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
                platform::beep();
            }
            TerminalEvent::Focused => {
                self.ws.focus_pane(pane);
                self.context_menu = None;
                self.reviewed_focused(cx);
            }
            TerminalEvent::Prompt => {
                if self.sidebar_visible
                    && let Some(cwd) = self.ws.pane(pane).and_then(|i| i.cwd.clone())
                {
                    self.refresh_repo_of(&cwd, cx);
                }
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

    /// Looking at a pane clears "review" for its worktree.
    fn reviewed_focused(&mut self, cx: &mut Context<Self>) {
        if let Some(cwd) = self.focused_cwd() {
            self.sidebar.update(cx, |s, cx| {
                if s.model.mark_reviewed(&cwd) {
                    cx.notify();
                }
            });
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
        if self.sheet.take().is_some() || self.context_menu.take().is_some() {
            self.focus_active(window, cx);
            return;
        }
        let Some(pane) = self.ws.focused_pane() else {
            return;
        };
        self.ws.close_pane(pane);
        self.panes.remove(&pane);
        if self.ws.is_empty() {
            cx.quit();
            return;
        }
        self.focus_active(window, cx);
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

    fn on_sidebar_event(
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
                    .map(|(r, w)| (r.path.clone(), w.is_main));
                let Some((repo, is_main)) = repo else {
                    return;
                };
                let mut items = vec![(
                    "Open terminal".to_owned(),
                    MenuAction::OpenTerminal(path.clone()),
                )];
                for a in self
                    .adapters
                    .iter()
                    .filter(|a| self.config.agents.iter().any(|id| id == a.id().as_str()))
                {
                    items.push((
                        format!("Run {}", a.display_name()),
                        MenuAction::RunAgent(path.clone(), a.id()),
                    ));
                }
                items.push((
                    "New worktree...".into(),
                    MenuAction::NewWorktree(repo.clone()),
                ));
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
            SidebarEvent::ResumeSession {
                worktree,
                agent,
                session,
            } => {
                if let Some(agent) = AgentId::parse(&agent) {
                    self.run_agent(&worktree, agent, Some(SessionId(session)), window, cx);
                }
            }
            SidebarEvent::AddRepo => self.add_repo(&AddRepo, window, cx),
            SidebarEvent::OpenUrl(url) => cx.open_url(&url),
            SidebarEvent::FocusTab(tab) => {
                if self.ws.activate_tab_id(tab) {
                    self.focus_active(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Focus a pane already in the worktree, else open a tab there.
    fn open_worktree(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
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
        let command = match self.config.default_action {
            DefaultAction::Terminal => None,
            DefaultAction::Claude => self.agent_argv(path, AgentId::Claude, None),
            DefaultAction::Codex => self.agent_argv(path, AgentId::Codex, None),
        };
        self.open_tab_at(Some(path.to_path_buf()), command, window, cx);
    }

    fn agent_argv(
        &self,
        cwd: &Path,
        agent: AgentId,
        resume: Option<&SessionId>,
    ) -> Option<Vec<String>> {
        let adapter = self.adapters.iter().find(|a| a.id() == agent)?;
        let cmd = adapter.launch_command(cwd, resume, &Self::hook_bin());
        let mut argv = vec![cmd.get_program().to_string_lossy().into_owned()];
        argv.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
        Some(argv)
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

    fn run_menu_action(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        match action {
            MenuAction::OpenTerminal(path) => self.open_tab_at(Some(path), None, window, cx),
            MenuAction::RunAgent(path, agent) => self.run_agent(&path, agent, None, window, cx),
            MenuAction::NewWorktree(repo) => self.open_sheet(repo, window, cx),
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
                if !force {
                    let name = entry.branch.clone().unwrap_or_default();
                    let (title, lines) = if entry.safe_to_delete() {
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
                let task = cx.background_spawn({
                    let repo = repo.clone();
                    async move {
                        chda_core::delete_worktree(&repo, &entry.path, true)?;
                        if let Some(branch) = &entry.branch {
                            chda_core::delete_branch(&repo, branch, true)?;
                        }
                        Ok::<_, std::io::Error>(entry.path.clone())
                    }
                });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        view.status_line = Some(match result {
                            Ok(p) => format!("Deleted {}", p.display()),
                            Err(e) => e.to_string(),
                        });
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
                self.open_palette(items, window, cx);
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
            MenuAction::RefreshRepo(repo) => {
                self.refresh_repo(repo, cx);
                self.refresh_sessions(cx);
                self.refresh_prs(true, cx);
            }
        }
        cx.notify();
    }

    fn open_sheet(&mut self, repo: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let input = cx.new(|cx| TextInput::new("branch name", fg, blend(bg, fg, 0.12), cx));
        let sub = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            TextInputEvent::Submit(branch) => this.create_worktree(branch.clone(), window, cx),
            TextInputEvent::Cancel => {
                this.sheet = None;
                this.focus_active(window, cx);
            }
        });
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.sheet = Some(NewWorktreeSheet {
            repo,
            input,
            _sub: sub,
            error: None,
        });
        cx.notify();
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

    fn create_worktree(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        let branch = branch.trim().to_owned();
        let Some(sheet) = &mut self.sheet else {
            return;
        };
        if branch.is_empty() {
            sheet.error = Some("Branch name is empty".into());
            cx.notify();
            return;
        }
        let repo = sheet.repo.clone();
        let path = self.config.worktree_path(&repo, &branch);
        match chda_core::create_worktree(&repo, &branch, &path) {
            Ok(()) => {
                self.sheet = None;
                self.refresh_repo(repo, cx);
                let command = match self.config.default_action {
                    DefaultAction::Terminal => None,
                    DefaultAction::Claude => self.agent_argv(&path, AgentId::Claude, None),
                    DefaultAction::Codex => self.agent_argv(&path, AgentId::Codex, None),
                };
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

    fn palette_items(&self, cx: &App) -> Vec<PaletteItem> {
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
        ]
        .into_iter()
        .map(|(label, detail, action)| PaletteItem {
            label: label.into(),
            detail: detail.into(),
            command: PaletteCommand::Action(action),
        })
        .collect();
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
                    detail: wt.path.to_string_lossy().into_owned(),
                    command: PaletteCommand::GoToWorktree(wt.path.clone()),
                });
                for a in self.adapters.iter() {
                    items.push(PaletteItem {
                        label: format!("Run {} in {}/{branch}", a.display_name(), repo.name),
                        detail: wt.path.to_string_lossy().into_owned(),
                        command: PaletteCommand::RunAgent(wt.path.clone(), a.id().as_str().into()),
                    });
                }
                for s in wt.sessions.iter().take(5) {
                    items.push(PaletteItem {
                        label: format!("Resume {} in {}/{branch}", s.agent, repo.name),
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
            self.focus_active(window, cx);
            return;
        }
        let items = self.palette_items(cx);
        self.open_palette(items, window, cx);
    }

    fn open_palette(
        &mut self,
        items: Vec<PaletteItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let palette = cx.new(|cx| Palette::new(items, fg, blend(bg, fg, 0.08), window, cx));
        let sub = cx.subscribe_in(&palette, window, |this, _, event, window, cx| {
            this.palette = None;
            match event {
                PaletteEvent::Chosen(command) => {
                    this.run_palette_command(command.clone(), window, cx)
                }
                PaletteEvent::Dismissed => this.focus_active(window, cx),
            }
            cx.notify();
        });
        self.palette = Some((palette, sub));
        cx.notify();
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
            PaletteCommand::CheckoutBranch(repo, branch) => {
                let path = self.config.worktree_path(&repo, &branch);
                match chda_core::create_worktree(&repo, &branch, &path) {
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

    fn confirm_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            || self.confirm.take().is_some()
            || self.palette.take().is_some()
            || self.context_menu.take().is_some()
            || self.status_line.take().is_some()
        {
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
                div()
                    .id(("tab", i))
                    .px_3()
                    .py_1()
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .cursor_pointer()
                    .when(is_active, |d| d.bg(bg))
                    .when(!is_active, |d| d.text_color(fg.opacity(0.6)))
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        if e.click_count() >= 2 {
                            this.start_rename(tab_id, window, cx);
                        } else {
                            this.goto_tab(i, window, cx);
                        }
                    }))
                    .child(format!(
                        "{}{}  {}",
                        if bell { "\u{25cf} " } else { "" },
                        i + 1,
                        title
                    ))
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
                div()
                    .id(("menu", i))
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.12)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_menu_action(action.clone(), window, cx)
                    }))
                    .child(label)
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
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let repo_name = sheet
            .repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let preview = self
            .config
            .worktree_path(&sheet.repo, sheet.input.read(cx).text())
            .to_string_lossy()
            .into_owned();
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
                            .child(format!("New worktree in {repo_name}"))
                            .child(sheet.input.clone())
                            .child(div().text_xs().text_color(fg.opacity(0.6)).child(preview))
                            .children(
                                sheet.error.clone().map(|e| {
                                    div().text_xs().text_color(gpui::rgb(0xf38ba8)).child(e)
                                }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(fg.opacity(0.5))
                                    .child("Enter to create, Esc to cancel"),
                            ),
                    ),
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
        let status_line = self.status_line.clone();
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .children(self.render_tab_bar(cx))
            .child(div().flex_1().min_h_0().w_full().child(content));
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
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ResizeRight, _, cx| {
                this.ws.resize(Direction::Right, RESIZE_STEP);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ResizeUp, _, cx| {
                this.ws.resize(Direction::Up, RESIZE_STEP);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ResizeDown, _, cx| {
                this.ws.resize(Direction::Down, RESIZE_STEP);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &EqualizeSplits, _, cx| {
                this.ws.equalize();
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
                        .children(status_line.map(|s| {
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
                        })),
                )
            })
            .child(main)
            .children(self.render_context_menu(cx))
            .children(self.render_sheet(cx))
            .children(self.render_confirm(cx))
            .children(self.render_palette())
    }
}
