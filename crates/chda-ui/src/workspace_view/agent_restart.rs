use super::*;
use chda_core::PaneRun;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveryAction {
    Resume,
    New,
    Shell,
    Previous,
    Copy,
}

fn output_name_valid(name: &str) -> bool {
    name.starts_with("previous-run-")
        && name.ends_with(".txt")
        && name.len() < 160
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.'))
}

impl WorkspaceView {
    fn recovery_controls(&self, pane: PaneId) -> Vec<RecoveryAction> {
        let Some(info) = self.ws.pane(pane) else {
            return vec![];
        };
        if self.panes.contains_key(&pane) || !info.pane_run.as_ref().is_some_and(PaneRun::stopped) {
            return vec![];
        }
        let mut controls = Vec::new();
        if let Some(c) = &info.agent_launch {
            if c.managed
                && c.agent != AgentId::MiniMax
                && c.session.as_ref().is_some_and(|s| !s.0.is_empty())
            {
                controls.push(RecoveryAction::Resume);
            }
            controls.push(RecoveryAction::New);
        }
        controls.push(RecoveryAction::Shell);
        if self.previous_runs.contains_key(&pane) || info.previous_run.is_some() {
            controls.push(RecoveryAction::Previous);
            if self.previous_open.contains(&pane) {
                controls.push(RecoveryAction::Copy);
            }
        }
        controls
    }
    pub(super) fn recovery_key(
        &mut self,
        key: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.env.session_service.is_none()
            || !self.focus_handle.is_focused(window)
            || self.sheet.is_some()
            || self.palette.is_some()
            || self.note_sheet.is_some()
            || self.share_sheet.is_some()
            || self.context_menu.is_some()
        {
            return false;
        }
        let Some(pane) = self.ws.focused_pane() else {
            return false;
        };
        let controls = self.recovery_controls(pane);
        if controls.is_empty() {
            return false;
        }
        let selected = self
            .recovery_focus
            .filter(|(p, _)| *p == pane)
            .map(|(_, a)| a);
        match key.key.as_str() {
            "tab" if !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt => {
                let at = selected.and_then(|a| controls.iter().position(|c| *c == a));
                let next = if key.modifiers.shift {
                    at.map_or(controls.len() - 1, |i| {
                        (i + controls.len() - 1) % controls.len()
                    })
                } else {
                    at.map_or(0, |i| (i + 1) % controls.len())
                };
                self.recovery_focus = Some((pane, controls[next]));
                cx.notify();
            }
            "enter" | "space" | " " if !key.modifiers.modified() => {
                match selected.filter(|a| controls.contains(a)) {
                    Some(RecoveryAction::Resume) => self.restart_agent(pane, window, cx),
                    Some(RecoveryAction::New) => self.new_conversation(pane, window, cx),
                    Some(RecoveryAction::Shell) => self.start_shell(pane, window, cx),
                    Some(RecoveryAction::Previous) => {
                        if !self.previous_open.remove(&pane) {
                            self.load_previous_run(pane, cx);
                            self.previous_open.insert(pane);
                        }
                        cx.notify();
                    }
                    Some(RecoveryAction::Copy) => {
                        if let Some(v) = self.previous_runs.get(&pane) {
                            v.update(cx, |v, cx| v.copy_all(cx));
                        }
                    }
                    None => {}
                }
            }
            "escape" => {
                self.recovery_focus = None;
                cx.notify();
            }
            _ => return false,
        }
        true
    }
    fn new_conversation(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(context) = self.ws.pane(pane).and_then(|p| p.agent_launch.clone()) {
            self.open_managed_launch(
                &context.cwd,
                context.agent,
                context.options,
                None,
                window,
                cx,
            );
        }
    }
    pub(super) fn restore_service_pane(
        &mut self,
        p: chda_core::RestoredPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let record = self
            .env
            .session_service
            .as_ref()
            .and_then(|c| c.state().ok())
            .and_then(|s| s.sessions.into_iter().find(|r| r.pane == p.pane.raw()));
        if let Some(record) = &record {
            let info = self.ws.pane_mut(p.pane).unwrap();
            info.title = record.title.clone();
            info.agent_launch = record.launch.clone().or(info.agent_launch.clone());
            if let Some(event) = &record.agent {
                info.agent_session = Some(chda_core::AgentSessionRef {
                    agent: event.agent.clone(),
                    session: event.session_id.clone(),
                    cwd: Some(event.cwd.clone()),
                });
                info.agent_live = record.running && event.kind != HookKind::SessionEnd;
                info.agent = Some(chda_core::PaneAgent {
                    agent: event.agent.clone(),
                    status: match event.kind {
                        HookKind::SessionStart | HookKind::PromptSubmitted => AgentStatus::Working,
                        HookKind::WaitingInput => AgentStatus::WaitingInput,
                        HookKind::Stopped => AgentStatus::Review,
                        _ => AgentStatus::Idle,
                    },
                    since: event.timestamp,
                    seen: true,
                });
            }
            if record.running {
                let started_at = p
                    .run
                    .as_ref()
                    .and_then(PaneRun::started_at)
                    .unwrap_or_else(now_ms);
                info.pane_run = Some(PaneRun::Running { started_at });
                self.open_pane(p.pane, record.cwd.clone().or(p.cwd), None, window, cx);
                return;
            }
        }
        let mut status = self.env.session_service_error.clone().unwrap_or_else(|| match record.as_ref().and_then(|r|r.saved_at) {
            Some(at) => format!("Previous output saved {} ago. Choose how to continue; output after that save may be missing.", chda_core::activity_age(now_ms(),at)),
            None => "Previous session. Choose how to continue; no process was started.".into(),
        });
        if let Some(context) = self.ws.pane(p.pane).and_then(|i| i.agent_launch.as_ref())
            && let Err(error) = self.captured_resume_argv(context)
        {
            status.push(' ');
            status.push_str(&error);
        }
        let info = self.ws.pane_mut(p.pane).unwrap();
        info.pane_run = Some(PaneRun::failed(status));
        info.agent_live = false;
        if let Some(record) = record {
            if let Some(error) = &record.save_error {
                self.notify_error(format!("Session save failed: {error}"));
            }
            if let Some(client) = &self.env.session_service
                && let Ok(text) = client.output(record.identity)
                && !text.is_empty()
            {
                let settings = self.settings.clone();
                self.previous_runs.insert(
                    p.pane,
                    cx.new(|_| crate::readonly_text::ReadOnlyText::new(text, settings)),
                );
                self.previous_open.insert(p.pane);
            }
        }
        self.load_previous_run(p.pane, cx);
    }

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
        let adapter = self
            .adapters
            .iter()
            .find(|a| a.id() == context.agent)
            .ok_or("The recorded provider is unavailable")?;
        if adapter.session_roots().is_empty() {
            return Err("This provider does not expose verifiable local transcripts; exact resume is unavailable".into());
        }
        let session = context
            .session
            .as_ref()
            .filter(|s| !s.0.is_empty())
            .ok_or("No exact conversation ID was captured; restart is unavailable")?;
        if !adapter.has_session(session) {
            return Err(
                "The exact conversation transcript is missing; no new conversation was started"
                    .into(),
            );
        }
        self.captured_agent_argv(context)
    }

    pub(super) fn pane_exit(
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
        // A shell's exit, or a program chda started that succeeded, closes
        // the pane as cmd-w does; a failed program keeps its exit status and
        // output.
        let program = self
            .panes
            .get(&pane)
            .is_some_and(|(view, _)| view.read(cx).runs_program())
            || self
                .ws
                .pane(pane)
                .and_then(|p| p.agent_launch.as_ref())
                .is_some_and(|c| c.managed);
        let succeeded = status.is_some_and(|s| s.code == 0 && s.signal.is_none());
        if !program || succeeded {
            if self.remove_closed(CloseTarget::Pane(pane), cx) {
                if had_focus {
                    self.restore_focus(window, cx);
                } else {
                    self.sync_panes(cx);
                }
                self.sync_attention(cx);
                cx.notify();
            }
            return;
        }
        if self.env.session_service.is_some() {
            let resume_error = self
                .ws
                .pane(pane)
                .and_then(|p| p.agent_launch.as_ref())
                .filter(|c| c.agent == AgentId::MiniMax)
                .and_then(|c| self.captured_resume_argv(c).err());
            let stopped = PaneRun::Stopped {
                code: status.map(|s| s.code),
                signal: status.and_then(|s| s.signal.clone()),
                error: None,
            };
            let error = resume_error.map(|reason| format!("{}. {reason}", stopped.label()));
            self.ws.end_agent(pane);
            self.panes.remove(&pane);
            self.initializing_panes.remove(&pane);
            if let Some(info) = self.ws.pane_mut(pane) {
                info.pane_run = Some(PaneRun::Stopped {
                    code: status.map(|s| s.code),
                    signal: status.and_then(|s| s.signal.clone()),
                    error,
                });
                info.previous_run = None;
            }
            if let Some(Ok(output)) = output {
                let settings = self.settings.clone();
                let text = output.clone();
                self.previous_runs.insert(
                    pane,
                    cx.new(|_| crate::readonly_text::ReadOnlyText::new(text, settings)),
                );
                self.previous_open.insert(pane);
            }
            self.sync_panes(cx);
            self.save_session();
            cx.notify();
            return;
        }
        let agents = self.pane_agents(&[pane]);
        self.ws.end_agent(pane);
        self.resync_closed_agents(agents, cx);
        self.initializing_panes.remove(&pane);
        self.panes.remove(&pane);
        if let Some(info) = self.ws.pane_mut(pane) {
            info.pane_run = Some(PaneRun::Stopped {
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

    pub(super) fn start_shell(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(info) = self.ws.pane(pane) else {
            return;
        };
        if !info.pane_run.as_ref().is_some_and(PaneRun::stopped) {
            return;
        }
        if self.initializing_panes.contains(&pane) {
            return;
        }
        let cwd = info.cwd.clone();
        if let Some(path) = &cwd
            && !path.is_dir()
        {
            self.ws.pane_mut(pane).unwrap().pane_run = Some(PaneRun::failed(format!(
                "The directory {} is missing; no shell was started",
                path.display()
            )));
            cx.notify();
            return;
        }
        let info = self.ws.pane_mut(pane).unwrap();
        info.agent_launch = None;
        info.agent_session = None;
        info.agent = None;
        info.agent_live = false;
        info.pane_run = None;
        self.agent_report_at.remove(&pane);
        self.previous_open.remove(&pane);
        self.initializing_panes.insert(pane);
        self.open_pane(pane, cwd, None, window, cx);
        self.sync_panes(cx);
        self.save_session();
        if self.ws.focused_pane() == Some(pane) {
            self.refocus_terminal(window, cx);
        }
        cx.notify();
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
        if !info.pane_run.as_ref().is_some_and(PaneRun::stopped) {
            return;
        }
        let Some(mut context) = info.agent_launch.clone().filter(|c| c.managed) else {
            return;
        };
        let reported = info
            .agent_session
            .as_ref()
            .filter(|s| {
                s.agent == context.agent.as_str()
                    && context.session.as_ref().is_some_and(|id| s.session == id.0)
            })
            .and_then(|s| s.cwd.clone());
        let result = (|| {
            context.cwd =
                chda_core::restore_agent_cwd(reported.as_deref(), Some(&context.cwd), None, None)?
                    .unwrap();
            let argv = self.captured_resume_argv(&context)?;
            TerminalView::plan(
                &self.settings,
                &self.env,
                pane.raw(),
                Some(context.cwd.clone()),
                Some(argv),
            )
            .map_err(|e| e.to_string())
        })();
        let plan = match result {
            Ok(plan) => plan,
            Err(e) => {
                self.ws.pane_mut(pane).unwrap().pane_run = Some(PaneRun::failed(e));
                cx.notify();
                return;
            }
        };
        self.ws.pane_mut(pane).unwrap().agent_launch = Some(context.clone());
        let started_at = now_ms();
        self.ws.pane_mut(pane).unwrap().pane_run = Some(PaneRun::Starting { started_at });
        self.previous_open.remove(&pane);
        let settings = self.settings.clone();
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move { TerminalView::prepare(plan) });
        cx.spawn_in(window,async move |this,cx| {
            let _operation = operation;
            let prepared = task.await;
            let _ = this.update_in(cx,|view,window,cx| {
                if view.quitting || !view.ws.pane(pane).and_then(|p|p.pane_run.as_ref())
                    .is_some_and(|run|matches!(run,PaneRun::Starting { started_at: at } if *at==started_at)) { return; }
                match prepared {
                    Ok(prepared) => {
                        view.attach_terminal(pane,prepared,settings,Some(context.cwd),window,cx);
                        if view.ws.focused_pane()==Some(pane) { view.refocus_terminal(window,cx); }
                    }
                    Err(e) => { view.ws.pane_mut(pane).unwrap().pane_run = Some(PaneRun::failed(format!("Could not restart: {e}"))); }
                }
                view.save_session();
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn render_process_pane(&self, pane: PaneId, cx: &mut Context<Self>) -> AnyElement {
        let Some(info) = self.ws.pane(pane) else {
            return div().into_any_element();
        };
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let selected = |action| self.recovery_focus == Some((pane, action));
        let stopped = info.pane_run.as_ref().is_some_and(PaneRun::stopped);
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
                    c.managed
                        && (c.agent != AgentId::MiniMax || self.env.session_service.is_none())
                        && c.session.as_ref().is_some_and(|s| !s.0.is_empty())
                });
            let mut status = div()
                .id(("recovery-status", pane.raw()))
                .max_h(px(320.0))
                .overflow_y_scroll()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .text_sm()
                .child(
                    info.pane_run
                        .as_ref()
                        .map_or_else(|| "Process unavailable".into(), PaneRun::label),
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
            let starting = matches!(info.pane_run, Some(PaneRun::Starting { .. }));
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
                        .when(selected(RecoveryAction::Resume), |d| {
                            d.bg(blend(bg, fg, 0.18))
                        })
                        .child(if starting {
                            "Starting…"
                        } else if self.env.session_service.is_some() {
                            "Resume conversation"
                        } else {
                            "Restart"
                        })
                        .when(can_restart, |d| {
                            d.cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.restart_agent(pane, window, cx)
                                }))
                        }),
                );
            }
            if stopped && context.is_some() {
                status = status.child(
                    div()
                        .id(("new-conversation", pane.raw()))
                        .debug_selector(|| "new-conversation".into())
                        .px_3()
                        .py_1()
                        .border_1()
                        .border_color(fg.opacity(0.5))
                        .rounded_sm()
                        .when(selected(RecoveryAction::New), |d| d.bg(blend(bg, fg, 0.18)))
                        .cursor_pointer()
                        .child("New conversation")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.new_conversation(pane, window, cx);
                        })),
                );
            }
            if stopped {
                status = status.child(
                    div()
                        .id(("start-shell", pane.raw()))
                        .debug_selector(|| "start-shell".into())
                        .px_3()
                        .py_1()
                        .border_1()
                        .border_color(fg.opacity(0.5))
                        .rounded_sm()
                        .when(selected(RecoveryAction::Shell), |d| {
                            d.bg(blend(bg, fg, 0.18))
                        })
                        .cursor_pointer()
                        .child("Start shell")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_shell(pane, window, cx)
                        })),
                );
            }
            if selected(RecoveryAction::Copy) {
                status = status.child(
                    div()
                        .p_1()
                        .bg(blend(bg, fg, 0.18))
                        .child("Copy all previous output — press Enter"),
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
                    .when(selected(RecoveryAction::Previous), |d| {
                        d.bg(blend(bg, fg, 0.18))
                    })
                    .child(if open {
                        if live.is_some() {
                            "Back to current run"
                        } else {
                            "Hide previous output"
                        }
                    } else {
                        "Show previous output"
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
