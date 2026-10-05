//! Provider-reported quota windows. Never inferred from session tokens.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub name: String,
    pub used_percent: f64,
    pub window_minutes: Option<u64>,
    pub resets_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    pub provider: String,
    /// Account when the provider identifies it; otherwise the exact source session.
    pub scope: String,
    pub account_known: bool,
    pub source: String,
    pub observed_at: u64,
    pub pane: Option<u64>,
    pub session: Option<String>,
    pub windows: Vec<QuotaWindow>,
}

fn window(
    name: &str,
    value: &Value,
    percent: &str,
    reset: &str,
    minutes: Option<u64>,
) -> Option<QuotaWindow> {
    let used_percent = value.get(percent)?.as_f64()?;
    if !used_percent.is_finite() || !(0.0..=100.0).contains(&used_percent) {
        return None;
    }
    Some(QuotaWindow {
        name: name.into(),
        used_percent,
        window_minutes: minutes,
        resets_at: value.get(reset).and_then(Value::as_u64),
    })
}

pub fn claude_quota(value: &Value, pane: Option<u64>, now: u64) -> Option<QuotaSnapshot> {
    let session = value.get("session_id")?.as_str()?.to_owned();
    if session.is_empty() {
        return None;
    }
    let limits = value.get("rate_limits");
    let windows = [
        ("five_hour", "5h", Some(300)),
        ("seven_day", "7d", Some(10080)),
        ("spend_limit", "Spend", None),
    ]
    .into_iter()
    .filter_map(|(key, label, minutes)| {
        window(
            label,
            limits?.get(key)?,
            "used_percentage",
            "resets_at",
            minutes,
        )
    })
    .collect();
    Some(QuotaSnapshot {
        provider: "claude".into(),
        scope: format!("Session {session}"),
        account_known: false,
        source: "Claude Code status line".into(),
        observed_at: now,
        pane,
        session: Some(session),
        windows,
    })
}

pub fn codex_quota(value: &Value, account: &Value, now: u64) -> Option<QuotaSnapshot> {
    let account = account.get("account")?;
    if account.get("type")?.as_str()? != "chatgpt" {
        return None;
    }
    let scope = account
        .get("email")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?
        .to_owned();
    let limits = value.get("rateLimitsByLimitId").and_then(Value::as_object);
    let mut windows = Vec::new();
    let buckets: Vec<(&str, &Value)> = match limits {
        Some(limits) => limits.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        None => vec![("codex", value.get("rateLimits")?)],
    };
    for (id, bucket) in buckets {
        for key in ["primary", "secondary"] {
            let Some(raw) = bucket.get(key) else {
                continue;
            };
            let minutes = raw.get("windowDurationMins").and_then(Value::as_u64);
            let name = match minutes {
                Some(m) if m % 1440 == 0 => format!("{}d", m / 1440),
                Some(m) if m % 60 == 0 => format!("{}h", m / 60),
                Some(m) => format!("{m}m"),
                None => key.into(),
            };
            if let Some(w) = window(
                &format!("{id} {name}"),
                raw,
                "usedPercent",
                "resetsAt",
                minutes,
            ) {
                windows.push(w);
            }
        }
    }
    Some(QuotaSnapshot {
        provider: "codex".into(),
        scope,
        account_known: true,
        source: "Codex App Server · local CLI account".into(),
        observed_at: now,
        pane: None,
        session: None,
        windows,
    })
}

impl QuotaSnapshot {
    pub fn stale(&self, now: u64) -> bool {
        now.saturating_sub(self.observed_at) > 300_000
    }
    pub fn representative(&self) -> Option<&QuotaWindow> {
        // The window nearest exhaustion, with a stable tie-break on its label.
        self.windows.iter().max_by(|a, b| {
            a.used_percent
                .total_cmp(&b.used_percent)
                .then_with(|| b.name.cmp(&a.name))
        })
    }
    pub fn details(&self, now: u64) -> String {
        let mut lines = vec![format!(
            "{}\n{}\nUpdated {}s ago{}",
            self.scope,
            self.source,
            now.saturating_sub(self.observed_at) / 1000,
            if self.stale(now) { " · stale" } else { "" }
        )];
        for w in &self.windows {
            lines.push(format!(
                "{}: {:.0}% used · {}",
                w.name,
                w.used_percent,
                reset_text(w.resets_at, now)
            ));
        }
        if self.windows.is_empty() {
            lines.push("Quota not available for this session or authentication mode".into());
        }
        if !self.account_known {
            lines.push("Account identity is not provided; sessions are kept separate, never added together".into());
        }
        lines.join("\n")
    }
}

