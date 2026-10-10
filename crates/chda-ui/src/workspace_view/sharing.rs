use super::*;
use chda_core::agents::{SharePreview, sharing_preview};

pub(crate) struct ShareSheet {
    pub pane: PaneId,
    tab: TabId,
    identity: Option<(String, String)>,
    request: u64,
    selected: String,
    pub input: Option<Entity<TextInput>>,
    source: Option<String>,
    show_selected: bool,
    focus: Option<gpui::WeakFocusHandle>,
    _events: Option<Subscription>,
}

impl WorkspaceView {
    pub(super) fn share_identity(&self, pane: PaneId) -> Option<(String, String)> {
        let info = self.ws.pane(pane)?;
        info.agent_session
            .as_ref()
            .map(|s| (s.agent.clone(), s.session.clone()))
            .or_else(|| {
                let launch = info.agent_launch.as_ref()?;
                Some((
                    launch.agent.as_str().into(),
                    launch.session.as_ref()?.0.clone(),
                ))
            })
    }
    pub(super) fn open_share_sheet(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .ws
            .active_tab()
            .filter(|t| t.panes().contains(&pane))
            .map(|t| t.id)
        else {
            return;
        };
        let Some((term, _)) = self.panes.get(&pane) else {
            return;
        };
        self.share_generation = self.share_generation.wrapping_add(1);
        let request = self.share_generation;
        self.share_sheet = Some(ShareSheet {
            pane,
            tab,
            identity: self.share_identity(pane),
            request,
            selected: String::new(),
            input: None,
            source: None,
            show_selected: false,
            focus: window.focused(cx).map(|f| f.downgrade()),
            _events: None,
        });
        term.read(cx).read_selection(request);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
    pub(super) fn receive_share_selection(
        &mut self,
        pane: PaneId,
        request: u64,
        text: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .share_sheet
            .as_ref()
            .is_some_and(|s| s.pane == pane && s.request == request)
        {
            return;
        }
        let Some(selected) = text.filter(|s| !s.is_empty()) else {
            self.close_share_sheet(window, cx);
            self.notify_error("Select the draft text before copying for sharing".into());
            return;
        };
        let scope = self.share_sheet.as_ref().unwrap().identity.clone();
        self.share_sheet.as_mut().unwrap().selected = selected.clone();
        let adapter = scope
            .as_ref()
            .and_then(|(a, _)| AgentId::parse(a))
            .and_then(|a| self.adapters.iter().find(|x| x.id() == a));
        let roots = adapter.map(|a| a.session_roots()).unwrap_or_default();
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move {
            match scope.and_then(|(a, id)| Some((AgentId::parse(&a)?, SessionId(id)))) {
                Some((agent, id)) => sharing_preview(agent, &id, &roots, &selected),
                None => SharePreview {
                    text: selected,
                    source: None,
                },
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let preview = task.await;
            let _ = this.update_in(cx, |v, window, cx| {
                v.apply_share_preview(request, preview, window, cx)
            });
            drop(operation);
        })
        .detach();
    }
    fn share_scope_current(&self, sheet: &ShareSheet) -> bool {
        self.ws
            .active_tab()
            .is_some_and(|t| t.id == sheet.tab && t.panes().contains(&sheet.pane))
            && self.share_identity(sheet.pane) == sheet.identity
    }
    fn apply_share_preview(
        &mut self,
        request: u64,
        preview: SharePreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sheet) = self.share_sheet.as_ref().filter(|s| s.request == request) else {
            return;
        };
        if !self.share_scope_current(sheet) {
            // A background reply must not reopen an old pane or steal focus.
            self.share_sheet = None;
            cx.notify();
            return;
        }
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Review the selected text", fg, bg, cx);
            input.multiline = true;
            input.soft_wrap = true;
            input.set_text(&preview.text, cx);
            input
        });
        let events = cx.subscribe_in(&input, window, |v, _, event, window, cx| match event {
            TextInputEvent::Submit(_) => v.copy_share_preview(window, cx),
            TextInputEvent::Cancel => v.close_share_sheet(window, cx),
        });
        if self.focus_handle.is_focused(window) {
            window.focus(&input.focus_handle(cx), cx);
        }
        let sheet = self.share_sheet.as_mut().unwrap();
        sheet.input = Some(input);
        sheet.source = preview.source;
        sheet._events = Some(events);
        cx.notify();
    }
    pub(super) fn copy_share_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.share_sheet.as_ref() else {
            return;
        };
        if !self.share_scope_current(sheet) {
            self.notify_error(
                "The selected conversation changed. Select its text again before copying.".into(),
            );
            self.close_share_sheet(window, cx);
            return;
        }
        let Some(input) = &sheet.input else { return };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            input.read(cx).text().into(),
        ));
        self.close_share_sheet(window, cx);
    }
    pub(super) fn close_share_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(sheet) = self.share_sheet.take() {
            if self.ws.active_tab().is_some_and(|t| t.id == sheet.tab)
                && let Some(focus) = sheet.focus.and_then(|f| f.upgrade())
            {
                window.focus(&focus, cx);
            } else {
                self.refocus_terminal(window, cx);
            }
            cx.notify();
        }
    }
    pub(super) fn render_share_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.share_sheet.as_ref()?;
        let mut body = div()
            .id("sharing-preview")
            .debug_selector(|| "sharing-preview".into())
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0();
        body = body.child(if sheet.input.is_none() {
            "Preparing selected text…"
        } else if sheet.source.is_some() {
            "Matched assistant text · review before copying"
        } else {
            "Source match unavailable · selected text kept"
        });
        body = body.child(div().text_xs().child(match &sheet.identity {
            Some((agent, session)) => format!("{agent} · conversation {session}"),
            None => "Selected terminal pane · local preview".into(),
        }));
        if let Some(source) = &sheet.source {
            body = body.tooltip(crate::tooltip::text(source.clone()));
        }
        if let Some(input) = &sheet.input {
            body = body
                .child(
                    div()
                        .id("sharing-editor")
                        .debug_selector(|| "sharing-editor".into())
                        .max_h(px(220.0))
                        .overflow_y_scroll()
                        .min_w_0()
                        .child(input.clone()),
                )
                .child(
                    div()
                        .id("sharing-show-selected")
                        .cursor_pointer()
                        .child(if sheet.show_selected {
                            "Hide selected terminal text"
                        } else {
                            "Show selected terminal text"
                        })
                        .on_click(cx.listener(|v, _, _, cx| {
                            if let Some(s) = &mut v.share_sheet {
                                s.show_selected = !s.show_selected;
                            }
                            cx.notify();
                        })),
                );
            if sheet.show_selected {
                body = body.child(
                    div()
                        .id("sharing-original")
                        .max_h(px(120.0))
                        .overflow_y_scroll()
                        .text_xs()
                        .child(sheet.selected.clone()),
                );
            }
        }
        body = body.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    div()
                        .id("sharing-cancel")
                        .cursor_pointer()
                        .p_2()
                        .child("Cancel")
                        .on_click(cx.listener(|v, _, window, cx| v.close_share_sheet(window, cx))),
                )
                .children(sheet.input.as_ref().map(|_| {
                    div()
                        .id("sharing-copy")
                        .debug_selector(|| "sharing-copy".into())
                        .cursor_pointer()
                        .p_2()
                        .rounded_sm()
                        .bg(gpui::rgb(0x89b4fa))
                        .text_color(gpui::rgb(0x181825))
                        .child("Copy plain text")
                        .on_click(cx.listener(|v, _, window, cx| v.copy_share_preview(window, cx)))
                })),
        );
        Some(self.sheet_frame(
            "Copy for sharing".into(),
            div().child(body),
            None,
            "Shift ↵ New line · Enter Copy · Esc Cancel",
        ))
    }
}
