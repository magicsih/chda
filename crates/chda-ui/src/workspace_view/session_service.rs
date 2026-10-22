use super::*;

fn contains(node: &chda_core::SavedNode, pane: u64) -> bool {
    match node {
        chda_core::SavedNode::Pane { pane: Some(id), .. } => *id == pane,
        chda_core::SavedNode::Split { first, second, .. } => {
            contains(first, pane) || contains(second, pane)
        }
        _ => false,
    }
}
fn window_contains(window: &chda_core::SavedWindow, pane: u64) -> bool {
    window.tabs.iter().any(|t| match &t.content {
        chda_core::SavedTabContent::Terminal { root, .. } => contains(root, pane),
        _ => false,
    })
}

impl WorkspaceView {
    pub(super) fn watch_service(
        client: Option<chda_session::Client>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = client else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let client = client.clone();
                let state = cx.background_spawn(async move {
                    crate::platform::sessions::upgrade_if_idle(&client)?;
                    client.state()
                }).await;
                if this
                    .update(cx, |view, cx| match state {
                        Ok(state) => {
                            let mut registry = view.env.windows.borrow_mut();
                            let changed = registry.service_error != state.save_error;
                            registry.service_error = state.save_error;
                            registry.service_saved_at = state.layout_saved_at;
                            registry.service_records = state.sessions;
                            drop(registry);
                            view.sync_service_sidebar(cx);
                            if changed { cx.notify(); }
                        }
                        Err(error) => {
                            let error = Some(format!("Session service connection lost: {error}. Reopen chda to reconnect; input is not replayed."));
                            let mut registry = view.env.windows.borrow_mut();
                            if registry.service_error != error {
                                registry.service_error = error;
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
        })
        .detach();
    }

    pub(super) fn sync_service_sidebar(&self, cx: &mut Context<Self>) {
        let registry = self.env.windows.borrow();
        let rows = registry
            .service_records
            .iter()
            .filter(|r| {
                !r.closed
                    && !registry
                        .entries
                        .iter()
                        .any(|e| window_contains(&e.saved, r.pane))
            })
            .map(|r| {
                let title = if r.title.is_empty() {
                    r.cwd
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("Session {}", r.pane))
                } else {
                    r.title.clone()
                };
                let status = if r.running {
                    "Detached — reconnect".into()
                } else {
                    format!(
                        "Previous output — {}",
                        r.exit
                            .as_ref()
                            .map(|s| format!("exited {}", s.code))
                            .unwrap_or_else(|| "choose how to resume".into())
                    )
                };
                (r.pane, title, status)
            })
            .collect::<Vec<_>>();
        self.sidebar.update(cx, |s, cx| {
            if s.detached_sessions != rows {
                s.detached_sessions = rows;
                cx.notify();
            }
        });
    }

    pub(super) fn reconnect_service_session(
        &mut self,
        pane: u64,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self
            .env
            .windows
            .borrow()
            .entries
            .iter()
            .find(|e| window_contains(&e.saved, pane))
            .cloned()
        {
            let _ = entry
                .window
                .update(cx, |_, window, _| window.activate_window());
            return;
        }
        let saved = {
            let mut registry = self.env.windows.borrow_mut();
            if let Some(index) = registry
                .detached
                .iter()
                .position(|w| window_contains(w, pane))
            {
                Some(registry.detached.remove(index))
            } else {
                None
            }
        };
        if let Some(saved) = saved {
            crate::open_workspace_window(Some(saved), self.env.clone(), cx);
            return;
        }
        let Some(record) = self
            .env
            .windows
            .borrow()
            .service_records
            .iter()
            .find(|r| r.pane == pane && !r.closed)
            .cloned()
        else {
            return;
        };
        let agent = record.agent.as_ref();
        let saved = chda_core::SavedWindow {
            bounds: None,
            active: 0,
            tabs: vec![chda_core::SavedTab {
                title: (!record.title.is_empty()).then_some(record.title.clone()),
                content: chda_core::SavedTabContent::Terminal {
                    root: chda_core::SavedNode::Pane {
                        last_activity: 0,
                        cwd: record.cwd.clone(),
                        repo: None,
                        agent: agent.map(|e| e.agent.clone()),
                        session: agent.map(|e| e.session_id.clone()),
                        agent_cwd: agent.map(|e| e.cwd.clone()),
                        launch: record.launch.map(Box::new),
                        run: None,
                        previous_run: None,
                        pane: Some(record.pane),
                    },
                    focused: 0,
                    zoomed: None,
                },
            }],
        };
        crate::open_workspace_window(Some(saved), self.env.clone(), cx);
    }
}
