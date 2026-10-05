//! Claude Code: `claude` CLI, `~/.claude/projects` transcripts, hooks
//! injected per launch with `--settings`.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::session::{file_mtime_ms, parse_rfc3339_ms, snippet};
use crate::usage::{ModelUsage, Usage};
use crate::{
    AgentAdapter, AgentId, AgentSession, HookInstallReport, SessionId, shell_quote, which,
    write_if_changed,
};

pub struct ClaudeAdapter;

const HOOK_EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "Stop",
    "Notification",
    "PermissionRequest",
    "SessionEnd",
];

/// Where the per-launch settings file lives under the data dir.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks").join("claude-settings.json")
}

/// The settings JSON that makes Claude Code call `chda hook claude`.
pub fn hooks_settings(hook_bin: &Path) -> Value {
    let command = format!("{} hook claude", shell_quote(hook_bin));
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        hooks.insert(
            event.into(),
            json!([{ "hooks": [{ "type": "command", "command": command, "timeout": 10 }] }]),
        );
    }
    json!({ "hooks": hooks })
}

impl AgentAdapter for ClaudeAdapter {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }

    fn short_label(&self) -> String {
        "CC".into()
    }

    fn executable(&self, home: &Path, path: &std::ffi::OsStr) -> Option<PathBuf> {
        which("claude", path).or_else(|| {
            let native = home.join(".local/bin/claude");
            chda_pty::is_executable(&native)
                .then(|| native.canonicalize().ok())
                .flatten()
        })
    }

    fn is_installed(&self, path: &std::ffi::OsStr) -> bool {
        which("claude", path).is_some()
    }

    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, hook_bin: &Path) -> Command {
        let mut cmd = Command::new("claude");
        cmd.current_dir(cwd);
        if let Some(data_dir) = crate::hook::data_dir() {
            cmd.arg("--settings").arg(settings_path(&data_dir));
        }
        let _ = hook_bin;
        if let Some(id) = resume {
            cmd.arg("--resume").arg(&id.0);
        }
        cmd
    }

    fn session_roots(&self) -> Vec<PathBuf> {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".claude").join("projects"))
            .into_iter()
            .collect()
    }

    /// Claude Code files transcripts under `projects/<directory with every
    /// non-alphanumeric character replaced by '-'>`, so only folders that
    /// match a worktree (or a directory inside one) need reading.
    fn session_dirs(&self, worktrees: &[PathBuf]) -> Vec<PathBuf> {
        let prefixes: Vec<String> = worktrees.iter().map(|w| project_dir_name(w)).collect();
        self.session_roots()
            .into_iter()
            .flat_map(|root| fs::read_dir(root).into_iter().flatten().flatten())
            .map(|entry| entry.path())
            .filter(|dir| {
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
                name.is_some_and(|name| {
                    prefixes.iter().any(|p| {
                        name == *p
                            || name
                                .strip_prefix(p.as_str())
                                .is_some_and(|r| r.starts_with('-'))
                    })
                })
            })
            .collect()
    }

    fn parse_session(&self, file: &Path) -> Option<AgentSession> {
        parse_transcript(file)
    }

    fn install_hooks(&self, data_dir: &Path, hook_bin: &Path) -> io::Result<HookInstallReport> {
        let path = settings_path(data_dir);
        write_if_changed(
            &path,
            &serde_json::to_string_pretty(&hooks_settings(hook_bin))?,
        )?;
        Ok(HookInstallReport {
            file: Some(path),
            note: None,
        })
    }
}

/// The folder name Claude Code uses for a directory's transcripts.
fn project_dir_name(dir: &Path) -> String {
    dir.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Text of a user message, if it is a real prompt rather than a tool result.
fn user_text(message: &Value) -> Option<String> {
    match message.get("content")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let text: Vec<&str> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            (!text.is_empty()).then(|| text.join(" "))
        }
        _ => None,
    }
}

/// The parts of an assistant line that carry usage. Deserializing into
/// this skips the rest of the line (message content, tool calls) without
/// building it, which matters on large transcripts.
#[derive(serde::Deserialize)]
struct UsageLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<UsageMessage>,
}

