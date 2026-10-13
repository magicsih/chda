//! Codex: CLI session rollouts, runtime status through OSC titles, and
//! menu-launch notify hooks supplying the conversation ID for restore.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::session::{file_mtime_ms, parse_rfc3339_ms, snippet};
use crate::usage::{LimitUsage, ModelUsage, Usage};
use crate::{AgentAdapter, AgentId, AgentSession, HookInstallReport, SessionId, which};

pub struct CodexAdapter;

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))
}

/// The user's own `notify` program from `~/.codex/config.toml`, so chda can
/// run it after its own hook instead of replacing it.
pub fn existing_notify(config_toml: &str) -> Option<Vec<String>> {
    let line = config_toml
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("notify") && l[6..].trim_start().starts_with('='))?;
    let value = line.split_once('=')?.1.trim();
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    let items: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    (!items.is_empty()).then_some(items)
}

/// `-c notify=[...]` value: chda's hook, then the user's program if any.
pub fn notify_override(hook_bin: &Path, existing: Option<&[String]>) -> String {
    let mut argv: Vec<String> = vec![
        hook_bin.to_string_lossy().into_owned(),
        "hook".into(),
        "codex".into(),
    ];
    if let Some(existing) = existing {
        argv.push("--then".into());
        argv.extend(existing.iter().cloned());
    }
    let quoted: Vec<String> = argv
        .iter()
        .map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("notify=[{}]", quoted.join(","))
}

/// Same notify bridge for menu launches and commands typed in an integrated shell.
pub fn notify_config(hook_bin: &Path) -> String {
    let existing = codex_home()
        .and_then(|h| fs::read_to_string(h.join("config.toml")).ok())
        .and_then(|t| existing_notify(&t));
    notify_override(hook_bin, existing.as_deref())
}

/// Per-launch title format, shared by menu launches and interactive shells.
/// No user config or notify command is changed.
pub const CODEX_TITLE_CONFIG: &str = "tui.terminal_title=[\"activity\",\"app-name\",\"run-state\"]";
pub const CODEX_TITLE_ENV: &str = "CHDA_CODEX_TITLE_CONFIG";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexRunState {
    Ready,
    Working,
    WaitingInput,
}

/// Only the title format chda requests is recognized. In Codex, `Waiting`
/// means a background terminal is running, not a request for user input.
pub fn parse_codex_title(title: &str) -> Option<CodexRunState> {
    let title = title.trim();
    if matches!(
        title,
        "[ ! ] Action Required | codex" | "[ . ] Action Required | codex"
    ) {
        return Some(CodexRunState::WaitingInput);
    }
    let title = match title.split_once(' ') {
        Some((
            "⠋" | "⠙" | "⠹" | "⠸" | "⠼" | "⠴" | "⠦" | "⠧" | "⠇" | "⠏" | "|" | "/" | "-" | "\\",
            rest,
        )) => rest,
        _ => title,
    };
    let state = match title.strip_prefix("codex | ")? {
        "Ready" | "Starting" => CodexRunState::Ready,
        "Working" | "Thinking" | "Waiting" => CodexRunState::Working,
        _ => return None,
    };
    Some(state)
}

impl AgentAdapter for CodexAdapter {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn display_name(&self) -> &str {
        "Codex"
    }

    fn short_label(&self) -> String {
        "CX".into()
    }

    fn is_installed(&self, path: &std::ffi::OsStr) -> bool {
        which("codex", path).is_some()
    }

    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, hook_bin: &Path) -> Command {
        let mut cmd = Command::new("codex");
        cmd.current_dir(cwd);
        cmd.arg("-c")
            .arg(notify_config(hook_bin))
            .arg("-c")
            .arg(CODEX_TITLE_CONFIG);
        if let Some(id) = resume {
            cmd.arg("resume").arg(&id.0);
        }
        cmd
    }

    fn session_roots(&self) -> Vec<PathBuf> {
        codex_home()
            .map(|h| h.join("sessions"))
            .into_iter()
            .collect()
    }

    fn parse_session(&self, file: &Path) -> Option<AgentSession> {
        parse_rollout(file)
    }

    fn install_hooks(&self, _: &Path, _: &Path) -> io::Result<HookInstallReport> {
        // Nothing to write: the notify override travels on the command line.
        Ok(HookInstallReport {
            file: None,
            note: Some(
                "status comes from Codex terminal titles; notify keeps the session ID for restore"
                    .into(),
            ),
        })
    }
}

