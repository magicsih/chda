//! chda's own configuration: `~/.config/chda/config.toml`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default worktree path template (ADR-0003).
pub const DEFAULT_WORKTREE_TEMPLATE: &str = "{repo_parent}/{repo_name}.worktrees/{branch}";

/// What a tab is called when the user has not renamed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabTitle {
    /// The worktree's branch; falls back to the directory name.
    #[default]
    Branch,
    /// The last component of the working directory.
    Path,
}

/// Labels used by the ACTIVE sidebar list, independent of tab titles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActiveLabel {
    /// First line of the branch note, falling back to the branch name.
    #[default]
    Alias,
    Branch,
}

/// What to run in the terminal opened for a new worktree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultAction {
    /// A plain shell.
    #[default]
    Terminal,
    Claude,
    Codex,
    Gemini,
    Copilot,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl DefaultAction {
    /// The agent this action runs, by id; `None` for a plain shell.
    pub fn agent(self) -> Option<&'static str> {
        match self {
            DefaultAction::Terminal => None,
            DefaultAction::Claude => Some("claude"),
            DefaultAction::Codex => Some("codex"),
            DefaultAction::Gemini => Some("gemini"),
            DefaultAction::Copilot => Some("copilot"),
            DefaultAction::OpenCode => Some("opencode"),
        }
    }
}

/// How "Update branch" brings a worktree's upstream in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PullStrategy {
    /// Fast-forward only; a diverged branch stops and offers a choice.
    #[default]
    FfOnly,
    /// Replay local commits on the upstream, unless any of them is already
    /// pushed (then it stops and offers a merge).
    Rebase,
    /// Merge commit when the branch has diverged.
    Merge,
}

/// A named way to start an agent, e.g. Claude Code with `--model opus`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AgentPreset {
    /// Shown in menus: "Run <name>".
    pub name: String,
    /// Agent id, e.g. `claude` or `codex`.
    pub agent: String,
    /// Arguments added after the ones chda passes for its hooks.
    #[serde(default)]
    pub args: Vec<String>,
}

/// A branch pinned to the sidebar's STARRED list. The repository's main
/// worktree path keeps equal branch names in different repositories apart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StarredBranch {
    pub repo: PathBuf,
    pub branch: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ChdaConfig {
    /// Opt-in preparation for new worktrees; approval is stored separately in private app data.
    pub repo_preparation: BTreeMap<PathBuf, PreparationPlan>,
    /// Registered repositories (main worktree paths).
    pub repos: Vec<PathBuf>,
    /// Template with `{repo_parent}`, `{repo_name}` and `{branch}`.
    pub worktree_path_template: String,
    pub default_action: DefaultAction,
    pub tab_title: TabTitle,
    pub active_label: ActiveLabel,
    /// Hide ACTIVE rows while keeping the header and count visible.
    pub active_collapsed: bool,
    /// Hide live idle rows while keeping the header and count visible.
    pub idle_agents_collapsed: bool,
    /// Hide registered folders and starred branches beneath PROJECT.
    pub project_collapsed: bool,
    /// Agents offered in the sidebar and the palette, by id (`claude`,
    /// `codex`, `gemini`, `copilot`, `opencode`).
    pub agents: Vec<String>,
    pub sidebar_width: u32,
    pub sidebar_visible: bool,
    pub notifications: bool,
    /// Command that opens a cmd-clicked file path, with `{file}`, `{line}`
    /// and `{column}` placeholders, e.g. `zed {file}:{line}:{column}`.
    /// Unset opens the file with the system's default application.
    pub editor: Option<String>,
    /// Reopen the last window's tabs, splits and directories on launch.
    pub restore_session: bool,
    /// When restoring the last session, reopen the agent conversation a
    /// pane was running instead of a plain shell.
    pub restore_agents: bool,
    /// Theme name; replaces the Ghostty config's `theme` when set.
    pub theme: Option<String>,
    /// Pull request host per repository path, for remotes whose host chda
    /// cannot work out, e.g. `"/src/app" = "github.example.com"`.
    pub repo_hosts: BTreeMap<PathBuf, String>,
    /// What "Update branch" does when the branch has diverged.
    pub pull: PullStrategy,
    /// Named agent launches for the sidebar menu and the palette.
    pub agent_presets: Vec<AgentPreset>,
    /// The app the title bar's open button uses, e.g. `vscode`; picked in
    /// the title bar and saved here.
    pub open_in: Option<String>,
    /// Ask GitHub once a day whether a newer release is out.
    pub update_check: bool,
    /// Branches in the sidebar's STARRED list, in the order they were added.
    pub starred: Vec<StarredBranch>,
}