#[derive(serde::Deserialize)]
struct UsageMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
}

#[derive(serde::Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    cache_creation: Option<CacheSplit>,
}

#[derive(serde::Deserialize)]
struct CacheSplit {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

/// Token usage of one assistant line: its message id, then the usage.
/// Claude Code writes one response over several lines that repeat the same
/// id and usage, so callers count each id once.
fn line_usage(line: &str) -> Option<(String, ModelUsage)> {
    quick_line_usage(line).or_else(|| full_line_usage(line))
}

/// [`line_usage`] without scanning the message content: an unescaped
/// `"key":` can only be a real key (quotes inside JSON strings are
/// escaped), so the message id, model and usage object are found by
/// position and only the usage object is parsed. `None` when any of them
/// is missing; the full parse then decides.
fn quick_line_usage(line: &str) -> Option<(String, ModelUsage)> {
    if !line.contains("\"type\":\"assistant\"") {
        return None;
    }
    let string_after = |key: &str| {
        let start = line.find(key)? + key.len();
        let end = start + line[start..].find('"')?;
        Some(line[start..end].to_owned())
    };
    let id = string_after("\"id\":\"msg_").map(|rest| format!("msg_{rest}"))?;
    let model = string_after("\"model\":\"")?;
    // The message's usage comes after its content, so take the last one.
    let at = line.rfind("\"usage\":{")? + "\"usage\":".len();
    let usage: RawUsage = serde_json::Deserializer::from_str(&line[at..])
        .into_iter()
        .next()?
        .ok()?;
    Some((id, model_usage(model, usage)))
}

fn model_usage(model: String, usage: RawUsage) -> ModelUsage {
    let (write_5m, write_1h) = match usage.cache_creation {
        Some(split) => (
            split.ephemeral_5m_input_tokens,
            split.ephemeral_1h_input_tokens,
        ),
        None => (usage.cache_creation_input_tokens, 0),
    };
    ModelUsage {
        model,
        input: usage.input_tokens,
        cache_write_5m: write_5m,
        cache_write_1h: write_1h,
        cache_read: usage.cache_read_input_tokens,
        output: usage.output_tokens,
    }
}

/// [`line_usage`] by deserializing the line's fields that matter.
fn full_line_usage(line: &str) -> Option<(String, ModelUsage)> {
    let line: UsageLine = serde_json::from_str(line).ok()?;
    if line.kind.as_deref() != Some("assistant") {
        return None;
    }
    let message = line.message?;
    let usage = message.usage?;
    Some((
        message.id?,
        model_usage(message.model.unwrap_or_default(), usage),
    ))
}

/// Add the usage recorded in `file` to `usage`, once per response.
fn add_usage(file: &Path, seen: &mut HashSet<String>, usage: &mut Usage) {
    let Ok(lines) = crate::session::matching_lines(file, &["\"usage\""]) else {
        return;
    };
    for line in lines {
        if let Some((id, u)) = line_usage(&line)
            && seen.insert(id)
        {
            usage.add(&u);
        }
    }
}

/// Add the usage of the subagents a session started
/// (`<session>/subagents/*.jsonl`).
fn add_subagent_usage(file: &Path, seen: &mut HashSet<String>, usage: &mut Usage) {
    let subagents = file.with_extension("").join("subagents");
    if let Ok(entries) = std::fs::read_dir(subagents) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort();
        for f in files {
            add_usage(&f, seen, usage);
        }
    }
}

