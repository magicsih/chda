//! `AgentAdapter` trait, the adapters for Claude Code, Codex, Gemini CLI,
//! GitHub Copilot CLI and OpenCode, the hook receiver and the session indexer.
//!
//! Platform-specific IPC code lives under `ipc`.

mod children;
mod claude;
mod codex;
pub mod control;
mod copilot;
mod gemini;
pub mod hook;
pub mod ipc;
mod launch;
mod mcp;
mod opencode;
mod pricing;
pub mod quota;
mod session;
mod sharing;
pub mod statusline;
mod usage;

use std::path::{Path, PathBuf};
use std::process::Command;

pub use children::{ChildActivity, ChildBoard, ChildEvent, ChildState, read_codex_children};
pub use claude::ClaudeAdapter;
pub use codex::{
    CODEX_TITLE_CONFIG, CODEX_TITLE_ENV, CodexAdapter, CodexRunState,
    notify_config as codex_notify_config, parse_codex_title,
};
pub use copilot::CopilotAdapter;
pub use gemini::GeminiAdapter;
pub use hook::{HookEvent, HookKind, PANE_ENV, hook_main};
pub use launch::{
    AgentLaunchContext, LaunchHistory, PermissionPolicy, permission_arguments,
    validate_resume_options,
};
pub use mcp::mcp_main;
pub use opencode::OpenCodeAdapter;
pub use session::{AgentSession, SessionCache, SessionId};
pub use sharing::{SharePreview, sharing_preview};
pub use usage::{LimitUsage, ModelUsage, Usage, compact_tokens};

/// Which agent a thing belongs to.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    Claude,
    Codex,
    Gemini,
    Copilot,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl AgentId {
    pub const ALL: [AgentId; 5] = [
        AgentId::Claude,
        AgentId::Codex,
        AgentId::Gemini,
        AgentId::Copilot,
        AgentId::OpenCode,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::Claude => "claude",
            AgentId::Codex => "codex",
            AgentId::Gemini => "gemini",
            AgentId::Copilot => "copilot",
            AgentId::OpenCode => "opencode",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == s)
    }
}

/// How hooks were (or were not) installed for an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookInstallReport {
    /// File chda wrote, if any.
    pub file: Option<PathBuf>,
    /// Why status tracking may be degraded, if it is.
    pub note: Option<String>,
}

/// Everything chda needs to know about one coding agent.
pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> AgentId;
    fn display_name(&self) -> &str;
    /// Two-letter label for compact lists, e.g. `CC` for Claude Code. The
    /// default is the display name's first two letters.
    fn short_label(&self) -> String {
        self.display_name()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }
    /// Whether the agent's binary is on the launch environment's `PATH`.
    fn is_installed(&self, path: &std::ffi::OsStr) -> bool;
    /// Validated executable for detection and every launch route.
    fn executable(&self, _home: &Path, path: &std::ffi::OsStr) -> Option<PathBuf> {
        self.is_installed(path)
            .then(|| which(self.id().as_str(), path))
            .flatten()
    }
    /// Command that runs the agent in `cwd`, optionally resuming a session.
    /// `hook_bin` is the chda executable the agent should call for hooks.
    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, hook_bin: &Path) -> Command;
    /// Directories holding session transcripts.
    fn session_roots(&self) -> Vec<PathBuf>;
    /// Directories to scan for sessions that ran inside `worktrees`. The
    /// default is every root; adapters that file transcripts by directory
    /// can skip the rest.
    fn session_dirs(&self, worktrees: &[PathBuf]) -> Vec<PathBuf> {
        let _ = worktrees;
        self.session_roots()
    }
    /// The directory a transcript's session ran in, read cheaply from the
    /// top of the file.
    fn session_cwd(&self, file: &Path) -> Option<PathBuf> {
        session::head_cwd(file)
    }
    /// Whether the agent still has the transcript of session `id`, so it can
    /// be resumed. Claude uses exact filenames; Codex additionally confirms
    /// the full session ID in rollout metadata. Other storage formats override it.
    fn has_session(&self, id: &SessionId) -> bool {
        !id.0.is_empty()
            && self.session_roots().iter().any(|root| {
                session::jsonl_files(root)
                    .iter()
                    .any(|f| session::transcript_matches_id(f, self.id(), id))
            })
    }
    /// Parse one transcript file; `None` when it is not a session.
    fn parse_session(&self, file: &Path) -> Option<AgentSession>;
    /// Write hook configuration under `data_dir` so the agent reports to
    /// `hook_bin`. Never touches the user's own config files.
    fn install_hooks(&self, data_dir: &Path, hook_bin: &Path)
    -> std::io::Result<HookInstallReport>;
}

/// The adapters chda ships.
pub fn adapters() -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(ClaudeAdapter),
        Box::new(CodexAdapter),
        Box::new(GeminiAdapter),
        Box::new(CopilotAdapter),
        Box::new(OpenCodeAdapter),
    ]
}

