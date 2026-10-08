use super::*;
use chda_core::ManagedRun;

fn output_name_valid(name: &str) -> bool {
    name.starts_with("previous-run-")
        && name.ends_with(".txt")
        && name.len() < 160
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.'))
}

impl WorkspaceView {
    /// Prune only output files this window owned, after the complete app
    /// manifest is saved. Another window's reference keeps its file alive.
    pub(super) fn prune_owned_outputs(&self, saved: &chda_core::SavedSession, dir: &Path) {
        fn collect(node: &chda_core::SavedNode, names: &mut HashSet<String>) {
            match node {
                chda_core::SavedNode::Pane {
                    previous_run: Some(name),
                    ..
                } => {
                    names.insert(name.clone());
                }
                chda_core::SavedNode::Split { first, second, .. } => {
                    collect(first, names);
                    collect(second, names);
                }
                _ => {}
            }
        }
        let mut references = HashSet::new();
        for window in &saved.windows {
            for tab in &window.tabs {
                if let chda_core::SavedTabContent::Terminal { root, .. } = &tab.content {
                    collect(root, &mut references);
                }
            }
        }
        self.owned_outputs.borrow_mut().retain(|pane, name| {
            if self.ws.pane(*pane).is_some()
                || references.contains(name)
                || !output_name_valid(name)
            {
                return true;
            }
            match std::fs::remove_file(dir.join("agent-output").join(name)) {
                Ok(()) => false,
                Err(e) => e.kind() != std::io::ErrorKind::NotFound,
            }
        });
    }
    pub(super) fn captured_resume_argv(
        &self,
        context: &chda_core::agents::AgentLaunchContext,
    ) -> Result<Vec<String>, String> {
        chda_core::agents::validate_resume_options(context.agent, &context.options)?;
        let session = context
            .session
            .as_ref()
            .filter(|s| !s.0.is_empty())
            .ok_or("No exact conversation ID was captured; restart is unavailable")?;
        let adapter = self
            .adapters
            .iter()
            .find(|a| a.id() == context.agent)
            .ok_or("The recorded provider is unavailable")?;
        if adapter.session_roots().is_empty() {
            return Err("This provider does not expose verifiable local transcripts; exact resume is unavailable".into());
        }
        if !adapter.has_session(session) {
            return Err(
                "The exact conversation transcript is missing; no new conversation was started"
                    .into(),
            );
        }
        self.captured_agent_argv(context)
    }