fn user_message_text(item: &Value) -> Option<String> {
    match item.get("content")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let text: Vec<&str> = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            (!text.is_empty()).then(|| text.join(" "))
        }
        _ => None,
    }
}

// These eight contextual wrappers recognized by Codex 0.162.1 use complete,
// case-insensitive markers with leading/trailing whitespace ignored. A partial wrapper or
// markers inside a normal prompt remain user input.
fn is_context_message(text: &str) -> bool {
    const WRAPPERS: &[(&str, &str)] = &[
        ("# AGENTS.md instructions", "</INSTRUCTIONS>"),
        ("<environment_context>", "</environment_context>"),
        ("<skill>", "</skill>"),
        ("<user_shell_command>", "</user_shell_command>"),
        ("<turn_aborted>", "</turn_aborted>"),
        ("<subagent_notification>", "</subagent_notification>"),
        (
            "<agent_message_board_notification>",
            "</agent_message_board_notification>",
        ),
        ("<recommended_plugins>", "</recommended_plugins>"),
    ];
    let text = text.trim();
    WRAPPERS.iter().any(|(open, close)| {
        text.get(..open.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(open))
            && text
                .get(text.len().saturating_sub(close.len())..)
                .is_some_and(|s| s.eq_ignore_ascii_case(close))
    })
}

fn response_user_text(item: &Value) -> Option<String> {
    if item.get("type").and_then(Value::as_str) != Some("message")
        || item.get("role").and_then(Value::as_str) != Some("user")
    {
        return None;
    }
    let blocks = item.get("content")?.as_array()?;
    let text: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("input_text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    if text.is_empty() || text.iter().any(|s| is_context_message(s)) {
        return None;
    }
    Some(text.join(" "))
}

/// Token usage of a rollout, fed line by line. `token_count` events carry
/// running totals for the session; each increase is counted toward the
/// model of the turn it happened in (`turn_context`), so a session that
/// switched models splits correctly. The last rate-limit reading is kept.
#[derive(Default)]
struct UsageTracker {
    usage: Usage,
    model: String,
    /// Last totals: input, cached input, output.
    prev: (u64, u64, u64),
}

impl UsageTracker {
    fn token_count(&mut self, payload: &Value) {
        if let Some(total) = payload.pointer("/info/total_token_usage") {
            let n = |key: &str| total.get(key).and_then(Value::as_u64).unwrap_or(0);
            let now = (
                n("input_tokens"),
                n("cached_input_tokens"),
                n("output_tokens"),
            );
            // Totals only grow; a drop means they started over.
            let base = if now.0 < self.prev.0 || now.2 < self.prev.2 {
                (0, 0, 0)
            } else {
                self.prev
            };
            let input = now.0 - base.0;
            let cached = now.1.saturating_sub(base.1).min(input);
            self.usage.add(&ModelUsage {
                model: self.model.clone(),
                input: input - cached,
                cache_read: cached,
                output: now.2 - base.2,
                ..Default::default()
            });
            self.prev = now;
        }
        if let Some(limit) = payload.pointer("/rate_limits/primary") {
            let used = limit.get("used_percent").and_then(Value::as_f64);
            let window = limit.get("window_minutes").and_then(Value::as_u64);
            if let (Some(used), Some(window)) = (used, window) {
                self.usage.limit = Some(LimitUsage {
                    used_percent: used.round() as u32,
                    window_minutes: window,
                });
            }
        }
    }
}

