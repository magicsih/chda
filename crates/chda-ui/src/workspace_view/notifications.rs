use super::*;
use chda_core::notifications::{NotificationContext, Severity};

impl WorkspaceView {
    fn notification_context(&self) -> NotificationContext {
        self.notification_pane_context(self.ws.focused_pane())
    }

    pub(super) fn notification_pane_context(&self, pane: Option<PaneId>) -> NotificationContext {
        NotificationContext {
            repository: pane
                .and_then(|p| self.ws.pane(p))
                .and_then(|p| p.cwd.clone()),
            tab: pane
                .and_then(|p| self.ws.tabs().iter().find(|t| t.panes().contains(&p)))
                .map(|tab| self.ws.tab_title(tab)),
            pane: pane.map(PaneId::raw),
        }
    }

    pub(super) fn notify(&mut self, message: String) {
        self.notifications.record(
            now_ms(),
            Severity::Info,
            message,
            self.notification_context(),
            None,
        );
    }

    pub(super) fn notify_error(&mut self, message: String) {
        self.notifications.record(
            now_ms(),
            Severity::Error,
            message,
            self.notification_context(),
            None,
        );
    }

    pub(super) fn notify_repo(
        &mut self,
        message: String,
        severity: Severity,
        path: &Path,
        operation: Option<u64>,
    ) {
        let context = NotificationContext {
            repository: Some(path.to_path_buf()),
            ..Default::default()
        };
        match operation {
            Some(id) => {
                self.notifications
                    .update_operation(id, now_ms(), severity, message, context)
            }
            None => self
                .notifications
                .record(now_ms(), severity, message, context, None),
        };
    }

    pub(super) fn begin_repo_notification(&mut self, message: String, path: &Path) -> u64 {
        self.notifications.start_operation(
            now_ms(),
            message,
            NotificationContext {
                repository: Some(path.to_path_buf()),
                ..Default::default()
            },
        )
    }

    pub(super) fn close_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notifications_open = false;
        if let Some(handle) = self
            .notification_focus
            .take()
            .and_then(|handle| handle.upgrade())
        {
            window.focus(&handle, cx);
        } else {
            self.refocus_terminal(window, cx);
        }
        cx.notify();
    }

    pub(super) fn render_notification_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let unread = self.notifications.unread();
        div()
            .id("notifications-toggle")
            .debug_selector(|| "notifications-toggle".into())
            .h(px(22.0))
            .px_1()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .flex_shrink_0()
            .cursor_pointer()
            .hover(|s| s.bg(fg.opacity(0.1)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .tooltip(crate::tooltip::text(format!(
                "Notifications — {unread} unread"
            )))
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                if this.notifications_open {
                    this.close_notifications(window, cx);
                } else {
                    this.notification_focus = window.focused(cx).map(|handle| handle.downgrade());
                    this.notifications.mark_open();
                    this.notifications_open = true;
                    window.focus(&this.focus_handle, cx);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .font(gpui::Font {
                        family: crate::fonts::DEFAULT_FAMILY.into(),
                        fallbacks: Some(crate::fonts::fallbacks()),
                        ..Default::default()
                    })
                    .text_base()
                    .child(optical("\u{f0f3}")),
            )
            .when(unread > 0, |d| {
                d.child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(unread.to_string()),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_notifications(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.notifications_open {
            return None;
        }
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let entries = self.notifications.newest();
        let empty = entries.is_empty();
        let rows = entries
            .into_iter()
            .map(|entry| {
                let id = entry.id;
                let pane = entry.context.pane.and_then(|raw| self.ws.pane_by_raw(raw));
                let context = [
                    entry
                        .context
                        .repository
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned()),
                    entry.context.tab.as_ref().map(|t| format!("Tab: {t}")),
                    entry.context.pane.map(|p| format!("Pane: {p}")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                div()
                    .id(gpui::ElementId::Name(
                        format!("notification-entry-{id}").into(),
                    ))
                    .debug_selector(move || format!("notification-entry-{id}"))
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .rounded_sm()
                    .border_b_1()
                    .border_color(fg.opacity(0.12))
                    .when(self.notifications.unread_entry(entry), |d| {
                        d.bg(fg.opacity(0.06))
                    })
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.1)))
                    .tooltip(crate::tooltip::text(if pane.is_some() {
                        "Go to the exact source pane"
                    } else {
                        "Mark as read — no open source pane"
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.notifications.mark_selected(id);
                        if let Some(pane) = pane
                            && this.ws.focus_pane(pane)
                        {
                            this.notifications_open = false;
                            this.notification_focus = None;
                            this.focus_active(window, cx);
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .child(div().flex_1().child(entry.severity.label()))
                            .child(timestamp(entry.timestamp))
                            .when(self.notifications.unread_entry(entry), |d| {
                                d.child("Unread")
                            }),
                    )
                    .child(div().child(entry.message.clone()))
                    .when(!context.is_empty(), |d| {
                        d.child(div().text_xs().text_color(fg.opacity(0.7)).child(context))
                    })
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        Some(
            deferred(
                div()
                    .id("notifications-popup")
                    .debug_selector(|| "notifications-popup".into())
                    .absolute()
                    .top(px(TITLE_BAR_HEIGHT + 8.0))
                    .right_2()
                    .w(px(480.0))
                    .max_w(relative(0.95))
                    .max_h(relative(0.75))
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(fg.opacity(0.2))
                    .bg(bg)
                    .text_color(fg)
                    .text_sm()
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .gap_2()
                            .p_1()
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Notifications"),
                            )
                            .child(
                                div()
                                    .id("notifications-close")
                                    .debug_selector(|| "notifications-close".into())
                                    .cursor_pointer()
                                    .px_2()
                                    .hover(|s| s.bg(fg.opacity(0.1)))
                                    .tooltip(crate::tooltip::text("Close notifications — Escape"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_notifications(window, cx)
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .id("notification-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .max_h(px(440.0))
                            .flex()
                            .flex_col()
                            .children(rows)
                            .when(empty, |d| {
                                d.child(div().p_3().child("No notifications yet"))
                            }),
                    )
                    .when(self.notifications.dropped > 0, |d| {
                        d.child(div().text_xs().p_1().child(format!(
                            "{} older notifications removed · history lasts until restart",
                            self.notifications.dropped
                        )))
                    }),
            )
            .into_any_element(),
        )
    }
}

/// An absolute date and a labeled timezone keep retained events unambiguous.
fn timestamp(milliseconds: u64) -> String {
    match time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000) {
        Ok(date) => format!(
            "{} · {:02}:{:02}:{:02} UTC",
            date.date(),
            date.hour(),
            date.minute(),
            date.second()
        ),
        Err(_) => "Timestamp unavailable".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamps_include_a_calendar_date_and_timezone() {
        assert_eq!(super::timestamp(0), "1970-01-01 · 00:00:00 UTC");
        assert_eq!(super::timestamp(86_400_000), "1970-01-02 · 00:00:00 UTC");
        assert_eq!(super::timestamp(u64::MAX), "Timestamp unavailable");
    }
}