/// Parse a Claude Code transcript. The format is internal to Claude Code;
/// this reads `type == "user"` lines for the prompts, assistant usage for
/// the tokens, and tolerates anything else.
pub fn parse_transcript(file: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut started_at = None;
    let mut first = None;
    let mut count = 0;
    let mut seen = HashSet::new();
    let mut usage = Usage::default();
    let needles = &["\"type\":\"user\"", "\"usage\""];
    for line in crate::session::matching_lines(file, needles).ok()? {
        if line.contains("\"type\":\"assistant\"") {
            if let Some((id, u)) = line_usage(&line)
                && seen.insert(id)
            {
                usage.add(&u);
            }
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("user")
            || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(message) = v.get("message") else {
            continue;
        };
        let Some(prompt) = user_text(message) else {
            continue;
        };
        // Skip synthetic prompts Claude Code inserts itself.
        if prompt.starts_with('<') && prompt.contains("</") && count == 0 {
            continue;
        }
        count += 1;
        if first.is_none() {
            first = Some(prompt);
            id = v
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            cwd = v.get("cwd").and_then(Value::as_str).map(PathBuf::from);
            started_at = v
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);
        }
    }
    let first = first?;
    let id = id.or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
    let last_active_at = file_mtime_ms(file)?;
    Some(AgentSession {
        id: SessionId(id),
        agent: AgentId::Claude,
        cwd: cwd?,
        started_at: started_at.unwrap_or(last_active_at),
        last_active_at,
        snippet: snippet(&first, 120),
        message_count: count,
        file: file.to_path_buf(),
        usage: {
            add_subagent_usage(file, &mut seen, &mut usage);
            usage
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_transcript_and_builds_hook_settings() {
        let dir = std::env::temp_dir().join(format!("chda-claude-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc.jsonl");
        fs::write(&file, concat!(
            r#"{"type":"last-prompt","sessionId":"abc"}"#, "\n",
            r#"{"type":"user","isSidechain":false,"sessionId":"abc","cwd":"/tmp/proj","timestamp":"2026-09-09T12:42:15.371Z","message":{"role":"user","content":"다국어 지원은   아직\n반영 안 됐나?"}}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}"#, "\n",
            r#"{"type":"user","sessionId":"abc","cwd":"/tmp/proj","message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}}"#, "\n",
            r#"{"type":"user","sessionId":"abc","cwd":"/tmp/proj","message":{"role":"user","content":[{"type":"text","text":"second"}]}}"#, "\n",
            "not json\n",
        )).unwrap();
        let s = parse_transcript(&file).unwrap();
        assert_eq!(s.id, SessionId("abc".into()));
        assert_eq!(s.cwd, PathBuf::from("/tmp/proj"));
        assert_eq!(s.started_at, 1_788_957_735_371);
        assert_eq!(s.snippet, "다국어 지원은 아직 반영 안 됐나?");
        assert_eq!(s.message_count, 2);

        let settings = hooks_settings(Path::new("/opt/chda bin/chda"));
        let stop = &settings["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(stop["command"], "'/opt/chda bin/chda' hook claude");
        assert_eq!(stop["timeout"], 10);
        assert!(settings["hooks"].get("PermissionRequest").is_some());

        let report = ClaudeAdapter
            .install_hooks(&dir, Path::new("/usr/local/bin/chda"))
            .unwrap();
        let written = fs::read_to_string(report.file.unwrap()).unwrap();
        assert!(written.contains("/usr/local/bin/chda hook claude"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn usage_counts_each_response_once_and_includes_subagents() {
        let dir = std::env::temp_dir().join(format!("chda-claude-usage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("s1/subagents")).unwrap();
        let file = dir.join("s1.jsonl");
        // One response written over two lines with the same id and usage.
        let a1 = r#"{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":10,"cache_creation_input_tokens":1000,"cache_read_input_tokens":20000,"output_tokens":300,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":1000}},"content":[]}}"#;
        fs::write(&file, [
            r#"{"type":"user","sessionId":"s1","cwd":"/w","message":{"role":"user","content":"go"}}"#,
            a1,
            a1,
            r#"{"type":"assistant","message":{"id":"msg_2","model":"claude-opus-5-5","usage":{"input_tokens":5,"cache_creation_input_tokens":0,"cache_read_input_tokens":21000,"output_tokens":100},"content":[]}}"#,
            r#"{"type":"assistant","message":{"id":"msg_3","model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0},"content":[]}}"#,
        ].join("\n")).unwrap();
        fs::write(dir.join("s1/subagents/agent-a.jsonl"),
            r#"{"type":"assistant","message":{"id":"msg_9","model":"claude-haiku-4-5","usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":500},"content":[]}}"#,
        ).unwrap();
        let s = parse_transcript(&file).unwrap();
        let opus = s
            .usage
            .models
            .iter()
            .find(|m| m.model == "claude-opus-5-5")
            .unwrap();
        assert_eq!(
            (
                opus.input,
                opus.cache_write_5m,
                opus.cache_write_1h,
                opus.cache_read,
                opus.output
            ),
            (15, 0, 1000, 41_000, 400)
        );
        assert_eq!(
            s.usage.models.len(),
            2,
            "haiku from the subagent, no synthetic entry"
        );
        // By hand: Opus 5.5 at $4 in, $8 1h writes, $0.20 reads, $20 out;
        // Haiku 4.5 at $1 in, $5 out.
        let expected = (15.0 * 4.0
            + 1000.0 * 8.0
            + 41_000.0 * 0.2
            + 400.0 * 20.0
            + 2000.0 * 1.0
            + 500.0 * 5.0)
            / 1_000_000.0;
        let (dollars, complete) = s.usage.cost();
        assert!(complete);
        assert!(
            (dollars - expected).abs() < 1e-12,
            "{dollars} vs {expected}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quick_and_full_usage_parses_agree() {
        // Content mentions model and usage keys inside strings (escaped) and a
        // tool input with its own "model" key, after the message's model.
        let line = r#"{"parentUuid":"p","type":"assistant","message":{"model":"claude-sonnet-5-5","id":"msg_01AB","type":"message","role":"assistant","content":[{"type":"text","text":"the \"usage\":{\"input_tokens\":999} and \"model\":\"x\""},{"type":"tool_use","id":"toolu_1","input":{"model":"gpt-5"}}],"usage":{"input_tokens":3,"cache_creation_input_tokens":40,"cache_read_input_tokens":500,"output_tokens":60,"cache_creation":{"ephemeral_5m_input_tokens":40,"ephemeral_1h_input_tokens":0}}},"requestId":"r","uuid":"u"}"#;
        let quick = quick_line_usage(line).unwrap();
        assert_eq!(quick, full_line_usage(line).unwrap());
        assert_eq!(quick.0, "msg_01AB");
        assert_eq!(
            (
                quick.1.model.as_str(),
                quick.1.input,
                quick.1.cache_write_5m,
                quick.1.cache_read,
                quick.1.output
            ),
            ("claude-sonnet-5-5", 3, 40, 500, 60)
        );
        // Without a message id neither parse counts the line.
        assert_eq!(
            line_usage(r#"{"type":"assistant","message":{"usage":{}}}"#),
            None
        );
    }

    #[test]
    fn project_folders_follow_claude_code_naming() {
        assert_eq!(
            project_dir_name(Path::new("/Users/me/.agent")),
            "-Users-me--agent"
        );
        assert_eq!(
            project_dir_name(Path::new("/src/app.worktrees/feat_x")),
            "-src-app-worktrees-feat-x"
        );
    }
}

#[cfg(test)]
mod executable_tests {
    use super::*;
    #[test]
    fn path_install_wins_and_native_install_is_validated() {
        let home =
            std::env::temp_dir().join(format!("chda-claude-executable-{}", std::process::id()));
        let path = home.join("shell/bin");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let adapter = ClaudeAdapter;
        assert!(
            adapter
                .executable(&home, std::ffi::OsStr::new(""))
                .is_none()
        );
        let native = home.join(".local/bin/claude");
        std::fs::write(&native, "not executable").unwrap();
        assert!(
            adapter
                .executable(&home, std::ffi::OsStr::new(""))
                .is_none()
        );
        std::fs::copy(std::env::current_exe().unwrap(), &native).unwrap();
        assert_eq!(
            adapter.executable(&home, std::ffi::OsStr::new("")).unwrap(),
            native.canonicalize().unwrap()
        );
        let shell = path.join("claude");
        std::fs::copy(std::env::current_exe().unwrap(), &shell).unwrap();
        assert_eq!(
            adapter.executable(&home, path.as_os_str()).unwrap(),
            shell.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
