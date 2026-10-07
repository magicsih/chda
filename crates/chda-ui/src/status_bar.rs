//! Persistent provider quota and selected-pane resources, independent of errors.
use crate::{
    terminal_element::hsla,
    terminal_view::now_ms,
    workspace_view::{WorkspaceView, blend},
};
use chda_core::{
    PaneId,
    agents::quota::{CodexQuotaError, QuotaSnapshot, reset_text},
    resources::Resources,
};
use gpui::{AnyElement, Context, Window, div, prelude::*, px};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Default)]
pub(crate) enum CodexQuotaStatus {
    #[default]
    Loading,
    Ready(String),
    Failed(CodexQuotaError),
}

/// All windows share provider reports; pane resources remain window-local.
#[derive(Default)]
pub(crate) struct ProviderQuotas {
    pub reports: HashMap<(String, String), QuotaSnapshot>,
    pub codex: CodexQuotaStatus,
}

#[derive(Default)]
pub(crate) struct StatusBar {
    pub quotas: Rc<RefCell<ProviderQuotas>>,
    pub resource_pane: Option<PaneId>,
    pub resources: Option<Resources>,
    pub resource_error: Option<String>,
    pub details_open: bool,
}
impl ProviderQuotas {
    pub fn report(&mut self, quota: QuotaSnapshot) {
        let key = (quota.provider.clone(), quota.scope.clone());
        if self
            .reports
            .get(&key)
            .is_none_or(|old| quota.observed_at >= old.observed_at)
        {
            if quota.windows.is_empty()
                && let Some(old) = self.reports.get_mut(&key)
                && !old.windows.is_empty()
            {
                // A status-line event without quota is not a new usage reading.
                // Keep the real observation time so the retained value can age.
                old.pane = quota.pane;
            } else {
                self.reports.insert(key, quota);
            }
            if self.reports.len() > 128
                && let Some(oldest) = self
                    .reports
                    .iter()
                    .min_by_key(|(_, q)| q.observed_at)
                    .map(|(key, _)| key.clone())
            {
                self.reports.remove(&oldest);
            }
        }
    }
    pub fn codex_result(&mut self, result: Result<QuotaSnapshot, CodexQuotaError>) {
        match result {
            Ok(quota) => {
                self.codex = CodexQuotaStatus::Ready(quota.scope.clone());
                self.report(quota);
            }
            Err(error) => self.codex = CodexQuotaStatus::Failed(error),
        }
    }
    pub fn provider(&self, provider: &str, pane: Option<PaneId>) -> Option<&QuotaSnapshot> {
        if provider == "codex" {
            let scope = match &self.codex {
                CodexQuotaStatus::Ready(scope) => scope.as_str(),
                CodexQuotaStatus::Failed(error) => error.scope()?,
                CodexQuotaStatus::Loading => return None,
            };
            return self.reports.get(&(provider.into(), scope.into()));
        }
        self.reports
            .values()
            .filter(|q| q.provider == provider)
            .max_by_key(|q| {
                (
                    q.pane.is_some() && q.pane == pane.map(PaneId::raw),
                    q.observed_at,
                )
            })
    }
    pub fn text(&self, provider: &str, pane: Option<PaneId>, now: u64) -> String {
        let report = self.provider(provider, pane);
        if let Some(quota) = report.and_then(QuotaSnapshot::representative) {
            let retrying = provider == "codex" && matches!(self.codex, CodexQuotaStatus::Failed(_));
            let suffix = match (report.is_some_and(|q| q.stale(now)), retrying) {
                (true, true) => " · stale · retrying",
                (true, false) => " · stale",
                (false, true) => " · retrying",
                (false, false) => "",
            };
            return format!("{:.0}% · {}{suffix}", quota.used_percent, quota.name);
        }
        match (provider, &self.codex) {
            ("codex", CodexQuotaStatus::Loading) => "Loading",
            ("codex", CodexQuotaStatus::Failed(error)) => error.label(),
            _ => "Waiting for usage",
        }
        .into()
    }

    pub fn details(&self, provider: &str, pane: Option<PaneId>, now: u64) -> String {
        let mut detail = self.provider(provider, pane).map(|q| q.details(now)).unwrap_or_else(|| {
            if provider == "claude" {
                "Waiting for Claude Code to report usage after its first response. Subscription limits appear only when the CLI provides them.".into()
            } else {
                "Loading the local Codex CLI account quota.".into()
            }
        });
        if provider == "codex"
            && let CodexQuotaStatus::Failed(error) = &self.codex
        {
            if self.provider(provider, pane).is_some() {
                detail.push_str("\nLast known usage; the current account was verified, but its quota refresh failed.\n");
                detail.push_str(error.details());
            } else {
                detail = error.details().into();
            }
        }
        detail
    }
}

