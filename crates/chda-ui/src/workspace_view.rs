//! The window's root view: a tab bar and the split tree of terminal panes.

use std::collections::HashMap;

use chda_core::{Axis, Direction, Node, PaneId, Workspace};
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, Render, Subscription, Window,
    actions, div, prelude::*, px, relative,
};

use crate::platform;
use crate::settings::Settings;
use crate::terminal_element::hsla;
use crate::terminal_view::{TerminalEvent, TerminalView};

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
        Quit,
    ]
);

/// Fraction of the tab a keyboard resize moves the divider by.
const RESIZE_STEP: f32 = 0.05;

pub struct WorkspaceView {
    settings: Settings,
    ws: Workspace,
    panes: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    focus_handle: FocusHandle,
}

impl WorkspaceView {
    pub fn new(settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            settings,
            ws: Workspace::new(),
            panes: HashMap::new(),
            focus_handle: cx.focus_handle(),
        };
        this.new_tab(&NewTab, window, cx);
        this
    }

    /// Directory new panes start in: the focused pane's.
    fn inherited_cwd(&self) -> Option<std::path::PathBuf> {
        self.ws
            .focused_pane()
            .and_then(|p| self.ws.pane(p))
            .and_then(|info| info.cwd.clone())
    }

    fn open_pane(
        &mut self,
        pane: PaneId,
        cwd: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(info) = self.ws.pane_mut(pane) {
            info.cwd = cwd.clone();
        }
        let settings = self.settings.clone();
        let view = cx.new(|cx| TerminalView::new(settings, cwd, window, cx));
        let sub = cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
            this.on_pane_event(pane, event, window, cx)
        });
        self.panes.insert(pane, (view, sub));
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
            }
        }
        self.sync_title(window);
        cx.notify();
    }

    /// Give keyboard focus to the workspace's focused pane.
    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.ws.focused_pane()
            && let Some((view, _)) = self.panes.get(&pane)
        {
            let handle = view.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
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
        let (_, pane) = self.ws.new_tab();
        self.open_pane(pane, cwd, window, cx);
        self.focus_active(window, cx);
    }

    fn close_surface(&mut self, _: &CloseSurface, window: &mut Window, cx: &mut Context<Self>) {
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
    }

    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.inherited_cwd();
        if let Some(pane) = self.ws.split(axis) {
            self.open_pane(pane, cwd, window, cx);
            self.focus_active(window, cx);
        }
    }

    fn goto_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws.activate_tab(index) {
            self.focus_active(window, cx);
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
}

/// Mix `a` towards `b` by `t`.
fn blend(a: Hsla, b: Hsla, t: f32) -> Hsla {
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
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_surface))
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
            .children(self.render_tab_bar(cx))
            .child(div().flex_1().min_h_0().w_full().child(content))
    }
}
