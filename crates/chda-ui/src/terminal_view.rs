//! A view that owns one terminal session and forwards input to it.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use std::time::Duration;

use chda_term::{
    Event, Frame, KeyAction, KeyCode, KeyInput, LinkTarget, Modifiers, MouseAction,
    MouseButton as TermButton, MouseInput, SearchQuery, SearchStatus, Session, SessionOptions,
    Size, default_data_dir, launch_for, login_shell, parse_pwd_report,
};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, KeyDownEvent, Keystroke, Modifiers as GpuiModifiers, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
    ScrollWheelEvent, Subscription, TouchPhase, UTF16Selection, Window, actions, div, prelude::*,
    px,
};

use crate::settings::Settings;
use crate::terminal_element::{TerminalElement, hsla};
use crate::text_input::{TextInput, TextInputEvent};

actions!(
    terminal,
    [
        Copy,
        Paste,
        JumpToPrevPrompt,
        JumpToNextPrompt,
        Find,
        FindNext,
        FindPrevious,
        CloseFind,
        ToggleFindCase,
        ToggleFindRegex,
    ]
);

/// The search bar at the top of a pane (`cmd-f`).
struct SearchBar {
    input: Entity<TextInput>,
    _observe: Subscription,
    _events: Subscription,
    query: SearchQuery,
    status: SearchStatus,
}

/// What a terminal tells its workspace.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminalEvent {
    /// The shell exited; the pane should go away.
    Exited,
    /// OSC 0/2 title; empty means cleared.
    Title(String),
    /// Working directory, from OSC 7 or the process fallback.
    Cwd(PathBuf),
    Bell,
    /// The user clicked into this pane.
    Focused,
    /// A command finished and the shell shows its prompt again.
    Prompt,
    /// Output or input happened (at most once per second).
    Activity(u64),
    /// The user cmd-clicked an existing file path.
    OpenPath {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
}

/// Where a cmd-clicked link goes.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkOpen {
    Url(String),
    Path {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
}