impl StatusBar {
    pub fn render(
        &self,
        view: &WorkspaceView,
        window: &Window,
        cx: &mut Context<WorkspaceView>,
    ) -> AnyElement {
        let fg = hsla(view.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(view.settings.colors.background.unwrap_or_default());
        let pane = view.ws.focused_pane();
        let quotas = self.quotas.borrow();
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
            let report = quotas.provider(provider, pane);
            let quota = report.and_then(QuotaSnapshot::representative);
            let detail = quotas.details(provider, pane, now);
            let text = quotas.text(provider, pane, now);
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
                } else if self.resource_error.is_none() {
                    "Pane · Loading metrics".into()
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
        let quotas = self.quotas.borrow();
        let mut lines: Vec<String> = quotas
            .reports
            .values()
            .map(|q| format!("{}\n{}", q.provider, q.details(now_ms())))
            .collect();
        lines.sort();
        if let CodexQuotaStatus::Failed(error) = &quotas.codex {
            lines.push(format!("Codex: {}", error.details()));
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
    use chda_core::agents::quota::{claude_quota, codex_quota};
    use serde_json::json;

    fn codex(account: &str, used: f64, at: u64) -> QuotaSnapshot {
        codex_quota(
            &json!({"rateLimits":{"primary":{"usedPercent":used,"windowDurationMins":300,"resetsAt":9000}}}),
            &json!({"account":{"type":"chatgpt","email":account}}), at).unwrap()
    }

    #[test]
    fn startup_shows_waiting_and_loading_instead_of_an_error() {
        let quotas = ProviderQuotas::default();
        assert_eq!(quotas.text("claude", None, 100), "Waiting for usage");
        assert_eq!(quotas.text("codex", None, 100), "Loading");
    }

    #[test]
    fn empty_claude_reports_preserve_usage_and_its_actual_age() {
        let mut quotas = ProviderQuotas::default();
        let report = claude_quota(
            &json!({"session_id":"s", "rate_limits":{"five_hour":{"used_percentage":25}}}),
            Some(1),
            100,
        )
        .unwrap();
        quotas.report(report.clone());
        // A later event without usage must not erase the reading or make it fresh.
        quotas.report(claude_quota(&json!({"session_id":"s"}), Some(2), 200).unwrap());
        let kept = quotas.provider("claude", None).unwrap();
        assert_eq!(kept.windows, report.windows);
        assert_eq!(kept.observed_at, 100);
        assert_eq!(kept.pane, Some(2));
        assert_eq!(quotas.text("claude", None, 300101), "25% · 5h · stale");
        quotas.report(claude_quota(&json!({"session_id":"other"}), Some(2), 300).unwrap());
        assert_eq!(
            quotas.text("claude", None, 300),
            "Waiting for usage",
            "another session never inherits the earlier session's quota"
        );
    }

    #[test]
    fn failed_account_probe_hides_old_usage_and_recovery_keeps_accounts_separate() {
        let mut quotas = ProviderQuotas::default();
        quotas.codex_result(Ok(codex("first@example.test", 20.0, 100)));
        assert_eq!(quotas.text("codex", None, 100), "20% · codex 5h");
        quotas.codex_result(Err(CodexQuotaError::Retry {
            scope: None,
            message: "connection closed".into(),
        }));
        assert!(quotas.provider("codex", None).is_none());
        assert_eq!(quotas.text("codex", None, 200), "Retrying");
        assert_eq!(quotas.reports.len(), 1, "history remains in details");
        quotas.codex_result(Ok(codex("second@example.test", 40.0, 200)));
        assert_eq!(quotas.reports.len(), 2);
        assert_eq!(
            quotas.provider("codex", None).unwrap().scope,
            "second@example.test"
        );
    }

    #[test]
    fn transport_failure_keeps_only_the_account_verified_by_this_probe() {
        let mut quotas = ProviderQuotas::default();
        quotas.codex_result(Ok(codex("first@example.test", 20.0, 100)));
        quotas.codex_result(Err(CodexQuotaError::Retry {
            scope: Some("first@example.test".into()),
            message: "query timed out".into(),
        }));
        assert_eq!(quotas.text("codex", None, 200), "20% · codex 5h · retrying");
        assert!(
            quotas
                .details("codex", None, 200)
                .contains("Last known usage")
        );
        assert_eq!(quotas.provider("codex", None).unwrap().observed_at, 100);
        assert_eq!(
            quotas.text("codex", None, 300101),
            "20% · codex 5h · stale · retrying"
        );
        quotas.codex_result(Err(CodexQuotaError::Retry {
            scope: Some("second@example.test".into()),
            message: "query timed out".into(),
        }));
        assert!(quotas.provider("codex", None).is_none());
        quotas.codex_result(Err(CodexQuotaError::SignInRequired));
        assert_eq!(quotas.text("codex", None, 300), "Sign in");
        assert!(quotas.provider("codex", None).is_none());
    }

    #[test]
    fn empty_codex_report_does_not_reset_last_usage_or_choose_another_account() {
        let mut quotas = ProviderQuotas::default();
        quotas.codex_result(Ok(codex("first@example.test", 20.0, 100)));
        quotas.codex_result(Ok(codex("second@example.test", 40.0, 200)));
        let mut empty = codex("first@example.test", 0.0, 300);
        empty.windows.clear();
        quotas.codex_result(Ok(empty));
        assert_eq!(quotas.text("codex", None, 300), "20% · codex 5h");
        assert_eq!(quotas.provider("codex", None).unwrap().observed_at, 100);
    }
}
