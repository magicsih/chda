use super::*;
use chda_core::agents::{
    AgentLaunchContext, LaunchHistory, PermissionPolicy, permission_arguments,
};

pub(super) struct LaunchItem {
    pub context: AgentLaunchContext,
    pub captured: bool,
}
pub(super) struct LaunchSheet {
    pub items: Vec<LaunchItem>,
    pub error: Option<String>,
}

impl WorkspaceView {
    pub(super) fn open_managed_launch(
        &mut self,
        cwd: &Path,
        agent: AgentId,
        options: Vec<String>,
        session: Option<SessionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .launch_item(cwd, agent, options, session)
            .and_then(|item| {
                let argv = if item.context.session.is_some() {
                    self.captured_resume_argv(&item.context)?
                } else {
                    self.captured_agent_argv(&item.context)?
                };
                Ok((item.context, argv))
            });
        match result {
            Ok((context, argv)) => {
                let pane = self.ws.new_tab().1;
                let cwd = context.cwd.clone();
                self.ws.pane_mut(pane).unwrap().agent_launch = Some(context);
                self.open_pane(pane, Some(cwd), Some(argv), window, cx);
                self.focus_active(window, cx);
            }
            Err(e) => {
                self.notify_error(e);
                cx.notify();
            }
        }
    }
    pub(super) fn capture_launch_session(&mut self, pane: PaneId, agent: &str, session: &str) {
        if session.is_empty() {
            return;
        }
        let Some(context) = self
            .ws
            .pane_mut(pane)
            .and_then(|info| info.agent_launch.as_mut())
        else {
            return;
        };
        if !context.managed
            || context.agent.as_str() != agent
            || context.session.as_ref().is_some_and(|id| id.0 == session)
        {
            return;
        }
        context.session = Some(SessionId(session.into()));
        let captured = context.clone();
        if let Err(e) = self.remember_launch(captured) {
            self.notify_error(format!(
                "Could not retain this conversation's launch options: {e}"
            ));
        }
    }
    pub(super) fn launch_item(
        &self,
        cwd: &Path,
        agent: AgentId,
        options: Vec<String>,
        session: Option<SessionId>,
    ) -> Result<LaunchItem, String> {
        if let Some(session) = &session
            && let Some(dir) = &self.env.data_dir
        {
            let history = LaunchHistory::load(dir)
                .map_err(|e| format!("Could not read captured launch options: {e}"))?;
            if let Some(context) = history.get(agent, session) {
                return Ok(LaunchItem {
                    context: context.clone(),
                    captured: true,
                });
            }
        }
        let adapter = self
            .adapters
            .iter()
            .find(|a| a.id() == agent)
            .ok_or("Unknown agent")?;
        let executable = adapter
            .executable(&self.env.home(), &self.env.search_path())
            .ok_or_else(|| format!("{} is not on PATH", adapter.display_name()))?;
        Ok(LaunchItem {
            context: AgentLaunchContext {
                agent,
                executable,
                cwd: cwd.to_path_buf(),
                options,
                policy: PermissionPolicy::Inherit,
                session,
                managed: true,
                reported_account_scope: self
                    .status_bar
                    .quotas
                    .borrow()
                    .provider(agent.as_str(), None)
                    .filter(|report| report.account_known && !report.stale(now_ms()))
                    .map(|report| report.scope.clone()),
            },
            captured: false,
        })
    }

    pub(super) fn show_agent_launch(
        &mut self,
        cwd: &Path,
        agent: AgentId,
        options: Vec<String>,
        session: Option<SessionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launch_sheet.is_some() {
            self.notify_error("Finish or cancel the current agent launch first".into());
            cx.notify();
            return;
        }
        match self.launch_item(cwd, agent, options, session) {
            Ok(item) => {
                self.launch_sheet = Some(LaunchSheet {
                    items: vec![item],
                    error: None,
                });
                self.notifications_open = false;
                self.notification_focus = None;
                window.focus(&self.focus_handle, cx);
            }
            Err(e) => self.notify_error(e),
        }
        cx.notify();
    }

    pub(super) fn captured_agent_argv(
        &self,
        context: &AgentLaunchContext,
    ) -> Result<Vec<String>, String> {
        if !context.managed {
            return Err("This command has no captured managed-launch context".into());
        }
        if !context.cwd.is_dir() {
            return Err(format!(
                "The recorded directory no longer exists: {}",
                context.cwd.display()
            ));
        }
        if !ipc::is_executable(&context.executable) {
            return Err(format!(
                "The recorded {} executable is unavailable",
                context.agent.as_str()
            ));
        }
        let options = permission_arguments(context.agent, context.policy, &context.options)?;
        let mut argv = self
            .agent_argv_for_executable(
                &context.cwd,
                context.agent,
                context.session.as_ref(),
                &context.executable,
            )
            .ok_or("Could not build the captured launch")?;
        argv.extend(options);
        Ok(argv)
    }

