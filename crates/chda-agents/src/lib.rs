//! `AgentAdapter` trait, Claude Code and Codex adapters, the hook receiver
//! and the session indexer.
//!
//! Platform-specific IPC code lives under `ipc`.

mod claude;
mod codex;
pub mod hook;
pub mod ipc;
mod session;

use std::path::{Path, PathBuf};
use std::process::Command;

pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use hook::{HookEvent, HookKind, PANE_ENV, hook_main};
pub use session::{AgentSession, SessionCache, SessionId};

/// Which agent a thing belongs to.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    Claude,
    Codex,
}

impl AgentId {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::Claude => "claude",
            AgentId::Codex => "codex",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(AgentId::Claude),
            "codex" => Some(AgentId::Codex),
            _ => None,
        }
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
    /// Whether the agent's binary is on `PATH`.
    fn is_installed(&self) -> bool;
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
    /// Parse one transcript file; `None` when it is not a session.
    fn parse_session(&self, file: &Path) -> Option<AgentSession>;
    /// Write hook configuration under `data_dir` so the agent reports to
    /// `hook_bin`. Never touches the user's own config files.
    fn install_hooks(&self, data_dir: &Path, hook_bin: &Path)
    -> std::io::Result<HookInstallReport>;
}

/// The adapters chda ships.
pub fn adapters() -> Vec<Box<dyn AgentAdapter>> {
    vec![Box::new(ClaudeAdapter), Box::new(CodexAdapter)]
}

/// Find an executable on `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// Index the sessions that ran inside `worktrees`, newest first.
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
    out.sort_by_key(|s| std::cmp::Reverse(s.started_at));
    out
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
        let chda: Vec<_> = sessions
            .iter()
            .filter(|s| s.cwd.ends_with("magicsih/chda"))
            .collect();
        eprintln!("chda sessions: {}", chda.len());
    }
}
