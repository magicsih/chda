//! Persistent provider quota and selected-pane resources, independent of errors.
use crate::{
    terminal_element::hsla,
    terminal_view::now_ms,
    workspace_view::{WorkspaceView, blend},
};
use chda_core::{
    PaneId,
    agents::quota::{QuotaSnapshot, reset_text},
    resources::Resources,
};
use gpui::{AnyElement, Context, Window, div, prelude::*, px};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct StatusBar {
    pub quotas: HashMap<(String, String), QuotaSnapshot>,
    pub codex_error: Option<String>,
    pub resource_pane: Option<PaneId>,
    pub resources: Option<Resources>,
    pub resource_error: Option<String>,
    pub details_open: bool,
}
impl StatusBar {
    pub fn report(&mut self, quota: QuotaSnapshot) {
        let key = (quota.provider.clone(), quota.scope.clone());
        if self
            .quotas
            .get(&key)
            .is_none_or(|old| quota.observed_at >= old.observed_at)
        {
            self.quotas.insert(key, quota);
            if self.quotas.len() > 128
                && let Some(oldest) = self
                    .quotas
                    .iter()
                    .min_by_key(|(_, q)| q.observed_at)
                    .map(|(key, _)| key.clone())
            {
                self.quotas.remove(&oldest);
            }
        }
    }
    pub fn provider(&self, provider: &str, pane: Option<PaneId>) -> Option<&QuotaSnapshot> {
        // Failed account/authentication probes do not establish that a prior
        // account is still current. Retain history only in the details view.
        if provider == "codex" && self.codex_error.is_some() {
            return None;
        }
        self.quotas
            .values()
            .filter(|q| q.provider == provider)
            .max_by_key(|q| {
                (
                    q.pane.is_some() && q.pane == pane.map(PaneId::raw),
                    q.observed_at,
                )
            })
    }
    pub fn render(
        &self,
        view: &WorkspaceView,
        window: &Window,
        cx: &mut Context<WorkspaceView>,
    ) -> AnyElement {
        let fg = hsla(view.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(view.settings.colors.background.unwrap_or_default());
        let pane = view.ws.focused_pane();
        let now = now_ms();
        let narrow = f32::from(window.viewport_size().width) < 850.0;
        let mut bar = div()
            .id("status-bar")
            .debug_selector(|| "status-bar".into())
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(fg.opacity(0.15))
            .bg(blend(bg, fg, 0.04))
            .text_xs()
            .text_color(fg);
        for (provider, label) in [("claude", "Claude"), ("codex", "Codex")] {
            let report = self.provider(provider, pane);
            let quota = report.and_then(QuotaSnapshot::representative);
            let stale = report.is_some_and(|q| q.stale(now));
            let detail = report.map(|q| q.details(now)).unwrap_or_else(|| if provider == "claude" { "Waiting for a Claude Code status line report. Subscription limits are available only when the CLI provides them.".into() } else { self.codex_error.clone().unwrap_or_else(|| "Loading the local Codex CLI account quota".into()) });
            let text = match quota {
                Some(q) => format!(
                    "{:.0}% · {}{}",
                    q.used_percent,
                    q.name,
                    if stale { " · stale" } else { "" }
                ),
                None => {
                    if provider == "codex" && report.is_none() && self.codex_error.is_none() {
                        "Loading".into()
                    } else {
                        "Unavailable".into()
                    }
                }
            };
            let color = match quota.map(|q| q.used_percent) {
                Some(p) if p >= 90.0 => gpui::rgb(0xe07a5f).into(),
                _ => fg.opacity(0.8),
            };
            let mut item = div()
                .id(provider)
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .cursor_pointer()
                .tooltip(crate::tooltip::text(detail))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.status_bar.details_open = !view.status_bar.details_open;
                    cx.notify();
                }))
                .child(div().font_weight(gpui::FontWeight::MEDIUM).child(label))
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(text),
                );
            if let Some(q) = quota
                && !narrow
            {
                item = item
                    .child(
                        div()
                            .w(px(44.0))
                            .h(px(5.0))
                            .rounded_sm()
                            .bg(fg.opacity(0.12))
                            .child(
                                div()
                                    .w(px(44.0 * q.used_percent as f32 / 100.0))
                                    .h_full()
                                    .rounded_sm()
                                    .bg(color),
                            ),
                    )
                    .child(
                        div()
                            .text_color(fg.opacity(0.65))
                            .child(reset_text(q.resets_at, now)),
                    );
            }
            bar = bar.child(item);
        }
        bar = bar.child(div().flex_1());
        let resource = if self.resource_pane == pane {
            self.resources.as_ref()
        } else {
            None
        };
        let text = match resource {
            Some(r) => format!(
                "Pane · CPU {} · RAM {:.0} MiB · Ports {}",
                r.cpu_percent
                    .map(|c| format!("{c:.0}%"))
                    .unwrap_or_else(|| "…".into()),
                r.rss_bytes as f64 / 1_048_576.0,
                if r.ports_error.is_some() {
                    "?".into()
                } else {
                    r.listeners.len().to_string()
                }
            ),
            None => {
                if pane.is_none() {
                    "No terminal pane".into()
                } else {
                    "Pane metrics unavailable".into()
                }
            }
        };
        bar.child(
            div()
                .id("session-resources")
                .min_w_0()
                .max_w(px(if narrow { 185.0 } else { 330.0 }))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .cursor_pointer()
                .tooltip(crate::tooltip::text(
                    resource.map(Resources::details).unwrap_or_else(|| {
                        self.resource_error
                            .clone()
                            .unwrap_or_else(|| "Loading focused-pane metrics".into())
                    }),
                ))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.status_bar.details_open = !view.status_bar.details_open;
                    cx.notify();
                }))
                .child(text),
        )
        .into_any_element()
    }
    pub fn details(
        &self,
        view: &WorkspaceView,
        cx: &mut Context<WorkspaceView>,
    ) -> Option<AnyElement> {
        if !self.details_open {
            return None;
        }
        let fg = hsla(view.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(view.settings.colors.background.unwrap_or_default());
        let mut lines: Vec<String> = self
            .quotas
            .values()
            .map(|q| format!("{}\n{}", q.provider, q.details(now_ms())))
            .collect();
        lines.sort();
        if let Some(error) = &self.codex_error {
            lines.push(format!("Codex: {error}"));
        }
        if let Some(resources) = &self.resources
            && self.resource_pane == view.ws.focused_pane()
        {
            lines.push(resources.details());
        }
        if let Some(error) = &self.resource_error {
            lines.push(error.clone());
        }
        if lines.is_empty() {
            lines.push(
                "Usage reports are not available yet. No quota is inferred from token counts."
                    .into(),
            );
        }
        Some(
            gpui::deferred(
                div()
                    .absolute()
                    .bottom(px(30.0))
                    .right_2()
                    .w(px(480.0))
                    .max_w(gpui::relative(0.95))
                    .max_h(gpui::relative(0.7))
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(fg.opacity(0.2))
                    .bg(blend(bg, fg, 0.06))
                    .shadow_lg()
                    .occlude()
                    .text_sm()
                    .text_color(fg)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Usage & session details"),
                            )
                            .child(
                                div()
                                    .id("usage-close")
                                    .cursor_pointer()
                                    .px_2()
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.status_bar.details_open = false;
                                        cx.notify();
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .id("usage-details-content")
                            .overflow_y_scroll()
                            .max_h(px(380.0))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .children(lines.into_iter().map(|line| div().child(line))),
                    ),
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chda_core::agents::quota::codex_quota;
    use serde_json::json;

    #[test]
    fn failed_account_probe_hides_old_usage_and_recovery_keeps_accounts_separate() {
        let mut bar = StatusBar::default();
        let report = |account: &str, used: f64, at| {
            codex_quota(
            &json!({"rateLimits":{"primary":{"usedPercent":used,"windowDurationMins":300,"resetsAt":9000}}}),
            &json!({"account":{"type":"chatgpt","email":account}}), at).unwrap()
        };
        bar.report(report("first@example.test", 20.0, 100));
        assert_eq!(
            bar.provider("codex", None)
                .unwrap()
                .representative()
                .unwrap()
                .used_percent,
            20.0
        );
        bar.codex_error = Some("Account authentication unavailable".into());
        assert!(bar.provider("codex", None).is_none());
        assert_eq!(bar.quotas.len(), 1, "last known history remains in details");
        bar.report(report("second@example.test", 40.0, 200));
        bar.codex_error = None;
        assert_eq!(bar.quotas.len(), 2);
        let current = bar.provider("codex", None).unwrap();
        assert_eq!(current.scope, "second@example.test");
        assert_eq!(current.representative().unwrap().used_percent, 40.0);
    }
}