/// A launch command as the argument vector a pane runs. Environment
/// variables the adapter set go in front through `env`, since a pane takes
/// only a program and its arguments.
pub fn command_argv(cmd: &Command) -> Vec<String> {
    let lossy = |s: &std::ffi::OsStr| s.to_string_lossy().into_owned();
    let mut argv = Vec::new();
    let envs: Vec<String> = cmd
        .get_envs()
        .filter_map(|(k, v)| Some(format!("{}={}", lossy(k), lossy(v?))))
        .collect();
    if !envs.is_empty() {
        argv.push("env".to_owned());
        argv.extend(envs);
    }
    argv.push(lossy(cmd.get_program()));
    argv.extend(cmd.get_args().map(lossy));
    argv
}

/// Quote a path for a hook command line that a shell runs.
pub(crate) fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars()
        .all(|c| c.is_alphanumeric() || "/._-+".contains(c))
    {
        s.into_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Write `text` to `path` unless it already holds exactly that.
pub(crate) fn write_if_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// Find an executable on `PATH`.
pub fn which(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|p| crate::ipc::is_executable(p))
        .and_then(|p| p.canonicalize().ok())
}

/// Index the sessions that ran inside `worktrees`, most recently active
/// first.
pub fn index_sessions(
    adapters: &[Box<dyn AgentAdapter>],
    cache: &mut SessionCache,
    worktrees: &[PathBuf],
) -> Vec<AgentSession> {
    let wanted = |cwd: &Path| worktrees.iter().any(|w| cwd.starts_with(w));
    let mut out = Vec::new();
    for adapter in adapters {
        for root in adapter.session_dirs(worktrees) {
            for file in session::jsonl_files(&root) {
                let session = cache.session(
                    &file,
                    wanted,
                    || adapter.session_cwd(&file),
                    || adapter.parse_session(&file),
                );
                if let Some(s) = session {
                    out.push(s);
                }
            }
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.last_active_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_launch_env_goes_through_env() {
        for id in AgentId::ALL {
            assert_eq!(AgentId::parse(id.as_str()), Some(id));
            assert_eq!(
                serde_json::to_string(&id).unwrap(),
                format!("\"{}\"", id.as_str())
            );
        }
        assert_eq!(adapters().len(), AgentId::ALL.len());
        let mut cmd = Command::new("gemini");
        cmd.arg("--resume").arg("abc");
        assert_eq!(command_argv(&cmd), vec!["gemini", "--resume", "abc"]);
        cmd.env("GEMINI_CLI_SYSTEM_DEFAULTS_PATH", "/d/a b.json");
        assert_eq!(
            command_argv(&cmd),
            vec![
                "env",
                "GEMINI_CLI_SYSTEM_DEFAULTS_PATH=/d/a b.json",
                "gemini",
                "--resume",
                "abc"
            ]
        );
    }
}

#[cfg(test)]
mod real_sessions {
    #[test]
    #[ignore = "indexes the real ~/.claude and ~/.codex session files; run with CHDA_REAL=1"]
    fn index_real_sessions() {
        if std::env::var_os("CHDA_REAL").is_none() {
            return;
        }
        let adapters = super::adapters();
        // Directories from CHDA_REAL_WORKTREES (colon separated), else HOME.
        let worktrees: Vec<std::path::PathBuf> = std::env::var("CHDA_REAL_WORKTREES")
            .map(|v| v.split(':').map(std::path::PathBuf::from).collect())
            .unwrap_or_else(|_| vec![std::env::var("HOME").unwrap().into()]);
        let dir = std::env::temp_dir().join(format!("chda-real-index-{}", std::process::id()));
        let mut cache = super::SessionCache::new();
        let start = std::time::Instant::now();
        let sessions = super::index_sessions(&adapters, &mut cache, &worktrees);
        eprintln!("cold: {} sessions in {:?}", sessions.len(), start.elapsed());
        cache.save(&dir).unwrap();
        let mut warm = super::SessionCache::load(&dir);
        let start = std::time::Instant::now();
        let again = super::index_sessions(&adapters, &mut warm, &worktrees);
        eprintln!(
            "warm (after restart): {} sessions in {:?}",
            again.len(),
            start.elapsed()
        );
        let _ = std::fs::remove_dir_all(&dir);
        for s in sessions.iter().take(5) {
            eprintln!(
                "{:?} {} {} {:?} {}",
                s.agent, s.started_at, s.message_count, s.cwd, s.snippet
            );
        }
        for agent in super::AgentId::ALL {
            eprintln!(
                "{}: {}",
                agent.as_str(),
                sessions.iter().filter(|s| s.agent == agent).count()
            );
        }
        let chda: Vec<_> = sessions
            .iter()
            .filter(|s| s.cwd.ends_with("magicsih/chda"))
            .collect();
        eprintln!("chda sessions: {}", chda.len());
    }
}
