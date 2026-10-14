//! Reorder tabs and move running panes between tabs and windows (#248).
//! A pane keeps its process, scrollback, agent status, child activity and
//! conversation identity wherever it goes; pane ids stay the same, so
//! Sessions rows, notifications and "jump to waiting agent" follow it.
use super::*;

/// A tab being dragged to a new place in the tab bar.
#[derive(Clone)]
pub(crate) struct TabDrag {
    pub(super) tab: TabId,
    pub(super) title: String,
}

/// What follows the mouse while a tab is dragged.
pub(super) struct TabDragPreview {
    pub(super) title: String,
    pub(super) fg: Hsla,
    pub(super) bg: Hsla,
}

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded_sm()
            .bg(self.bg)
            .border_1()
            .border_color(self.fg.opacity(0.3))
            .text_sm()
            .text_color(self.fg)
            .shadow_md()
            .child(self.title.clone())
    }
}

/// Where a moved pane goes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaneDestination {
    NewTab,
    Tab(TabId),
    /// Another window, or a new one.
    Window(Option<gpui::WindowId>),
}

impl WorkspaceView {
    /// Drop tab `dragged` where tab `onto` is, shifting the others.
    pub(super) fn drop_tab(&mut self, dragged: TabId, onto: TabId, cx: &mut Context<Self>) {
        if dragged == onto {
            return;
        }
        let Some(to) = self.ws.tabs().iter().position(|t| t.id == onto) else {
            return;
        };
        if self.ws.move_tab(dragged, to) {
            self.sync_panes(cx);
            cx.notify();
        }
    }

    /// The pane menu's move choices for `pane`.
    pub(super) fn pane_move_items(&self, pane: PaneId) -> Vec<(String, MenuAction)> {
        let Some(own) = self.ws.tabs().iter().find(|t| t.panes().contains(&pane)) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        let item = |label: String, to| (label, MenuAction::MovePane { pane, to });
        if own.panes().len() > 1 {
            items.push(item("Move to New Tab".into(), PaneDestination::NewTab));
        }
        for (i, tab) in self.ws.tabs().iter().enumerate() {
            if tab.id != own.id && tab.terminal().is_some() {
                items.push(item(
                    format!("Move to Tab {}: {}", i + 1, self.ws.tab_title(tab)),
                    PaneDestination::Tab(tab.id),
                ));
            }
        }
        // Another window would leave this one empty, which then closes.
        items.push(item(
            "Move to New Window".into(),
            PaneDestination::Window(None),
        ));
        let windows: Vec<_> = self.env.windows.borrow().entries.clone();
        for (i, entry) in windows.iter().enumerate() {
            if entry.view != self.self_weak {
                items.push(item(
                    format!("Move to Window {}", i + 1),
                    PaneDestination::Window(Some(entry.window.window_id())),
                ));
            }
        }
        items
    }

    /// Open the pane menu at `position`.
    pub(super) fn open_pane_menu(
        &mut self,
        pane: PaneId,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let items = self.pane_move_items(pane);
        if !items.is_empty() {
            self.context_menu = Some(ContextMenu { position, items });
            cx.notify();
        }
    }

    pub(crate) fn move_pane(
        &mut self,
        pane: PaneId,
        to: PaneDestination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self
            .ws
            .tabs()
            .iter()
            .find(|t| t.panes().contains(&pane))
            .map(|t| (t.id, t.panes().len()))
        else {
            return;
        };
        match to {
            PaneDestination::NewTab => {
                if from.1 == 1 {
                    return;
                }
                let Some((info, _)) = self.ws.take_pane(pane) else {
                    return;
                };
                self.ws.insert_pane_as_tab(pane, info);
            }
            PaneDestination::Tab(tab) => {
                if tab == from.0 {
                    return;
                }
                let aspect = self.tab_aspect(window);
                let Some((info, _)) = self.ws.take_pane(pane) else {
                    return;
                };
                if !self
                    .ws
                    .insert_pane_into_tab(pane, info.clone(), tab, aspect)
                {
                    self.ws.insert_pane_as_tab(pane, info);
                }
            }
            PaneDestination::Window(target) => {
                return self.move_pane_to_window(pane, target, window, cx);
            }
        }
        self.sync_panes(cx);
        self.focus_active(window, cx);
    }

    /// Hand the pane's running process to another window: detach it here,
    /// adopt it there, as a session-preserving update does.
    fn move_pane_to_window(
        &mut self,
        pane: PaneId,
        target: Option<gpui::WindowId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((view, _)) = self.panes.get(&pane) else {
            return;
        };
        let detach = view.read(cx).detach();
        cx.spawn_in(window, async move |this, cx| {
            let detached = cx
                .background_spawn(async move { detach.recv_timeout(Duration::from_secs(5)) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match detached {
                Ok(Ok(detached)) => this.hand_over(pane, detached, target, window, cx),
                Ok(Err(error)) => {
                    this.notify_error(format!("Could not move the pane: {error}"));
                    cx.notify();
                }
                Err(_) => {
                    this.notify_error("Could not move the pane: it did not stop in time".into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn hand_over(
        &mut self,
        pane: PaneId,
        detached: chda_term::DetachedSession,
        target: Option<gpui::WindowId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((info, _)) = self.ws.take_pane(pane) else {
            return;
        };
        // The view's session thread has ended; dropping it leaves the
        // process to the adopting window.
        self.panes.remove(&pane);
        let size = detached.size;
        {
            let mut registry = self.env.windows.borrow_mut();
            registry.update.inherited.insert(
                pane.raw(),
                (
                    chda_core::handoff::HandoffPane::new(
                        pane.raw(),
                        size.cols,
                        size.rows,
                        Vec::new(),
                    ),
                    detached,
                ),
            );
        }
        let entry = target.and_then(|id| {
            self.env
                .windows
                .borrow()
                .entries
                .iter()
                .find(|e| e.window.window_id() == id && e.view != self.self_weak)
                .cloned()
        });
        match entry {
            Some(entry) => {
                let _ = entry.window.update(cx, |_, window, cx| {
                    window.activate_window();
                    let _ = entry
                        .view
                        .update(cx, |v, cx| v.receive_pane(pane, info, window, cx));
                });
            }
            None => {
                self.env.windows.borrow_mut().moving_pane = Some((pane, info));
                crate::open_workspace_window(None, self.env.clone(), cx);
            }
        }
        if self.ws.is_empty() {
            window.remove_window();
        } else {
            self.sync_panes(cx);
            self.focus_active(window, cx);
        }
    }

    /// Show a pane another window handed over, in a new tab, with the state
    /// it had there.
    pub(crate) fn receive_pane(
        &mut self,
        pane: PaneId,
        info: chda_core::PaneInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = info.cwd.clone();
        self.ws.insert_pane_as_tab(pane, info.clone());
        self.open_pane(pane, cwd, None, window, cx);
        // Opening a terminal resets a managed agent's state; keep the one
        // the pane brought along.
        if let Some(current) = self.ws.pane_mut(pane) {
            *current = info;
        }
        match self.panes.get(&pane) {
            Some((view, _)) => view.read(cx).commit_adoption(),
            None => self.notify_error("Could not attach the moved pane".into()),
        }
        self.sync_panes(cx);
        self.focus_active(window, cx);
    }
}