    pub(super) fn managed_exit(
        &mut self,
        pane: PaneId,
        status: Option<&chda_term::ExitStatus>,
        output: Option<&Result<String, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_focus = self
            .panes
            .get(&pane)
            .is_some_and(|(view, _)| view.read(cx).focus_handle(cx).is_focused(window));
        self.ws.end_agent(pane);
        self.initializing_panes.remove(&pane);
        self.panes.remove(&pane);
        if let Some(info) = self.ws.pane_mut(pane) {
            info.managed_run = Some(ManagedRun::Stopped {
                code: status.map(|s| s.code),
                signal: status.and_then(|s| s.signal.clone()),
                error: None,
            });
        }
        if let Some(Err(error)) = output {
            self.ws.pane_mut(pane).unwrap().previous_run = None;
            self.previous_runs.remove(&pane);
            self.notify_error(format!("Could not capture previous output: {error}"));
        }
        if let Some(Ok(output)) = output {
            let filename = self
                .ws
                .pane(pane)
                .and_then(|p| p.previous_run.clone())
                .filter(|s| output_name_valid(s))
                .unwrap_or_else(|| {
                    format!(
                        "previous-run-{}-{}-{}.txt",
                        std::process::id(),
                        pane.raw(),
                        now_ms()
                    )
                });
            let result = self
                .env
                .data_dir
                .as_ref()
                .ok_or_else(|| "No data directory for previous output".to_string())
                .and_then(|dir| {
                    ipc::write_private_atomic(
                        &dir.join("agent-output").join(&filename),
                        output.as_bytes(),
                    )
                    .map_err(|e| e.to_string())
                });
            match result {
                Ok(()) => {
                    self.owned_outputs
                        .borrow_mut()
                        .insert(pane, filename.clone());
                    self.ws.pane_mut(pane).unwrap().previous_run = Some(filename);
                }
                Err(e) => {
                    self.ws.pane_mut(pane).unwrap().previous_run = None;
                    self.notify_error(format!("Could not retain previous output: {e}"));
                }
            }
            let settings = self.settings.clone();
            let text = output.to_owned();
            self.previous_runs.insert(
                pane,
                cx.new(|_| crate::readonly_text::ReadOnlyText::new(text, settings)),
            );
        }
        self.sync_panes(cx);
        self.sync_attention(cx);
        self.save_session();
        if had_focus {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    pub(super) fn load_previous_run(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        if self.previous_runs.contains_key(&pane) {
            return;
        }
        let Some(filename) = self.ws.pane(pane).and_then(|p| p.previous_run.as_ref()) else {
            return;
        };
        let result = self
            .env
            .data_dir
            .as_ref()
            .filter(|_| output_name_valid(filename))
            .ok_or_else(|| "Invalid previous-output reference".to_string())
            .and_then(|dir| {
                std::fs::read_to_string(dir.join("agent-output").join(filename))
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(text) => {
                let settings = self.settings.clone();
                self.previous_runs.insert(
                    pane,
                    cx.new(|_| crate::readonly_text::ReadOnlyText::new(text, settings)),
                );
            }
            Err(e) => self.notify_error(format!("Could not read previous output: {e}")),
        }
    }

    pub(crate) fn restart_agent(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(info) = self.ws.pane(pane) else {
            return;
        };
        if !info.managed_run.as_ref().is_some_and(ManagedRun::stopped) {
            return;
        }
        let Some(context) = info.agent_launch.clone().filter(|c| c.managed) else {
            return;
        };
        let result = (|| {
            let argv = self.captured_resume_argv(&context)?;
            TerminalView::plan(
                &self.settings,
                &self.env,
                pane.raw(),
                Some(context.cwd.clone()),
                Some(argv),
                true,
            )
            .map_err(|e| e.to_string())
        })();
        let plan = match result {
            Ok(plan) => plan,
            Err(e) => {
                self.ws.pane_mut(pane).unwrap().managed_run = Some(ManagedRun::failed(e));
                cx.notify();
                return;
            }
        };
        let started_at = now_ms();
        self.ws.pane_mut(pane).unwrap().managed_run = Some(ManagedRun::Starting { started_at });
        self.previous_open.remove(&pane);
        let settings = self.settings.clone();
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move { TerminalView::prepare(plan) });
        cx.spawn_in(window,async move |this,cx| {
            let _operation = operation;
            let prepared = task.await;
            let _ = this.update_in(cx,|view,window,cx| {
                if view.quitting || !view.ws.pane(pane).and_then(|p|p.managed_run.as_ref())
                    .is_some_and(|run|matches!(run,ManagedRun::Starting { started_at: at } if *at==started_at)) { return; }
                match prepared {
                    Ok(prepared) => {
                        view.attach_terminal(pane,prepared,settings,Some(context.cwd),window,cx);
                        if view.ws.focused_pane()==Some(pane) { view.refocus_terminal(window,cx); }
                    }
                    Err(e) => { view.ws.pane_mut(pane).unwrap().managed_run = Some(ManagedRun::failed(format!("Could not restart: {e}"))); }
                }
                view.save_session();
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn render_managed_pane(&self, pane: PaneId, cx: &mut Context<Self>) -> AnyElement {
        let Some(info) = self.ws.pane(pane) else {
            return div().into_any_element();
        };
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let stopped = info.managed_run.as_ref().is_some_and(ManagedRun::stopped);
        let live = self.panes.get(&pane);
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(bg)
            .text_color(fg);
        if live.is_none() {
            let context = info.agent_launch.as_ref();
            let can_restart = stopped
                && context.is_some_and(|c| {
                    c.managed && c.session.as_ref().is_some_and(|s| !s.0.is_empty())
                });
            let mut status = div().p_3().flex().flex_col().gap_2().text_sm().child(
                info.managed_run
                    .as_ref()
                    .map_or_else(|| "Process unavailable".into(), ManagedRun::label),
            );
            if let Some(context) = context {
                status = status
                    .child(format!(
                        "{} · {}",
                        self.agent_name(context.agent.as_str()),
                        context.cwd.display()
                    ))
                    .child(context.session.as_ref().map_or_else(
                        || "No exact conversation ID captured".into(),
                        |s| format!("Conversation: {}", s.0),
                    ))
                    .child(context.policy.label(context.agent));
                if let Some(scope) = &context.reported_account_scope {
                    status = status.child(format!(
                        "Account report at launch: {scope} · CLI authentication inherited"
                    ));
                }
            }
            let starting = matches!(info.managed_run, Some(ManagedRun::Starting { .. }));
            if can_restart || starting {
                status = status.child(
                    div()
                        .id(("restart-agent", pane.raw()))
                        .debug_selector(move || {
                            if starting {
                                "agent-starting".into()
                            } else {
                                "agent-restart".into()
                            }
                        })
                        .px_3()
                        .py_1()
                        .border_1()
                        .border_color(fg.opacity(0.5))
                        .rounded_sm()
                        .child(if starting { "Starting…" } else { "Restart" })
                        .when(can_restart, |d| {
                            d.cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.restart_agent(pane, window, cx)
                                }))
                        }),
                );
            }
            content = content.child(status);
        }
        if info.previous_run.is_some() || self.previous_runs.contains_key(&pane) {
            let open = self.previous_open.contains(&pane);
            content = content.child(
                div()
                    .id(("previous-run-toggle", pane.raw()))
                    .debug_selector(|| "previous-run-toggle".into())
                    .px_3()
                    .py_1()
                    .text_sm()
                    .cursor_pointer()
                    .child(if open {
                        "Back to current run"
                    } else {
                        "Previous run"
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.previous_open.remove(&pane) {
                            this.load_previous_run(pane, cx);
                            this.previous_open.insert(pane);
                            this.ws.focus_pane(pane);
                            window.focus(&this.focus_handle, cx);
                        } else if this.ws.focused_pane() == Some(pane) {
                            this.refocus_terminal(window, cx);
                        }
                        cx.notify();
                    })),
            );
        }
        let body = if self.previous_open.contains(&pane) {
            self.previous_runs
                .get(&pane)
                .map(|v| v.clone().into_any_element())
        } else {
            live.map(|(v, _)| {
                AnyView::from(v.clone())
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element()
            })
        };
        content
            .child(div().flex_1().min_h_0().w_full().children(body))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn output_references_cannot_escape_the_private_output_directory() {
        assert!(super::output_name_valid("previous-run-1-2-3.txt"));
        for name in [
            "../previous-run-1.txt",
            "previous-run-/secret.txt",
            "/previous-run-1.txt",
            "previous-run-1.txt/other",
            "previous-run-ä.txt",
        ] {
            assert!(!super::output_name_valid(name));
        }
    }
}