    pub(super) fn start_agent_launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = &self.launch_sheet else {
            return;
        };
        let argv = sheet
            .items
            .iter()
            .map(|item| {
                if item.context.session.is_some() {
                    self.captured_resume_argv(&item.context)
                } else {
                    self.captured_agent_argv(&item.context)
                }
            })
            .collect::<Result<Vec<_>, _>>();
        let argv = match argv {
            Ok(argv) => argv,
            Err(e) => {
                self.launch_sheet.as_mut().unwrap().error = Some(e);
                cx.notify();
                return;
            }
        };
        let sheet = self.launch_sheet.take().unwrap();
        let aspect = self.tab_aspect(window);
        for (index, (item, argv)) in sheet.items.into_iter().zip(argv).enumerate() {
            let pane = if index == 0 {
                Some(self.ws.new_tab().1)
            } else {
                self.ws.split_largest(aspect)
            };
            if let Some(pane) = pane {
                self.ws.pane_mut(pane).unwrap().agent_launch = Some(item.context.clone());
                self.open_pane(pane, Some(item.context.cwd), Some(argv), window, cx);
            }
        }
        self.focus_active(window, cx);
    }

    pub(super) fn remember_launch(&mut self, context: AgentLaunchContext) -> Result<(), String> {
        let Some(dir) = &self.env.data_dir else {
            return Err("No data directory to retain launch options".into());
        };
        let mut history = LaunchHistory::load(dir).map_err(|e| e.to_string())?;
        history.record(context);
        history.save(dir).map_err(|e| e.to_string())
    }

    pub(super) fn render_agent_launch(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.launch_sheet.as_ref()?;
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let mut sections = Vec::new();
        for (index, item) in sheet.items.iter().enumerate() {
            let context = &item.context;
            let mut section = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(self.agent_name(context.agent.as_str())),
                )
                .child(
                    div()
                        .text_xs()
                        .child(context.cwd.to_string_lossy().into_owned()),
                );
            if item.captured || !matches!(context.agent, AgentId::Claude | AgentId::Codex) {
                section = section.child(div().child(format!(
                    "Recorded policy: {}",
                    context.policy.label(context.agent)
                )));
            } else {
                for (policy, label, selector) in [
                    (
                        PermissionPolicy::Inherit,
                        "Use CLI configuration",
                        "launch-policy-inherit",
                    ),
                    (PermissionPolicy::Bypass, "Bypass", "launch-policy-bypass"),
                ] {
                    section = section.child(
                        div()
                            .id(gpui::ElementId::Name(format!("{selector}-{index}").into()))
                            .debug_selector(move || {
                                if index == 0 {
                                    selector.into()
                                } else {
                                    format!("{selector}-{index}")
                                }
                            })
                            .flex()
                            .items_center()
                            .gap_2()
                            .p_2()
                            .border_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .border_color(fg.opacity(if context.policy == policy {
                                0.65
                            } else {
                                0.2
                            }))
                            .hover(|s| s.bg(fg.opacity(0.08)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(item) = this
                                    .launch_sheet
                                    .as_mut()
                                    .and_then(|s| s.items.get_mut(index))
                                    && !item.captured
                                {
                                    item.context.policy = policy;
                                    this.launch_sheet.as_mut().unwrap().error = None;
                                    cx.notify();
                                }
                            }))
                            .child(if context.policy == policy {
                                "◉"
                            } else {
                                "○"
                            })
                            .child(label),
                    );
                }
            }
            section = section.child(div().text_xs().child(context.policy.label(context.agent)));
            let options =
                match permission_arguments(context.agent, context.policy, &context.options) {
                    Ok(options) if options.is_empty() => {
                        "No permission overrides added by chda".into()
                    }
                    Ok(options) => options
                        .iter()
                        .map(|arg| format!("{arg:?}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                    Err(e) => e,
                };
            section = section.child(div().text_xs().child(options));
            if let Some(session) = &context.session {
                section = section.child(
                    div()
                        .text_xs()
                        .child(format!("Resume exact session: {}", session.0)),
                );
            }
            if let Some(scope) = &context.reported_account_scope {
                section = section.child(div().text_xs().child(format!(
                    "Account report at launch: {scope} · CLI authentication inherited"
                )));
            }
            sections.push(section.into_any_element());
        }
        Some(
            deferred(
                div()
                    .id("agent-launch-sheet")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::black().opacity(0.35))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .w(px(540.0))
                            .max_w(relative(0.95))
                            .max_h(relative(0.9))
                            .p_3()
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .gap_2()
                            .bg(bg)
                            .text_color(fg)
                            .text_sm()
                            .border_1()
                            .border_color(fg.opacity(0.2))
                            .rounded_md()
                            .shadow_lg()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Launch agent"),
                            )
                            .child(
                                div()
                                    .id("launch-options-list")
                                    .flex_1()
                                    .min_h_0()
                                    .max_h(px(480.0))
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_col()
                                    .children(sections),
                            )
                            .when_some(sheet.error.clone(), |d, e| d.child(div().child(e)))
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id("launch-cancel")
                                            .debug_selector(|| "launch-cancel".into())
                                            .px_3()
                                            .py_1()
                                            .cursor_pointer()
                                            .child("Cancel")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.launch_sheet = None;
                                                this.refocus_terminal(window, cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("launch-start")
                                            .debug_selector(|| "launch-start".into())
                                            .px_3()
                                            .py_1()
                                            .border_1()
                                            .border_color(fg.opacity(0.5))
                                            .rounded_sm()
                                            .cursor_pointer()
                                            .child("Start")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.start_agent_launch(window, cx)
                                            })),
                                    ),
                            ),
                    ),
            )
            .into_any_element(),
        )
    }
}
