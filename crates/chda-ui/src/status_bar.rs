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
use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc};

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
    cache_loaded: bool,
    cache_saved_at: Option<u64>,
}

/// The last Claude reading per configuration folder, kept across restarts
/// and updates in the data folder.
const QUOTA_CACHE: &str = "quota-cache.json";

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
        self.claude(pane)
    }

    /// The Claude reading for `pane`: the newest one with usage from the
    /// configuration folder of the pane's own session, which shares its
    /// sign-in, else the pane's own report. Different folders never mix.
    /// A pane without a Claude session gets the newest reading with usage.
    fn claude(&self, pane: Option<PaneId>) -> Option<&QuotaSnapshot> {
        let reports = || self.reports.values().filter(|q| q.provider == "claude");
        let newest_with_usage = |dir: Option<&str>| {
            reports()
                .filter(|q| {
                    !q.windows.is_empty()
                        && dir.is_none_or(|dir| q.config_dir.as_deref() == Some(dir))
                })
                .max_by_key(|q| (q.observed_at, !q.previous_run))
        };
        match self.own_claude(pane) {
            Some(own) => own
                .config_dir
                .as_deref()
                .and_then(|dir| newest_with_usage(Some(dir)))
                .or(Some(own)),
            None => newest_with_usage(None).or_else(|| reports().max_by_key(|q| q.observed_at)),
        }
    }

    fn own_claude(&self, pane: Option<PaneId>) -> Option<&QuotaSnapshot> {
        let pane = pane?.raw();
        self.reports
            .values()
            .filter(|q| q.provider == "claude" && q.pane == Some(pane))
            .max_by_key(|q| q.observed_at)
    }

    /// Restore the readings saved by [`Self::save_cache`], once per run.
    pub fn load_cache(&mut self, data_dir: &Path) {
        if std::mem::replace(&mut self.cache_loaded, true) {
            return;
        }
        let Ok(bytes) = std::fs::read(data_dir.join(QUOTA_CACHE)) else {
            return;
        };
        let cached: Vec<QuotaSnapshot> = serde_json::from_slice(&bytes).unwrap_or_default();
        for mut quota in cached {
            if quota.provider != "claude" || quota.windows.is_empty() || quota.config_dir.is_none()
            {
                continue;
            }
            quota.previous_run = true;
            quota.pane = None;
            self.reports
                .entry((quota.provider.clone(), quota.scope.clone()))
                .or_insert(quota);
        }
    }

    /// Save the newest Claude reading with usage per configuration folder,
    /// at most once a minute. Only percentages and times are written.
    pub fn save_cache(&mut self, data_dir: &Path, now: u64) {
        if self
            .cache_saved_at
            .is_some_and(|at| now.saturating_sub(at) < 60_000)
        {
            return;
        }
        let mut newest: HashMap<&str, &QuotaSnapshot> = HashMap::new();
        for quota in self.reports.values() {
            if quota.provider != "claude" || quota.windows.is_empty() {
                continue;
            }
            let Some(dir) = quota.config_dir.as_deref() else {
                continue;
            };
            if newest
                .get(dir)
                .is_none_or(|q| q.observed_at < quota.observed_at)
            {
                newest.insert(dir, quota);
            }
        }
        let cached: Vec<QuotaSnapshot> = newest
            .into_values()
            .map(|q| QuotaSnapshot {
                pane: None,
                previous_run: false,
                ..q.clone()
            })
            .collect();
        if let Ok(json) = serde_json::to_vec(&cached)
            && std::fs::write(data_dir.join(QUOTA_CACHE), json).is_ok()
        {
            self.cache_saved_at = Some(now);
        }
    }
    pub fn text(&self, provider: &str, pane: Option<PaneId>, now: u64) -> String {
        let report = self.provider(provider, pane);
        if let Some(quota) = report.and_then(QuotaSnapshot::representative) {
            let retrying = provider == "codex" && matches!(self.codex, CodexQuotaStatus::Failed(_));
            let mut suffix = String::new();
            if report.is_some_and(|q| q.previous_run) {
                suffix.push_str(" · previous run");
            } else if report.is_some_and(|q| q.stale(now)) {
                suffix.push_str(" · stale");
            }
            if retrying {
                suffix.push_str(" · retrying");
            }
            return format!("{:.0}% · {}{suffix}", quota.used_percent, quota.name);
        }
        match (provider, &self.codex) {
            ("codex", CodexQuotaStatus::Loading) => "Loading",
            ("codex", CodexQuotaStatus::Failed(error)) => error.label(),
            _ => "Waiting for usage",
        }
        .into()
    }

    /// `live_claude`: the pane runs a live Claude Code session.
    pub fn details(
        &self,
        provider: &str,
        pane: Option<PaneId>,
        now: u64,
        live_claude: bool,
    ) -> String {
        let report = self.provider(provider, pane);
        let mut detail = report.map(|q| q.details(now)).unwrap_or_else(|| {
            if provider != "claude" {
                "Loading the local Codex CLI account quota.".into()
            } else if live_claude {
                "This Claude Code session has not reported through chda's status line. Sessions started outside chda, or with settings that replace its status line, report no usage.".into()
            } else {
                "No reading yet: Claude Code reports usage after its next response in a chda pane. Subscription limits appear only when the CLI provides them.".into()
            }
        });
        if provider == "claude"
            && let Some(q) = report.filter(|q| !q.windows.is_empty())
        {
            if q.previous_run {
                detail.push_str(
                    "\nKept from before chda restarted; it updates after the next Claude response.",
                );
            } else if q.pane != pane.map(PaneId::raw) {
                detail.push_str(if self.own_claude(pane).is_some() {
                    "\nFrom another Claude Code session with the same configuration folder, which shares one sign-in's limits."
                } else {
                    "\nFrom the most recent Claude Code session with a reading."
                });
            }
        }
        if provider == "codex"
            && let CodexQuotaStatus::Failed(error) = &self.codex
        {
            if self.provider(provider, pane).is_some() {
                detail.push_str("\nLast known usage; the current account was verified, but its quota refresh failed.\n");
                detail.push_str(&error.details());
            } else {
                detail = error.details();
            }
        }
        detail
    }
}

