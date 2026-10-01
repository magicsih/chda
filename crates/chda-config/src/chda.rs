//! chda's own configuration: `~/.config/chda/config.toml`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default worktree path template (ADR-0003).
pub const DEFAULT_WORKTREE_TEMPLATE: &str = "{repo_parent}/{repo_name}.worktrees/{branch}";

/// What to run in the terminal opened for a new worktree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultAction {
    /// A plain shell.
    #[default]
    Terminal,
    Claude,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ChdaConfig {
    /// Registered repositories (main worktree paths).
    pub repos: Vec<PathBuf>,
    /// Template with `{repo_parent}`, `{repo_name}` and `{branch}`.
    pub worktree_path_template: String,
    pub default_action: DefaultAction,
    /// Agents shown in the sidebar, by id (`claude`, `codex`).
    pub agents: Vec<String>,
    pub sidebar_width: u32,
    pub sidebar_visible: bool,
    pub notifications: bool,
}

impl Default for ChdaConfig {
    fn default() -> Self {
        Self {
            repos: Vec::new(),
            worktree_path_template: DEFAULT_WORKTREE_TEMPLATE.into(),
            default_action: DefaultAction::Terminal,
            agents: vec!["claude".into(), "codex".into()],
            sidebar_width: 280,
            sidebar_visible: true,
            notifications: true,
        }
    }
}

impl ChdaConfig {
    /// `$XDG_CONFIG_HOME/chda/config.toml`, default `~/.config/chda/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("chda").join("config.toml"))
    }

    /// Load from `path`; a missing file yields the defaults, a broken one an error.
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, text)
    }

    /// Resolve the worktree path for `branch` of the repository at `repo`.
    /// Slashes in the branch become dashes so the path stays flat.
    pub fn worktree_path(&self, repo: &Path, branch: &str) -> PathBuf {
        let parent = repo.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let branch = branch.replace('/', "-");
        PathBuf::from(
            self.worktree_path_template
                .replace("{repo_parent}", &parent.to_string_lossy())
                .replace("{repo_name}", &name)
                .replace("{branch}", &branch),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_resolves_worktree_paths() {
        let dir = std::env::temp_dir().join(format!("chda-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = ChdaConfig::load(&path).unwrap();
        assert_eq!(c, ChdaConfig::default());
        c.repos.push("/src/app".into());
        c.default_action = DefaultAction::Claude;
        c.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("default-action = \"claude\""), "{text}");
        assert_eq!(ChdaConfig::load(&path).unwrap(), c);
        assert_eq!(
            c.worktree_path(Path::new("/src/app"), "feat/x"),
            PathBuf::from("/src/app.worktrees/feat-x")
        );
        fs::write(&path, "sidebar-width = 320\nunknown = 1\n").unwrap();
        let c = ChdaConfig::load(&path).unwrap();
        assert_eq!(c.sidebar_width, 320);
        assert_eq!(c.agents, vec!["claude", "codex"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
