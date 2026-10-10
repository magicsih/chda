use super::*;
use chda_core::agents::{ChildActivity, ChildState};

impl WorkspaceView {
    fn parent_identity(&self, pane: PaneId) -> Option<(String, String)> {
        let info = self.ws.pane(pane)?;
        info.agent_session
            .as_ref()
            .map(|s| (s.agent.clone(), s.session.clone()))
            .or_else(|| {
                let launch = info.agent_launch.as_ref()?;
                Some((
                    launch.agent.as_str().to_owned(),
                    launch.session.as_ref()?.0.clone(),
                ))
            })
    }
    pub(super) fn sync_children(&mut self, cx: &mut Context<Self>) {
        let board = &self.env.windows.borrow().children;
        let mut groups = Vec::new();
        for tab in self.ws.tabs() {
            for pane in tab.panes() {
                let Some((agent, session)) = self.parent_identity(pane) else {
                    continue;
                };
                let children = board.children(&agent, &session);
                if children.is_empty() {
                    continue;
                }
                groups.push(crate::sidebar_view::ChildGroup {
                    pane,
                    tab: tab.id,
                    agent,
                    session,
                    label: self.ws.tab_title(tab),
                    children,
                });
            }
        }
        self.sidebar.update(cx, |s, cx| {
            if s.child_groups != groups {
                s.child_groups = groups;
                cx.notify();
            }
        });
    }
    pub(super) fn apply_child_activity(&mut self, event: ChildActivity, cx: &mut Context<Self>) {
        let parent = self.ws.tabs().iter().flat_map(|t| t.panes()).find(|p| {
            self.parent_identity(*p)
                .is_some_and(|(agent, session)| agent == event.agent && session == event.parent)
        });
        let Some(parent) = parent else { return };
        if self
            .env
            .windows
            .borrow()
            .started_at
            .is_some_and(|at| event.observed_at < at)
        {
            return;
        }
        let changed = self.env.windows.borrow_mut().children.apply(event.clone());
        if changed && event.child.state == ChildState::WaitingInput {
            self.notifications.record(
                event.observed_at,
                Severity::Info,
                format!(
                    "{} child {} is waiting for input",
                    event.agent,
                    event.child.label.as_deref().unwrap_or(&event.child.id)
                ),
                self.notification_pane_context(Some(parent)),
                None,
            );
        }
        self.sync_children(cx);
        let entries = self.env.windows.borrow().entries.clone();
        for entry in entries {
            if entry.view != self.self_weak {
                // A hook may have been routed here during another window's
                // update. Refresh it after the current entity update ends.
                cx.defer(move |cx| {
                    let _ = entry.view.update(cx, |v, cx| v.sync_children(cx));
                });
            }
        }
        if changed {
            cx.notify();
        }
    }
    pub(super) fn poll_children(&mut self, cx: &mut Context<Self>) {
        if !self.env.collect_telemetry {
            return;
        }
        let registry = self.env.windows.clone();
        let mut state = registry.borrow_mut();
        if state.child_polling || state.update.frozen || state.update.preparing || state.quitting {
            return;
        }
        let mut parents: Vec<String> = state
            .entries
            .iter()
            .flat_map(|e| e.sessions.values())
            .filter(|(agent, _, live)| agent == "codex" && *live)
            .map(|(_, session, _)| session.clone())
            .collect();
        parents.sort();
        parents.dedup();
        if parents.is_empty() {
            return;
        }
        let Some(executable) = self
            .adapters
            .iter()
            .find(|a| a.id() == AgentId::Codex)
            .and_then(|a| a.executable(&self.env.home(), &self.env.search_path()))
        else {
            return;
        };
        let environment = self.env.pane_env.clone();
        state.child_polling = true;
        let operation = state.update.operation();
        drop(state);
        let observed_at = now_ms();
        let queries = parents.clone();
        let task = cx.background_spawn(async move {
            chda_core::agents::read_codex_children(&executable, &environment, &queries, observed_at)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            registry.borrow_mut().child_polling = false;
            // Query completion never navigates or focuses a terminal.
            let entries = registry.borrow().entries.clone();
            let still_live: HashSet<_> = entries
                .iter()
                .flat_map(|e| e.sessions.values())
                .filter(|(a, _, live)| a == "codex" && *live)
                .map(|(_, s, _)| s.clone())
                .collect();
            let events = result.unwrap_or_default();
            {
                let mut registry = registry.borrow_mut();
                for parent in &parents {
                    if !still_live.contains(parent) {
                        continue;
                    }
                    for mut old in registry.children.children("codex", parent) {
                        if old.child.state != ChildState::Ended
                            && !events
                                .iter()
                                .any(|e| e.parent == *parent && e.child.id == old.child.id)
                        {
                            old.child.state = ChildState::Unknown;
                            old.child.started = false;
                            old.observed_at = observed_at;
                            registry.children.apply(old);
                        }
                    }
                }
            }
            for event in events {
                if still_live.contains(&event.parent)
                    && let Some(entry) = entries.iter().find(|e| {
                        e.sessions
                            .values()
                            .any(|(a, s, _)| a == "codex" && s == &event.parent)
                    })
                {
                    let _ = entry
                        .view
                        .update(cx, |v, cx| v.apply_child_activity(event, cx));
                }
            }
            let _ = this.update(cx, |v, cx| v.sync_children(cx));
            for entry in entries {
                let _ = entry.view.update(cx, |v, cx| v.sync_children(cx));
            }
            drop(operation);
        })
        .detach();
    }
    pub(super) fn open_child_details(
        &mut self,
        child: ChildActivity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A dedicated pane requires an exact provider/session match in the app registry.
        let matches: Vec<_> = self
            .env
            .windows
            .borrow()
            .entries
            .iter()
            .flat_map(|entry| {
                entry
                    .sessions
                    .iter()
                    .filter(|(_, (a, s, live))| a == &child.agent && s == &child.child.id && *live)
                    .map(|(p, _)| (entry.clone(), *p))
            })
            .collect();
        if matches.len() == 1 {
            let (entry, pane) = matches[0].clone();
            if entry.view == self.self_weak {
                self.on_sidebar_event(SidebarEvent::FocusPane(pane), window, cx);
            } else {
                let _ = entry.window.update(cx, |_, window, cx| {
                    window.activate_window();
                    let _ = entry.view.update(cx, |v, cx| {
                        v.on_sidebar_event(SidebarEvent::FocusPane(pane), window, cx)
                    });
                });
            }
            return;
        }
        self.child_focus = window.focused(cx).map(|f| f.downgrade());
        self.child_detail = Some(child);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
    pub(super) fn close_child_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.child_detail = None;
        if let Some(focus) = self.child_focus.take().and_then(|f| f.upgrade()) {
            window.focus(&focus, cx);
        } else {
            self.refocus_terminal(window, cx);
        }
        cx.notify();
    }
    pub(super) fn render_child_details(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let child = self.child_detail.as_ref()?;
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        Some(div().absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(0.4)).child(
            div().id("child-details").debug_selector(|| "child-details".into()).w(px(460.0)).max_w(relative(0.95)).max_h(relative(0.9)).overflow_y_scroll().p_4().flex().flex_col().gap_2().bg(bg).text_color(fg).rounded_md()
                .child("Child agent activity")
                .child(format!("{} · {}", child.agent, child.child.label.as_deref().unwrap_or("Unnamed child")))
                .child(format!("{} · last report {} ago", child.child.state.label(), chda_core::activity_age(now_ms(), child.observed_at)))
                .child(format!("Child: {}", child.child.id))
                .child(format!("Parent: {}", child.parent))
                .child("No unique dedicated pane is known. This view does not navigate to another conversation.")
                .child(div().id("close-child-details").cursor_pointer().p_2().child("Close").on_click(cx.listener(|v, _, window, cx| v.close_child_details(window, cx))))
        ).into_any_element())
    }
}