pub fn reset_text(reset: Option<u64>, now_ms: u64) -> String {
    let Some(reset) = reset else {
        return "reset time unavailable".into();
    };
    let seconds = reset.saturating_sub(now_ms / 1000);
    if seconds == 0 {
        return "awaiting a new quota report".into();
    }
    let days = seconds / 86400;
    let hours = seconds % 86400 / 3600;
    let minutes = seconds % 3600 / 60;
    if days > 0 {
        format!("resets in {days}d {hours}h")
    } else if hours > 0 {
        format!("resets in {hours}h {minutes}m")
    } else {
        format!("resets in {}m", minutes.max(1))
    }
}

/// Read the local CLI account through the supported protocol; no token files,
/// private provider endpoint, login, thread creation or model request.
pub fn read_codex(executable: &std::path::Path, now: u64) -> Result<QuotaSnapshot, String> {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        process::{Command, Stdio},
        sync::mpsc,
        time::{Duration, Instant},
    };
    let mut command = Command::new(executable);
    command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let result = chda_pty::with_process(&mut command, |child| {
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let count = reader.by_ref().take(1_048_577).read_line(&mut line);
                if !matches!(count, Ok(n) if n > 0)
                    || line.len() > 1_048_576
                    || tx.send(line).is_err()
                {
                    break;
                }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut request = |id: u64, method: &str, params: Value| -> std::io::Result<Value> {
            writeln!(
                stdin,
                "{}",
                serde_json::json!({"id":id,"method":method,"params":params})
            )?;
            stdin.flush()?;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let line = rx.recv_timeout(remaining).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::TimedOut, "Codex quota query timed out")
                })?;
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    if let Some(result) = value.get("result") {
                        if id == 1 {
                            writeln!(stdin, "{}", serde_json::json!({"method":"initialized"}))?;
                            stdin.flush()?;
                        }
                        return Ok(result.clone());
                    }
                    return Err(std::io::Error::other(
                        "Codex account quota is unavailable or requires ChatGPT authentication",
                    ));
                }
            }
        };
        request(
            1,
            "initialize",
            serde_json::json!({"clientInfo":{"name":"chda","version":env!("CARGO_PKG_VERSION")}}),
        )?;
        let account = request(2, "account/read", serde_json::json!({"refreshToken":false}))?;
        let limits = request(3, "account/rateLimits/read", serde_json::json!({}))?;
        codex_quota(&limits, &account, now).ok_or_else(|| {
            std::io::Error::other("No supported ChatGPT quota report from the local CLI account")
        })
    });
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn claude_keeps_windows_missing_data_and_session_scope() {
        let report = claude_quota(&json!({"session_id":"s", "rate_limits":{"five_hour":{"used_percentage":90.4,"resets_at":1000},"seven_day":{"used_percentage":100,"resets_at":2000}}}), Some(5), 100).unwrap();
        assert_eq!(report.windows.len(), 2);
        assert_eq!(report.representative().unwrap().used_percent, 100.0);
        assert!(!report.account_known);
        assert_eq!(report.pane, Some(5));
        assert!(
            claude_quota(&json!({"session_id":"s"}), None, 0)
                .unwrap()
                .windows
                .is_empty()
        );
        assert!(
            claude_quota(
                &json!({"session_id":"s","rate_limits":{"five_hour":{"used_percentage":-1}}}),
                None,
                0
            )
            .unwrap()
            .windows
            .is_empty()
        );
    }
    #[test]
    fn codex_keeps_account_and_multiple_buckets_without_adding_percentages() {
        let quota = codex_quota(&json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":1,"windowDurationMins":300,"resetsAt":1000},"secondary":{"usedPercent":90,"windowDurationMins":10080,"resetsAt":2000}}}}), &json!({"account":{"type":"chatgpt","email":"test@example.test"}}), 100).unwrap();
        assert_eq!(quota.windows.len(), 2);
        assert_eq!(quota.representative().unwrap().used_percent, 90.0);
        assert_eq!(quota.scope, "test@example.test");
        assert!(quota.stale(300101));
        assert_eq!(reset_text(Some(0), 1000), "awaiting a new quota report");
        assert_eq!(reset_text(None, 1000), "reset time unavailable");
        assert!(codex_quota(&json!({}), &json!({"account":{"type":"apiKey"}}), 0).is_none());
    }
}