/// The link under the mouse while cmd is held.
#[derive(Clone, Debug, PartialEq)]
pub struct HoveredLink {
    /// `(row, first col, last col)` per viewport row.
    pub cells: Vec<(u16, u16, u16)>,
    pub open: LinkOpen,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const CWD_POLL: Duration = Duration::from_secs(1);

/// Where the cell grid sits in the window, from the last layout.
#[derive(Clone, Copy, Debug)]
pub struct GridGeometry {
    pub origin: Point<Pixels>,
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub size: Size,
}

const BLINK_INTERVAL: Duration = Duration::from_millis(600);

pub struct TerminalView {
    session: Session,
    events: mpsc::Receiver<Event>,
    frame: Arc<Frame>,
    focus_handle: FocusHandle,
    pub settings: Settings,
    /// Grid size and cell size in pixels last sent to the session.
    grid: (Size, u32, u32),
    scroll_px: f32,
    /// In-progress IME composition shown at the cursor.
    pub marked_text: Option<String>,
    /// Cursor cell bounds from the last layout, for the IME candidate window.
    pub cursor_bounds: Option<Bounds<Pixels>>,
    pub geometry: Option<GridGeometry>,
    /// Shaped text reused across cursor blinks.
    pub layout_cache: Option<std::rc::Rc<crate::terminal_element::CachedLayout>>,
    /// Blink phase; the cursor is drawn when true.
    pub blink_on: bool,
    /// Bumped on input so the blink restarts in the visible phase.
    blink_epoch: u64,
    /// Cached so the blink timer never needs the window.
    focused: bool,
    window_active: bool,
    /// Once the shell reports OSC 7 the process fallback is not needed.
    osc7_seen: bool,
    cwd: Option<PathBuf>,
    last_activity_sent: u64,
    search: Option<SearchBar>,
    /// Viewport cell under the mouse, while it is over the grid.
    hover_cell: Option<(u16, u16)>,
    cmd_held: bool,
    pub hovered_link: Option<HoveredLink>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl TerminalView {
    pub fn new(
        settings: Settings,
        pane_id: u64,
        cwd: Option<PathBuf>,
        command: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (events_tx, events) = mpsc::channel();
        let (wake_tx, wake_rx) = unbounded::<()>();
        let mut options = SessionOptions {
            colors: settings.colors.clone(),
            scrollback: settings.scrollback,
            cwd: cwd.clone(),
            command,
            ..Default::default()
        };
        if let Some(lang) = crate::platform::default_lang() {
            options.env.push(("LANG".to_owned(), lang));
        }
        // Agents started in this pane report it back with their hook events.
        options
            .env
            .push((chda_core::agents::PANE_ENV.to_owned(), pane_id.to_string()));
        if let (Some(shell), Some(data_dir)) = (login_shell(), default_data_dir())
            && let Ok(launch) = launch_for(settings.shell_integration, &shell, &data_dir)
        {
            // A launch with arguments replaces the default login shell
            // command; its environment is useless without them, so another
            // program (an agent) gets neither.
            if launch.args.is_empty() {
                options.env.extend(launch.env);
            } else if options.command.is_none() {
                let mut command = vec![shell.to_string_lossy().into_owned()];
                command.extend(launch.args);
                options.command = Some(command);
                options.env.extend(launch.env);
            }
        }
        let session = Session::spawn(options, events_tx, move || {
            let _ = wake_tx.unbounded_send(());
        })
        .expect("failed to start the shell");
        let frame = session.frame();

        Self::drive(wake_rx, window, cx);
        Self::blink(cx);
        Self::poll_cwd(window, cx);

        let focus_handle = cx.focus_handle();
        cx.on_focus(&focus_handle, window, |this, _, _| {
            this.focused = true;
            this.session.focus(true)
        })
        .detach();
        cx.on_blur(&focus_handle, window, |this, _, _| {
            this.focused = false;
            this.session.focus(false)
        })
        .detach();
        cx.observe_window_activation(window, |this, window, _| {
            this.window_active = window.is_window_active();
        })
        .detach();

        Self {
            session,
            events,
            frame,
            focus_handle,
            settings,
            grid: (Size { cols: 80, rows: 24 }, 0, 0),
            scroll_px: 0.0,
            marked_text: None,
            cursor_bounds: None,
            geometry: None,
            layout_cache: None,
            blink_on: true,
            blink_epoch: 0,
            focused: false,
            window_active: window.is_window_active(),
            osc7_seen: false,
            cwd,
            last_activity_sent: 0,
            search: None,
            hover_cell: None,
            cmd_held: false,
            hovered_link: None,
        }
    }

    /// Ask the OS for the shell's directory until the shell starts
    /// reporting it itself.
    fn poll_cwd(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(CWD_POLL).await;
                let keep_going = this.update(cx, |view, _| {
                    if !view.osc7_seen {
                        view.session.query_cwd();
                    }
                    !view.osc7_seen
                });
                if !keep_going.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    fn set_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        if self.cwd.as_ref() != Some(&cwd) {
            self.cwd = Some(cwd.clone());
            cx.emit(TerminalEvent::Cwd(cwd));
        }
    }

    /// Toggle the cursor phase while the terminal asks for a blinking cursor.
    /// Runs without window access: an `update_in` would redraw the window
    /// every tick even when nothing changed.
    fn blink(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut epoch = 0;
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this.update(cx, |view, cx| {
                    let should_blink = view.window_active
                        && view.focused
                        && view.frame.cursor.is_some_and(|c| c.blinking);
                    if view.blink_epoch != epoch {
                        // Input happened: stay visible for a full interval.
                        epoch = view.blink_epoch;
                        if !view.blink_on {
                            view.blink_on = true;
                            cx.notify();
                        }
                    } else if should_blink {
                        view.blink_on = !view.blink_on;
                        cx.notify();
                    } else if !view.blink_on {
                        view.blink_on = true;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn touch(&mut self) {
        self.blink_epoch += 1;
        self.blink_on = true;
    }

    /// Pump session events into the view whenever the terminal thread wakes us.
    fn drive(mut wake_rx: UnboundedReceiver<()>, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            while wake_rx.next().await.is_some() {
                if this
                    .update_in(cx, |view, window, cx| view.drain_events(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn note_activity(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        if now.saturating_sub(self.last_activity_sent) >= 1000 {
            self.last_activity_sent = now;
            cx.emit(TerminalEvent::Activity(now));
        }
    }

    fn drain_events(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Frame => {
                    self.frame = self.session.frame();
                    self.update_link(cx);
                    self.note_activity(cx);
                    cx.notify();
                }
                Event::TitleChanged(title) => cx.emit(TerminalEvent::Title(title)),
                Event::PwdChanged(raw) => {
                    if let Some(path) = parse_pwd_report(&raw) {
                        self.osc7_seen = true;
                        self.set_cwd(path, cx);
                    }
                }
                Event::Cwd(Some(path)) => self.set_cwd(path, cx),
                Event::Cwd(None) => {}
                Event::Bell => cx.emit(TerminalEvent::Bell),
                Event::PromptShown => cx.emit(TerminalEvent::Prompt),
                Event::ClipboardWrite(text) | Event::SelectionText(Some(text)) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text))
                }
                Event::SelectionText(None) => {}
                Event::Search(status) => {
                    if let Some(bar) = &mut self.search {
                        bar.status = status;
                        cx.notify();
                    }
                }
                Event::Exited(_) => cx.emit(TerminalEvent::Exited),
            }
        }
    }

    /// Apply reloaded settings. Font, colors and padding change at once;
    /// scrollback size and shell integration apply to new panes.
    pub fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        if settings.colors != self.settings.colors {
            self.session.set_colors(settings.colors.clone());
        }
        self.settings = settings;
        self.layout_cache = None;
        cx.notify();
    }

    /// Scroll to the last shell prompt (where an agent was started).
    pub fn jump_to_last_prompt(&self) {
        self.session.jump_to_last_prompt();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    pub fn frame(&self) -> Arc<Frame> {
        Arc::clone(&self.frame)
    }

    /// Called by the element on every layout; tells the session when the
    /// grid or the cell size (font size changes) differs from the last one.
    pub fn set_grid(&mut self, grid: Size, cell_width: Pixels, line_height: Pixels) {
        let next = (
            grid,
            f32::from(cell_width).round() as u32,
            f32::from(line_height).round() as u32,
        );
        if next == self.grid {
            return;
        }
        self.grid = next;
        self.session.resize(next.0, next.1, next.2);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keys typed into the search bar bubble up here; they are not ours.
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let ks = &event.keystroke;
        // Command shortcuts belong to the app, never to the shell.
        if ks.modifiers.platform {
            return;
        }
        // Plain characters arrive through the input handler (IME path) when
        // the platform prefers it; sending them here too would double them.
        if event.prefer_character_input && ks.key_char.is_some() {
            return;
        }
        let Some(key) = key_code(ks) else {
            return;
        };
        self.touch();
        self.note_activity(cx);
        let text = ks
            .key_char
            .as_deref()
            .filter(|t| {
                t.chars()
                    .all(|c| !c.is_control() && !('\u{E000}'..='\u{F8FF}').contains(&c))
            })
            .map(str::to_owned);
        self.session.key(KeyInput {
            action: if event.is_held {
                KeyAction::Repeat
            } else {
                KeyAction::Press
            },
            key,
            mods: modifiers(ks.modifiers),
            text,
        });
        cx.stop_propagation();
    }

    /// Send committed text (typed characters or a finished IME composition).
    fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.marked_text = None;
        self.touch();
        if !text.is_empty() {
            self.session.text(text.to_owned());
        }
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.session.paste(text);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, _: &mut Context<Self>) {
        self.session.copy_selection();
    }

    fn jump_prev_prompt(&mut self, _: &JumpToPrevPrompt, _: &mut Window, _: &mut Context<Self>) {
        self.session.jump_to_prompt(-1);
    }

    fn jump_next_prompt(&mut self, _: &JumpToNextPrompt, _: &mut Window, _: &mut Context<Self>) {
        self.session.jump_to_prompt(1);
    }

    /// Open the search bar, or focus it again when it is already open.
    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_none() {
            let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
            let bg = hsla(self.settings.colors.background.unwrap_or_default());
            let input = cx.new(|cx| {
                TextInput::new("Find", fg, crate::workspace_view::blend(bg, fg, 0.12), cx)
            });
            let observe = cx.observe(&input, |this, input, cx| {
                let text = input.read(cx).text().to_owned();
                if let Some(bar) = &mut this.search
                    && bar.query.text != text
                {
                    bar.query.text = text;
                    this.run_search(cx);
                }
            });
            // Enter and escape can reach the field as text input instead of
            // key bindings (macOS marks some keys as character input).
            let events =
                cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                    TextInputEvent::Submit(_) => this.find_next(&FindNext, window, cx),
                    TextInputEvent::Cancel => this.close_find(&CloseFind, window, cx),
                });
            self.search = Some(SearchBar {
                input,
                _observe: observe,
                _events: events,
                query: SearchQuery::default(),
                status: SearchStatus::default(),
            });
        }
        if let Some(bar) = &self.search {
            let handle = bar.input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let Some(bar) = &mut self.search else {
            return;
        };
        if bar.query.text.is_empty() {
            bar.status = SearchStatus::default();
            self.session.search(None);
        } else {
            self.session.search(Some(bar.query.clone()));
        }
        cx.notify();
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, _: &mut Context<Self>) {
        if self.search.is_some() {
            self.session.search_step(true);
        }
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, _: &mut Context<Self>) {
        if self.search.is_some() {
            self.session.search_step(false);
        }
    }

    fn close_find(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.take().is_some() {
            self.session.search(None);
        }
        self.focus(window, cx);
        cx.notify();
    }

    fn toggle_find_case(&mut self, _: &ToggleFindCase, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(bar) = &mut self.search {
            bar.query.case_sensitive = !bar.query.case_sensitive;
            self.run_search(cx);
        }
    }

    fn toggle_find_regex(&mut self, _: &ToggleFindRegex, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(bar) = &mut self.search {
            bar.query.regex = !bar.query.regex;
            self.run_search(cx);
        }
    }

    fn render_search(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let bar = self.search.as_ref()?;
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let toggle = |id: &'static str, label: &'static str, on: bool, tip: &'static str| {
            div()
                .id(id)
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(if on { fg } else { fg.opacity(0.45) })
                .when(on, |d| d.bg(fg.opacity(0.18)))
                .hover(|s| s.bg(fg.opacity(0.12)))
                .tooltip(crate::tooltip::text(tip))
                .child(label)
        };
        let button = |id: &'static str, label: &'static str, tip: &'static str| {
            div()
                .id(id)
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(fg.opacity(0.12)))
                .tooltip(crate::tooltip::text(tip))
                .child(label)
        };
        let status = match (&bar.status.error, bar.status.current) {
            (Some(_), _) => "invalid".to_owned(),
            (None, _) if bar.query.text.is_empty() => String::new(),
            (None, Some(i)) => format!("{i}/{}", bar.status.total),
            (None, None) => "no matches".to_owned(),
        };
        let invalid =
            bar.status.error.is_some() || (!bar.query.text.is_empty() && bar.status.total == 0);
        Some(
            div()
                .absolute()
                .top_1()
                .right_2()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .p_1()
                .rounded_md()
                .bg(crate::workspace_view::blend(bg, fg, 0.08))
                .border_1()
                .border_color(fg.opacity(0.2))
                .shadow_md()
                .text_sm()
                .text_color(fg)
                .key_context("SearchBar")
                .on_action(cx.listener(Self::toggle_find_case))
                .on_action(cx.listener(Self::toggle_find_regex))
                .on_action(cx.listener(Self::close_find))
                .child(
                    div()
                        .w(px(200.0))
                        .overflow_hidden()
                        .child(bar.input.clone()),
                )
                .child(
                    toggle(
                        "find-case",
                        "Aa",
                        bar.query.case_sensitive,
                        "Match case (alt-c)",
                    )
                    .on_click(
                        cx.listener(|this, _, w, cx| this.toggle_find_case(&ToggleFindCase, w, cx)),
                    ),
                )
                .child(
                    toggle(
                        "find-regex",
                        ".*",
                        bar.query.regex,
                        "Regular expression (alt-r)",
                    )
                    .on_click(
                        cx.listener(|this, _, w, cx| {
                            this.toggle_find_regex(&ToggleFindRegex, w, cx)
                        }),
                    ),
                )
                .child(
                    div()
                        .min_w(px(64.0))
                        .text_xs()
                        .text_color(if invalid {
                            gpui::rgb(0xf38ba8).into()
                        } else {
                            fg.opacity(0.7)
                        })
                        .child(status),
                )
                .child(
                    button("find-older", "\u{2191}", "Older match (enter)")
                        .on_click(cx.listener(|this, _, w, cx| this.find_next(&FindNext, w, cx))),
                )
                .child(
                    button("find-newer", "\u{2193}", "Newer match (shift-enter)").on_click(
                        cx.listener(|this, _, w, cx| this.find_previous(&FindPrevious, w, cx)),
                    ),
                )
                .child(
                    button("find-close", "\u{2715}", "Close (esc)")
                        .on_click(cx.listener(|this, _, w, cx| this.close_find(&CloseFind, w, cx))),
                )
                .into_any_element(),
        )
    }

    /// Translate a window position into a grid cell and grid-relative pixels.
    fn mouse_input(
        &self,
        action: MouseAction,
        button: MouseButton,
        position: Point<Pixels>,
        mods: GpuiModifiers,
    ) -> Option<MouseInput> {
        let g = self.geometry?;
        let button = match button {
            MouseButton::Left => TermButton::Left,
            MouseButton::Middle => TermButton::Middle,
            MouseButton::Right => TermButton::Right,
            _ => return None,
        };
        let px_x = f32::from(position.x - g.origin.x).max(0.0);
        let px_y = f32::from(position.y - g.origin.y).max(0.0);
        let cell_x = ((px_x / f32::from(g.cell_width)) as u16).min(g.size.cols.saturating_sub(1));
        let cell_y = ((px_y / f32::from(g.line_height)) as u16).min(g.size.rows.saturating_sub(1));
        Some(MouseInput {
            action,
            button,
            mods: modifiers(mods),
            cell_x,
            cell_y,
            px_x: px_x as u32,
            px_y: px_y as u32,
        })
    }

    /// Recompute the link under the mouse; only shown while cmd is held.
    fn update_link(&mut self, cx: &mut Context<Self>) {
        let link = match (self.cmd_held, self.hover_cell) {
            (true, Some((x, y))) => chda_term::link_at(&self.frame, x, y)
                .and_then(|l| Some((self.resolve_link(l.target)?, l.cells)))
                .map(|(open, cells)| HoveredLink { cells, open }),
            _ => None,
        };
        if link != self.hovered_link {
            self.hovered_link = link;
            cx.notify();
        }
    }

    /// URLs open as they are; paths must exist, relative to the shell's
    /// directory or the home directory for `~`.
    fn resolve_link(&self, target: LinkTarget) -> Option<LinkOpen> {
        match target {
            LinkTarget::Url(url) => Some(LinkOpen::Url(url)),
            LinkTarget::Path { path, line, column } => {
                let p = if let Some(rest) = path.strip_prefix("~/") {
                    PathBuf::from(std::env::var_os("HOME")?).join(rest)
                } else {
                    let p = PathBuf::from(&path);
                    if p.is_absolute() {
                        p
                    } else {
                        self.cwd.as_ref()?.join(p)
                    }
                };
                p.exists().then_some(LinkOpen::Path {
                    path: p,
                    line,
                    column,
                })
            }
        }
    }

    /// Track the cell under the mouse and whether cmd is held.
    fn track_hover(
        &mut self,
        position: Point<Pixels>,
        mods: GpuiModifiers,
        cx: &mut Context<Self>,
    ) {
        self.cmd_held = mods.platform;
        self.hover_cell = self.geometry.and_then(|g| {
            let x = f32::from(position.x - g.origin.x) / f32::from(g.cell_width);
            let y = f32::from(position.y - g.origin.y) / f32::from(g.line_height);
            (x >= 0.0 && y >= 0.0 && x < f32::from(g.size.cols) && y < f32::from(g.size.rows))
                .then_some((x as u16, y as u16))
        });
        self.update_link(cx);
    }

    fn modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cmd_held != event.modifiers.platform {
            self.cmd_held = event.modifiers.platform;
            self.update_link(cx);
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            self.focus(window, cx);
            cx.emit(TerminalEvent::Focused);
        }
        self.track_hover(event.position, event.modifiers, cx);
        if event.button == MouseButton::Left
            && event.modifiers.platform
            && let Some(link) = self.hovered_link.clone()
        {
            match link.open {
                LinkOpen::Url(url) => cx.open_url(&url),
                LinkOpen::Path { path, line, column } => {
                    cx.emit(TerminalEvent::OpenPath { path, line, column })
                }
            }
            return;
        }
        let click_count = event.click_count.clamp(1, 3) as u8;
        if let Some(input) = self.mouse_input(
            MouseAction::Down { click_count },
            event.button,
            event.position,
            event.modifiers,
        ) {
            self.session.mouse(input);
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        if let Some(input) = self.mouse_input(
            MouseAction::Up,
            event.button,
            event.position,
            event.modifiers,
        ) {
            self.session.mouse(input);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.track_hover(event.position, event.modifiers, cx);
        let (action, button) = match event.pressed_button {
            Some(b) => (MouseAction::Drag, b),
            None => (MouseAction::Move, MouseButton::Left),
        };
        if let Some(input) = self.mouse_input(action, button, event.position, event.modifiers) {
            self.session.mouse(input);
        }
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, _: &mut Context<Self>) {
        let Some(g) = self.geometry else {
            return;
        };
        let line_height = g.line_height;
        match event.touch_phase {
            TouchPhase::Started => self.scroll_px = 0.0,
            TouchPhase::Ended | TouchPhase::Cancelled => return,
            TouchPhase::Moved => {}
        }
        let before = (self.scroll_px / f32::from(line_height)) as i32;
        self.scroll_px += f32::from(event.delta.pixel_delta(line_height).y);
        let after = (self.scroll_px / f32::from(line_height)) as i32;
        let lines = after - before;
        if lines != 0 {
            let px_x = f32::from(event.position.x - g.origin.x).max(0.0) as u32;
            let px_y = f32::from(event.position.y - g.origin.y).max(0.0) as u32;
            // Wheel up (positive delta) moves the viewport towards history.
            self.session
                .scroll(-(lines as isize), modifiers(event.modifiers), px_x, px_y);
        }
    }
}