impl Default for ChdaConfig {
    fn default() -> Self {
        Self {
            repo_preparation: BTreeMap::new(),
            repos: Vec::new(),
            worktree_path_template: DEFAULT_WORKTREE_TEMPLATE.into(),
            default_action: DefaultAction::Terminal,
            tab_title: TabTitle::Branch,
            active_label: ActiveLabel::Alias,
            active_collapsed: false,
            idle_agents_collapsed: false,
            project_collapsed: false,
            agents: vec!["claude".into(), "codex".into()],
            sidebar_width: 280,
            sidebar_visible: true,
            notifications: true,
            editor: None,
            restore_session: true,
            restore_agents: true,
            theme: None,
            repo_hosts: BTreeMap::new(),
            pull: PullStrategy::FfOnly,
            agent_presets: Vec::new(),
            open_in: None,
            update_check: true,
            starred: Vec::new(),
        }
    }
}

/// Explicit commands in order and regular gitignored files relative to the primary checkout.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PreparationPlan {
    pub commands: Vec<String>,
    pub files: Vec<PathBuf>,
}
impl PreparationPlan {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.files.is_empty()
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

    /// Add a branch to STARRED; false when it is already there.
    pub fn star(&mut self, repo: &Path, branch: &str) -> bool {
        if self.is_starred(repo, branch) {
            return false;
        }
        self.starred.push(StarredBranch {
            repo: repo.to_path_buf(),
            branch: branch.to_owned(),
        });
        true
    }

    /// Take a branch out of STARRED; false when it was not there.
    pub fn unstar(&mut self, repo: &Path, branch: &str) -> bool {
        let before = self.starred.len();
        self.starred
            .retain(|s| !(s.repo == repo && s.branch == branch));
        self.starred.len() != before
    }