/// Parse both Codex history modes. Explicit user events/items are authoritative;
/// response-only rollouts fall back to user input without contextual wrappers.
/// Model-input copies never double-count an explicit prompt.
pub fn parse_rollout(file: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut started_at = None;
    let mut first = None;
    let mut count = 0;
    let mut response_first = None;
    let mut response_count = 0;
    let mut tracker = UsageTracker::default();
    let needles = &[
        "session_meta",
        "UserMessage",
        "\"user_message\"",
        "\"input_text\"",
        "\"turn_context\"",
        "\"token_count\"",
    ];
    for line in crate::session::matching_lines(file, needles).ok()? {
        // A turn's context repeats the whole instructions; only its model is
        // needed, and an unescaped `"model":"` can only be a real key.
        if line.contains("\"type\":\"turn_context\"") {
            if let Some(start) = line.find("\"model\":\"").map(|i| i + "\"model\":\"".len())
                && let Some(len) = line[start..].find('"')
            {
                tracker.model = line[start..start + len].to_owned();
            }
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let payload = v.get("payload");
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                let p = payload?;
                id = p
                    .get("id")
                    .or_else(|| p.get("session_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                cwd = p.get("cwd").and_then(Value::as_str).map(PathBuf::from);
                started_at = p
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_ms);
            }
            Some("event_msg") => {
                let Some(p) = payload else { continue };
                let text = match p.get("type").and_then(Value::as_str) {
                    Some("token_count") => {
                        tracker.token_count(p);
                        continue;
                    }
                    Some("user_message") => {
                        p.get("message").and_then(Value::as_str).map(str::to_owned)
                    }
                    Some("item_completed") => p
                        .get("item")
                        .filter(|item| {
                            item.get("type").and_then(Value::as_str) == Some("UserMessage")
                        })
                        .and_then(user_message_text),
                    _ => None,
                };
                if let Some(text) = text {
                    count += 1;
                    if first.is_none() {
                        first = Some(text);
                    }
                }
            }
            Some("response_item") => {
                if let Some(text) = payload.and_then(response_user_text) {
                    response_count += 1;
                    if response_first.is_none() {
                        response_first = Some(text);
                    }
                }
            }
            _ => {}
        }
    }
    if first.is_none() {
        first = response_first;
        count = response_count;
    }
    let first = first?;
    let last_active_at = file_mtime_ms(file)?;
    Some(AgentSession {
        id: SessionId(id?),
        agent: AgentId::Codex,
        cwd: cwd?,
        started_at: started_at.unwrap_or(last_active_at),
        last_active_at,
        snippet: snippet(&first, 120),
        message_count: count,
        file: file.to_path_buf(),
        usage: tracker.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_test_rollout(name: &str, rows: &[Value]) -> Option<AgentSession> {
        let dir = std::env::temp_dir().join(format!("chda-codex-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout.jsonl");
        fs::write(
            &file,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let session = parse_rollout(&file);
        fs::remove_dir_all(dir).unwrap();
        session
    }

    fn metadata() -> Value {
        serde_json::json!({"type":"session_meta", "payload":{
            "id":"thread-legacy", "cwd":"/work/app",
            "timestamp":"2026-10-12T18:00:00Z"
        }})
    }

    fn user_response(text: &str) -> Value {
        serde_json::json!({"type":"response_item", "payload":{
            "type":"message", "role":"user",
            "content":[{"type":"input_text", "text":text}]
        }})
    }

    #[test]
    fn legacy_user_events_keep_repeated_prompts_without_counting_response_copies() {
        let message = serde_json::json!({"type":"event_msg", "payload":{
            "type":"user_message", "message":"같은 요청을 다시 실행해줘"
        }});
        let session = parse_test_rollout(
            "legacy",
            &[
                metadata(),
                user_response(
                    "# AGENTS.md instructions for /work/app\n<INSTRUCTIONS>context</INSTRUCTIONS>",
                ),
                user_response("같은 요청을 다시 실행해줘"),
                message.clone(),
                user_response("같은 요청을 다시 실행해줘"),
                message,
            ],
        )
        .expect("the default Codex history must appear in Resume");
        assert_eq!(session.id, SessionId("thread-legacy".into()));
        assert_eq!(session.cwd, PathBuf::from("/work/app"));
        assert_eq!(
            session.started_at,
            parse_rfc3339_ms("2026-10-12T18:00:00Z").unwrap()
        );
        assert_eq!(session.snippet, "같은 요청을 다시 실행해줘");
        assert_eq!(session.message_count, 2);
    }

    #[test]
    fn response_only_history_uses_user_input_and_ignores_other_roles() {
        let session = parse_test_rollout("responses", &[
            metadata(),
            serde_json::json!({"type":"response_item", "payload":{
                "type":"message", "role":"developer", "content":[{"type":"input_text", "text":"developer context"}]
            }}),
            user_response("코드 정리해줘"),
            serde_json::json!({"type":"response_item", "payload":{
                "type":"message", "role":"assistant", "content":[{"type":"output_text", "text":"answer"}]
            }}),
            user_response("코드 정리해줘"),
        ]).expect("a response-only saved conversation must remain resumable");
        assert_eq!(session.snippet, "코드 정리해줘");
        assert_eq!(session.message_count, 2);
        assert_eq!(session.cwd, PathBuf::from("/work/app"));
    }

    #[test]
    fn response_fallback_excludes_complete_context_blocks_but_keeps_normal_prompts() {
        let context = [
            "  # agents.md instructions for /work/app\n<INSTRUCTIONS>context</instructions>  ",
            " <ENVIRONMENT_CONTEXT>context</environment_context> ",
            "<skill>context</skill>",
            "<user_shell_command>context</user_shell_command>",
            "<turn_aborted>context</turn_aborted>",
            "<subagent_notification>context</subagent_notification>",
            "<agent_message_board_notification>context</agent_message_board_notification>",
            "<recommended_plugins>context</recommended_plugins>",
        ];
        let mut rows = vec![metadata()];
        rows.extend(context.iter().map(|s| user_response(s)));
        rows.push(serde_json::json!({"type":"response_item", "payload":{
            "type":"message", "role":"user", "content":[
                {"type":"input_text", "text":"<environment_context>context</environment_context>"},
                {"type":"input_text", "text":"another context block"}
            ]
        }}));
        rows.push(serde_json::json!({"type":"response_item", "payload":{
            "type":"message", "role":"user", "content":[{"type":"output_text", "text":"not input"}]
        }}));
        assert!(parse_test_rollout("context-only", &rows).is_none());
        rows.push(user_response(
            "Explain <environment_context>context</environment_context>",
        ));
        rows.push(user_response(
            "<environment_context>this prompt has no closing wrapper",
        ));
        let session = parse_test_rollout("context-and-prompt", &rows).unwrap();
        assert_eq!(
            session.snippet,
            "Explain <environment_context>context</environment_context>"
        );
        assert_eq!(session.message_count, 2);
    }

    #[test]
    fn paginated_user_items_take_precedence_over_response_copies() {
        let session = parse_test_rollout("paginated", &[
            metadata(), user_response("context copied into model input"),
            serde_json::json!({"type":"event_msg", "payload":{
                "type":"item_completed", "item":{"type":"UserMessage", "content":[{"type":"text", "text":"visible request"}]}
            }}),
        ]).unwrap();
        assert_eq!(session.snippet, "visible request");
        assert_eq!(session.message_count, 1);
    }

    #[test]
    fn runtime_titles_distinguish_work_user_input_and_background_waiting() {
        for state in ["Working", "Thinking", "Waiting"] {
            for prefix in ["", "⠋ ", "⠏ ", "| "] {
                assert_eq!(
                    parse_codex_title(&format!("{prefix}codex | {state}")),
                    Some(CodexRunState::Working)
                );
            }
        }
        for prefix in ["[ ! ]", "[ . ]"] {
            assert_eq!(
                parse_codex_title(&format!("{prefix} Action Required | codex")),
                Some(CodexRunState::WaitingInput)
            );
        }
        assert_eq!(
            parse_codex_title("codex | Ready"),
            Some(CodexRunState::Ready)
        );
        for title in [
            "",
            "Working",
            "codex | unknown",
            "editor | Working",
            "codex | Ready | notes",
        ] {
            assert_eq!(parse_codex_title(title), None);
        }
    }

    #[test]
    fn menu_and_resume_launches_include_status_titles() {
        let cmd = CodexAdapter.launch_command(
            Path::new("/work"),
            Some(&SessionId("thread".into())),
            Path::new("/bin/chda"),
        );
        let args: Vec<_> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[2..], ["-c", CODEX_TITLE_CONFIG, "resume", "thread"]);
    }

    #[test]
    fn rollout_usage_follows_running_totals_per_model() {
        let dir = std::env::temp_dir().join(format!("chda-codex-usage-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout-u.jsonl");
        let count = |input: u64, cached: u64, output: u64, used: f64| {
            format!(
                r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output}}}}},"rate_limits":{{"primary":{{"used_percent":{used},"window_minutes":10080}}}}}}}}"#
            )
        };
        fs::write(&file, [
            r#"{"type":"session_meta","payload":{"id":"u1","cwd":"/w"}}"#.to_owned(),
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":"go"}}}"#.to_owned(),
            r#"{"type":"turn_context","payload":{"model":"gpt-6.1-sol"}}"#.to_owned(),
            count(1000, 800, 50, 30.0),
            // The same totals reported twice add nothing.
            count(1000, 800, 50, 30.0),
            r#"{"type":"turn_context","payload":{"model":"gpt-6-astra"}}"#.to_owned(),
            count(3000, 2500, 150, 33.6),
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#.to_owned(),
        ].join("\n")).unwrap();
        let s = parse_rollout(&file).unwrap();
        let sol = &s.usage.models[0];
        assert_eq!(
            (sol.model.as_str(), sol.input, sol.cache_read, sol.output),
            ("gpt-6.1-sol", 200, 800, 50)
        );
        let astra = &s.usage.models[1];
        assert_eq!(
            (
                astra.model.as_str(),
                astra.input,
                astra.cache_read,
                astra.output
            ),
            ("gpt-6-astra", 300, 1700, 100)
        );
        assert_eq!(
            s.usage.limit,
            Some(LimitUsage {
                used_percent: 34,
                window_minutes: 10080
            })
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_a_rollout_and_chains_notify() {
        let dir = std::env::temp_dir().join(format!("chda-codex-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout.jsonl");
        fs::write(&file, concat!(
            r#"{"timestamp":"2026-05-22T08:22:31.613Z","type":"session_meta","payload":{"id":"019e","session_id":"019e","timestamp":"2026-05-22T08:22:31.540Z","cwd":"/work/app"}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"task_started"}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"이 작업 진행해줘.  둘째 줄"}]}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":"x"}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":"again"}}}"#, "\n",
        )).unwrap();
        let s = parse_rollout(&file).unwrap();
        assert_eq!(s.id, SessionId("019e".into()));
        assert_eq!(s.cwd, PathBuf::from("/work/app"));
        assert_eq!(s.snippet, "이 작업 진행해줘. 둘째 줄");
        assert_eq!(s.message_count, 2);
        assert_eq!(
            s.started_at,
            parse_rfc3339_ms("2026-05-22T08:22:31.540Z").unwrap()
        );

        let cfg = "model = \"x\"\nnotify = [\"/Apps/Client.app/Contents/MacOS/Client\", \"turn-ended\"]\n";
        let existing = existing_notify(cfg).unwrap();
        assert_eq!(
            existing,
            vec!["/Apps/Client.app/Contents/MacOS/Client", "turn-ended"]
        );
        assert_eq!(
            notify_override(Path::new("/usr/local/bin/chda"), Some(&existing)),
            r#"notify=["/usr/local/bin/chda","hook","codex","--then","/Apps/Client.app/Contents/MacOS/Client","turn-ended"]"#
        );
        assert_eq!(existing_notify("model = \"x\"\n"), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