fn modifiers(m: GpuiModifiers) -> Modifiers {
    Modifiers {
        shift: m.shift,
        alt: m.alt,
        ctrl: m.control,
        meta: m.platform,
    }
}

/// Map a GPUI keystroke to the terminal's logical key.
fn key_code(ks: &Keystroke) -> Option<KeyCode> {
    let key = ks.key.as_str();
    Some(match key {
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Escape,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Space,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::ArrowUp,
        "down" => KeyCode::ArrowDown,
        "left" => KeyCode::ArrowLeft,
        "right" => KeyCode::ArrowRight,
        _ => {
            if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
                && key.len() > 1
            {
                return Some(KeyCode::F(n));
            }
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return None,
            }
        }
    })
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // An empty selection keeps the IME candidate window anchored at the cursor.
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|t| 0..t.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked_text.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_text(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.settings.padding;
        let search = self.render_search(cx);
        div()
            .size_full()
            .relative()
            .bg(crate::terminal_element::hsla(
                self.settings.colors.background.unwrap_or_default(),
            ))
            .pl(px(p.left as f32))
            .pr(px(p.right as f32))
            .pt(px(p.top as f32))
            .pb(px(p.bottom as f32))
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::jump_prev_prompt))
            .on_action(cx.listener(Self::jump_next_prompt))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(TerminalElement::new(cx.entity(), self.focus_handle.clone()))
            .children(search)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(key: &str) -> Keystroke {
        Keystroke {
            modifiers: Default::default(),
            key: key.into(),
            key_char: None,
        }
    }

    #[test]
    fn keystroke_names_map_to_key_codes() {
        assert_eq!(key_code(&ks("a")), Some(KeyCode::Char('a')));
        assert_eq!(key_code(&ks("/")), Some(KeyCode::Char('/')));
        assert_eq!(key_code(&ks("enter")), Some(KeyCode::Enter));
        assert_eq!(key_code(&ks("pageup")), Some(KeyCode::PageUp));
        assert_eq!(key_code(&ks("f12")), Some(KeyCode::F(12)));
        assert_eq!(key_code(&ks("f")), Some(KeyCode::Char('f')));
        assert_eq!(key_code(&ks("capslock")), None);
    }
}