/// Whether `pane` runs a live Claude Code session.
fn live_claude(view: &WorkspaceView, pane: Option<PaneId>) -> bool {
    pane.and_then(|pane| view.ws.pane(pane))
        .is_some_and(|info| {
            info.agent_live && info.agent.as_ref().is_some_and(|a| a.agent == "claude")
        })
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
        let live_claude = live_claude(view, pane);
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
            let detail = quotas.details(provider, pane, now, live_claude);
            let text = quotas.text(provider, pane, now);
            let color = match quota.map(|q| q.used_percent) {
                Some(p) if p >= 90.0 => gpui::rgb(0xe07a5f).into(),
                _ => fg.opacity(0.8),
            };
            let mut item = div()
                .id(provider)
                .debug_selector(move || format!("usage-provider-{provider}"))
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .cursor_pointer()
                .tooltip(crate::tooltip::text(detail))
                .on_click(cx.listener(|view, _, window, cx| view.toggle_usage(window, cx)))
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
                .on_click(cx.listener(|view, _, window, cx| view.toggle_usage(window, cx)))
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
        let now = now_ms();
        let pane = view.ws.focused_pane();
        let live_claude = live_claude(view, pane);
        let mut reports: Vec<_> = quotas.reports.values().collect();
        reports.sort_by(|a, b| (&a.provider, &a.scope).cmp(&(&b.provider, &b.scope)));
        let mut content: Vec<AnyElement> = reports
            .iter()
            .enumerate()
            .map(|(index, q)| {
                let current = quotas
                    .provider(&q.provider, pane)
                    .is_some_and(|current| current.scope == q.scope);
                let mut section =
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(format!("{} · {}", q.provider, q.scope)),
                        )
                        .child(div().text_xs().text_color(fg.opacity(0.7)).child(format!(
                            "{} · observed {}s ago{}{}",
                            q.source,
                            now.saturating_sub(q.observed_at) / 1000,
                            if q.stale(now) { " · stale" } else { "" },
                            if current { "" } else { " · historical report" }
                        )))
                        .when(!q.account_known, |d| {
                            d.child(div().text_xs().text_color(fg.opacity(0.7))
                    .child("Account identity unavailable; each source session remains separate."))
                        })
                        .when(q.windows.is_empty(), |d| {
                            d.child(div().text_xs().child(
                                "Quota not reported for this session or authentication mode.",
                            ))
                        });
                for (window_index, w) in q.windows.iter().enumerate() {
                    let color: gpui::Hsla = if w.used_percent >= 90.0 {
                        gpui::rgb(0xe07a5f).into()
                    } else {
                        fg.opacity(0.8)
                    };
                    section = section.child(
                        div()
                            .id(gpui::ElementId::Name(
                                format!("quota-window-{index}-{window_index}").into(),
                            ))
                            .debug_selector(move || format!("quota-window-{index}-{window_index}"))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .pt_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1().min_w_0().child(w.name.clone()))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .child(format!("{:.0}% used", w.used_percent)),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h(px(7.0))
                                    .rounded_sm()
                                    .bg(fg.opacity(0.15))
                                    .child(
                                        div()
                                            .w(gpui::relative(w.used_percent as f32 / 100.0))
                                            .h_full()
                                            .rounded_sm()
                                            .bg(color),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(fg.opacity(0.7))
                                    .child(reset_text(w.resets_at, now)),
                            ),
                    );
                }
                section.into_any_element()
            })
            .collect();
        for provider in ["claude", "codex"] {
            if !reports.iter().any(|q| q.provider == provider) {
                content.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div().font_weight(gpui::FontWeight::BOLD).child(format!(
                                "{provider} · {}",
                                quotas.text(provider, pane, now)
                            )),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(fg.opacity(0.7))
                                .child(quotas.details(provider, pane, now, live_claude)),
                        )
                        .into_any_element(),
                );
            }
        }
        if let CodexQuotaStatus::Failed(error) = &quotas.codex {
            content.push(
                div()
                    .text_xs()
                    .child(format!("Codex · {}\n{}", error.label(), error.details()))
                    .into_any_element(),
            );
        }
        if let Some(resources) = &self.resources
            && self.resource_pane == pane
        {
            content.push(
                div()
                    .text_xs()
                    .child(resources.details())
                    .into_any_element(),
            );
        }
        if let Some(error) = &self.resource_error {
            content.push(div().text_xs().child(error.clone()).into_any_element());
        }
        Some(
            gpui::deferred(
                div()
                    .id("usage-popup")
                    .debug_selector(|| "usage-popup".into())
                    .absolute()
                    .bottom(px(30.0))
                    .left_2()
                    .w(px(480.0))
                    .max_w(gpui::relative(0.95))
                    .max_h(gpui::relative(0.7))
                    .flex()
                    .flex_col()
                    .min_h_0()
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
                                    .debug_selector(|| "usage-close".into())
                                    .cursor_pointer()
                                    .px_2()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.close_usage(window, cx)
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .id("usage-details-content")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .max_h(px(380.0))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .children(content),
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
        let (a, b) = panes();
        let mut quotas = ProviderQuotas::default();
        let report = claude_quota(
            &json!({"session_id":"s", "rate_limits":{"five_hour":{"used_percentage":25}}}),
            Some(a.raw()),
            100,
        )
        .unwrap();
        quotas.report(report.clone());
        // A later event without usage must not erase the reading or make it fresh.
        quotas.report(claude_quota(&json!({"session_id":"s"}), Some(b.raw()), 200).unwrap());
        let kept = quotas.provider("claude", Some(b)).unwrap();
        assert_eq!(kept.windows, report.windows);
        assert_eq!(kept.observed_at, 100);
        assert_eq!(kept.pane, Some(b.raw()));
        assert_eq!(quotas.text("claude", Some(b), 300101), "25% · 5h · stale");
        quotas.report(claude_quota(&json!({"session_id":"other"}), Some(b.raw()), 300).unwrap());
        assert_eq!(
            quotas.text("claude", Some(b), 300),
            "Waiting for usage",
            "a session without a known configuration folder never inherits another's quota"
        );
    }

    /// A Claude status line report from `session` in `pane`, with a 5h
    /// window at `used` percent, from the configuration folder `dir`.
    fn claude(session: &str, pane: PaneId, dir: &str, used: Option<f64>, at: u64) -> QuotaSnapshot {
        let value = match used {
            Some(used) => {
                json!({"session_id":session, "rate_limits":{"five_hour":{"used_percentage":used}}})
            }
            None => json!({"session_id":session}),
        };
        let mut report = claude_quota(&value, Some(pane.raw()), at).unwrap();
        report.config_dir = Some(dir.into());
        report
    }

    fn panes() -> (PaneId, PaneId) {
        let mut ws = chda_core::Workspace::new();
        (ws.new_tab().1, ws.new_tab().1)
    }

    #[test]
    fn a_session_without_a_reading_shows_its_configuration_folders_latest_usage() {
        let (a, b) = panes();
        let mut quotas = ProviderQuotas::default();
        quotas.report(claude("a", a, "/home/.claude", Some(25.0), 100));
        // The focused session is newer and has no reading yet (#244).
        quotas.report(claude("b", b, "/home/.claude", None, 200));
        assert_eq!(quotas.text("claude", Some(b), 300), "25% · 5h");
        assert!(
            quotas
                .details("claude", Some(b), 300, true)
                .contains("From another Claude Code session with the same configuration folder")
        );
        assert_eq!(quotas.text("claude", Some(a), 300), "25% · 5h");
        // A newer reading from either session wins for both.
        quotas.report(claude("b", b, "/home/.claude", Some(40.0), 400));
        assert_eq!(quotas.text("claude", Some(a), 500), "40% · 5h");
    }

    #[test]
    fn readings_never_cross_configuration_folders() {
        let (a, b) = panes();
        let mut quotas = ProviderQuotas::default();
        quotas.report(claude("a", a, "/home/.claude", Some(25.0), 100));
        quotas.report(claude("b", b, "/work/.claude", None, 200));
        assert_eq!(quotas.text("claude", Some(b), 300), "Waiting for usage");
        assert!(
            quotas
                .details("claude", Some(b), 300, true)
                .contains("after the next Claude response")
        );
        // A pane without a Claude session shows the newest reading.
        assert_eq!(quotas.text("claude", None, 300), "25% · 5h");
    }

    #[test]
    fn empty_states_say_what_chda_is_waiting_for() {
        let (a, _) = panes();
        let mut quotas = ProviderQuotas::default();
        assert!(
            quotas
                .details("claude", Some(a), 0, true)
                .contains("has not reported through chda's status line")
        );
        assert!(
            quotas
                .details("claude", Some(a), 0, false)
                .contains("after its next response")
        );
        let mut answered = claude_quota(
            &json!({"session_id":"a", "cost":{"total_api_duration_ms":3105}}),
            Some(a.raw()),
            100,
        )
        .unwrap();
        answered.config_dir = Some("/home/.claude".into());
        quotas.report(answered);
        assert_eq!(quotas.text("claude", Some(a), 200), "Waiting for usage");
        assert!(
            quotas
                .details("claude", Some(a), 200, true)
                .contains("this sign-in may not have them")
        );
    }

    #[test]
    fn the_last_reading_per_folder_survives_a_restart_marked_as_previous() {
        let dir = std::env::temp_dir().join(format!("chda-quota-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = panes();
        let mut quotas = ProviderQuotas::default();
        quotas.report(claude("a", a, "/home/.claude", Some(25.0), 100));
        quotas.save_cache(&dir, 100);
        // Within a minute nothing is rewritten.
        quotas.report(claude("a", a, "/home/.claude", Some(30.0), 200));
        quotas.save_cache(&dir, 200);

        let mut restarted = ProviderQuotas::default();
        restarted.load_cache(&dir);
        restarted.report(claude("b", b, "/home/.claude", None, 300));
        assert_eq!(
            restarted.text("claude", Some(b), 400),
            "25% · 5h · previous run"
        );
        assert!(
            restarted
                .details("claude", Some(b), 400, true)
                .contains("Kept from before chda restarted")
        );
        restarted.report(claude("b", b, "/home/.claude", Some(50.0), 500));
        assert_eq!(restarted.text("claude", Some(b), 600), "50% · 5h");
        std::fs::remove_dir_all(dir).unwrap();
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
                .details("codex", None, 200, false)
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
