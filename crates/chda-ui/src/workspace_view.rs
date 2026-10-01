//! The window's root view: sidebar, tab bar and the split tree of panes.
//! It also owns the agent hook receiver and the sidebar refresh schedule.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chda_config::{ChdaConfig, DefaultAction};
use chda_core::agents::{
    AgentAdapter, AgentId, HookEvent, HookKind, SessionCache, SessionId, adapters, data_dir, ipc,
};
use chda_core::{AgentEvent, AgentStatus, Axis, Direction, Node, PaneId, RepoWatcher, Workspace};
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, MouseButton, PathPromptOptions,
    Pixels, Point, Render, Subscription, Window, actions, anchored, deferred, div, prelude::*, px,
    relative,
};

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
        Dismiss,
        Quit,
    ]
);

/// Fraction of the tab a keyboard resize moves the divider by.
const RESIZE_STEP: f32 = 0.05;
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
    DeleteWorktree(PathBuf, PathBuf),
    RemoveRepo(PathBuf),
    RefreshRepo(PathBuf),
}

/// The "new worktree" sheet.
struct NewWorktreeSheet {
    repo: PathBuf,
    input: Entity<TextInput>,
    _sub: Subscription,
    error: Option<String>,
}

pub struct WorkspaceView {
    settings: Settings,
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

        let mut this = Self {
            sidebar_visible: config.sidebar_visible,
            settings,
            config,
            ws: Workspace::new(),
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
            status_line: None,
        };
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
        let task = cx.background_spawn({
            let repo = repo.clone();
            async move { chda_core::worktrees_of(&repo) }
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
                }
            });
        })
        .detach();
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

    /// Tell the sidebar which panes live in which worktree.
    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        let panes: Vec<(PaneId, PathBuf)> = self
            .ws
            .tabs()
            .iter()
            .flat_map(|t| t.panes())
            .filter_map(|p| self.ws.pane(p).and_then(|i| i.cwd.clone()).map(|c| (p, c)))
            .collect();
        self.sidebar.update(cx, |s, cx| {
            s.model.set_panes(&panes);
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
        if self.ws.activate_tab(index) {
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
                        "Delete worktree".into(),
                        MenuAction::DeleteWorktree(repo, path),
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
            MenuAction::DeleteWorktree(repo, path) => {
                let dirty = self
                    .sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&path)
                    .is_some_and(|(_, w)| {
                        w.badges.dirty_count() > 0 || w.badges.ahead.unwrap_or(0) > 0
                    });
                if dirty {
                    self.status_line = Some(format!(
                        "{}: has changes or unpushed commits; not deleted",
                        path.display()
                    ));
                } else {
                    match chda_core::delete_worktree(&repo, &path, false) {
                        Ok(()) => self.status_line = Some(format!("Deleted {}", path.display())),
                        Err(e) => self.status_line = Some(e.to_string()),
                    }
                }
                self.refresh_repo(repo, cx);
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

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.sheet.take().is_some()
            || self.context_menu.take().is_some()
            || self.status_line.take().is_some()
        {
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tabs = self.ws.tabs();
        if tabs.len() < 2 {
            return None;
        }
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let active = self.ws.active_index();
        let bar = div()
            .flex()
            .flex_row()
            .w_full()
            .flex_shrink_0()
            .bg(blend(bg, fg, 0.06))
            .text_sm()
            .text_color(fg)
            .children(tabs.iter().enumerate().map(|(i, tab)| {
                let title = self.ws.tab_title(tab);
                let bell = self.ws.pane(tab.focused).is_some_and(|p| p.bell);
                let is_active = active == Some(i);
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
                    .on_click(cx.listener(move |this, _, window, cx| this.goto_tab(i, window, cx)))
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
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(|this, _: &NextTab, w, cx| {
                this.ws.cycle_tab(true);
                this.focus_active(w, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, w, cx| {
                this.ws.cycle_tab(false);
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
                let last = this.ws.tabs().len().saturating_sub(1);
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
                                .px_2()
                                .py_1()
                                .text_xs()
                                .text_color(fg.opacity(0.8))
                                .bg(blend(bg, fg, 0.1))
                                .child(s)
                        })),
                )
            })
            .child(main)
            .children(self.render_context_menu(cx))
            .children(self.render_sheet(cx))
    }
}