    pub fn is_starred(&self, repo: &Path, branch: &str) -> bool {
        self.starred
            .iter()
            .any(|s| s.repo == repo && s.branch == branch)
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

/// Fill an editor command template. The template is split on blanks first,
/// so a path with spaces stays one argument. Missing line or column numbers
/// default to 1.
pub fn editor_command(
    template: &str,
    file: &Path,
    line: Option<u32>,
    column: Option<u32>,
) -> Vec<String> {
    let file = file.to_string_lossy();
    let line = line.unwrap_or(1).to_string();
    let column = column.unwrap_or(1).to_string();
    template
        .split_whitespace()
        .map(|t| {
            t.replace("{file}", &file)
                .replace("{line}", &line)
                .replace("{column}", &column)
        })
        .collect()
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
        c.active_label = ActiveLabel::Branch;
        c.active_collapsed = true;
        c.idle_agents_collapsed = true;
        c.project_collapsed = true;
        c.repo_preparation.insert(
            "/src/app".into(),
            PreparationPlan {
                commands: vec!["printf 'two words'\ntrue".into(), "npm ci".into()],
                files: vec!["local/한글 space.env".into(), "local/new\nline.env".into()],
            },
        );
        c.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("default-action = \"claude\""), "{text}");
        assert_eq!(ChdaConfig::load(&path).unwrap(), c);
        assert_eq!(
            c.worktree_path(Path::new("/src/app"), "feat/x"),
            PathBuf::from("/src/app.worktrees/feat-x")
        );
        fs::write(
            &path,
            "sidebar-width = 320\nunknown = 1\n[repo-hosts]\n\"/src/app\" = \"github.example.com\"\n",
        )
        .unwrap();
        let c = ChdaConfig::load(&path).unwrap();
        assert_eq!(c.sidebar_width, 320);
        assert_eq!(c.active_label, ActiveLabel::Alias);
        assert!(!c.active_collapsed);
        assert!(!c.idle_agents_collapsed);
        assert!(!c.project_collapsed);
        assert_eq!(
            c.repo_hosts.get(Path::new("/src/app")).map(String::as_str),
            Some("github.example.com")
        );
        assert_eq!(c.agents, vec!["claude", "codex"]);
        assert_eq!(c.pull, PullStrategy::FfOnly);
        fs::write(&path, "pull = \"rebase\"\n").unwrap();
        assert_eq!(ChdaConfig::load(&path).unwrap().pull, PullStrategy::Rebase);
        fs::write(&path, "pull = \"ff-only\"\n").unwrap();
        assert_eq!(ChdaConfig::load(&path).unwrap().pull, PullStrategy::FfOnly);
        fs::write(&path, "default-action = \"opencode\"\n").unwrap();
        let c = ChdaConfig::load(&path).unwrap();
        assert_eq!(c.default_action.agent(), Some("opencode"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn starred_branches_are_unique_per_repository_and_persist() {
        let dir = std::env::temp_dir().join(format!("chda-starred-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = ChdaConfig::default();
        assert!(c.star(Path::new("/src/a"), "main"));
        assert!(!c.star(Path::new("/src/a"), "main"), "no duplicates");
        assert!(c.star(Path::new("/src/b"), "main"), "same name, other repo");
        assert!(c.star(Path::new("/src/a"), "feat/x"));
        c.repos.push("/src/a".into());
        c.save(&path).unwrap();
        let loaded = ChdaConfig::load(&path).unwrap();
        assert_eq!(loaded, c);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[starred]]"), "{text}");
        let mut c = loaded;
        assert!(c.unstar(Path::new("/src/a"), "main"));
        assert!(!c.unstar(Path::new("/src/a"), "main"));
        assert!(c.is_starred(Path::new("/src/b"), "main"));
        assert_eq!(
            c.starred
                .iter()
                .map(|s| s.branch.as_str())
                .collect::<Vec<_>>(),
            ["main", "feat/x"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn agent_presets_round_trip() {
        let dir = std::env::temp_dir().join(format!("chda-presets-{}", std::process::id()));
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            concat!(
                "theme = \"x\"\n",
                "[[agent-presets]]\nname = \"Opus\"\nagent = \"claude\"\nargs = [\"--model\", \"opus\"]\n",
                "[[agent-presets]]\nname = \"Codex auto\"\nagent = \"codex\"\n",
            ),
        )
        .unwrap();
        let c = ChdaConfig::load(&path).unwrap();
        assert_eq!(
            c.agent_presets,
            vec![
                AgentPreset {
                    name: "Opus".into(),
                    agent: "claude".into(),
                    args: vec!["--model".into(), "opus".into()],
                },
                AgentPreset {
                    name: "Codex auto".into(),
                    agent: "codex".into(),
                    args: Vec::new(),
                },
            ]
        );
        c.save(&path).unwrap();
        assert_eq!(ChdaConfig::load(&path).unwrap(), c);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn editor_templates_keep_paths_whole() {
        assert_eq!(
            editor_command(
                "code --goto {file}:{line}:{column}",
                Path::new("/a b/c.rs"),
                Some(3),
                None
            ),
            vec!["code", "--goto", "/a b/c.rs:3:1"]
        );
    }
}
