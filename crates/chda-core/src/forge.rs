//! Pull request badges from the forge a repository lives on: GitHub through
//! `gh`, GitLab through `glab`, Gitea and Forgejo through `tea`. chda never
//! reads a token; it only runs the CLIs and parses their JSON.
//!
//! The host comes from each repository's remote, so self-hosted servers and
//! SSH host aliases work, and the login is checked per host, so one expired
//! token only affects that host's repositories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::sidebar::PrInfo;
use crate::{gitea, github, gitlab};

/// A repository on a forge: `host` and `owner/name` path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemoteRepo {
    pub host: String,
    pub path: String,
}

impl RemoteRepo {
    /// `host/owner/name`, the form `gh --repo` accepts.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.host, self.path)
    }
}

/// Host and repository path of a remote URL. `ssh` is true for SSH URLs,
/// whose host may be an alias from `~/.ssh/config`.
fn parse_remote_url(url: &str) -> Option<(String, bool, String)> {
    let (host, ssh, path) = if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        // Drop a port: gh and its config name hosts without one.
        let host = host.split(':').next()?;
        (host, scheme.contains("ssh"), path)
    } else {
        // scp-like `user@host:owner/name.git`.
        let (authority, path) = url.split_once(':')?;
        if authority.contains('/') {
            return None;
        }
        (authority.rsplit('@').next()?, true, path)
    };
    let path = path.trim_matches('/').trim_end_matches(".git");
    if host.is_empty() || !path.contains('/') {
        return None;
    }
    Some((host.to_ascii_lowercase(), ssh, path.to_owned()))
}

/// The real host name for an SSH host, from `ssh -G` output.
fn parse_ssh_config(output: &str) -> Option<String> {
    let host = output
        .lines()
        .find_map(|l| l.strip_prefix("hostname "))?
        .trim()
        .to_ascii_lowercase();
    // GitHub's SSH-over-443 endpoint is still github.com for gh.
    Some(match host.as_str() {
        "ssh.github.com" => "github.com".into(),
        _ => host,
    })
}

/// Resolve an SSH alias with `ssh -G`, once per alias.
fn resolve_ssh_host(alias: &str) -> String {
    static CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    if let Some(host) = CACHE.lock().unwrap().get_or_insert_default().get(alias) {
        return host.clone();
    }
    let host = Command::new("ssh")
        .args(["-G", alias])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| parse_ssh_config(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_else(|| alias.to_owned());
    CACHE
        .lock()
        .unwrap()
        .get_or_insert_default()
        .insert(alias.to_owned(), host.clone());
    host
}

/// The remote `gh` would use: the one `gh repo set-default` picked, then
/// `upstream`, `github`, `origin`, then any.
fn pick_remote(remotes: &[chda_git::RemoteInfo]) -> Option<&chda_git::RemoteInfo> {
    remotes
        .iter()
        .find(|r| r.gh_resolved.is_some())
        .or_else(|| {
            ["upstream", "github", "origin"]
                .iter()
                .find_map(|n| remotes.iter().find(|r| r.name == *n))
        })
        .or_else(|| remotes.first())
}

/// The forge repository of the repository at `repo`. `host_override` (from
/// `repo-hosts` in config.toml) replaces the host taken from the remote.
/// Blocking: may run `ssh -G` once per alias.
pub fn repo_remote(repo: &Path, host_override: Option<&str>) -> Option<RemoteRepo> {
    let remotes = chda_git::remotes(repo).ok()?;
    let (host, ssh, path) = parse_remote_url(&pick_remote(&remotes)?.url)?;
    let host = match host_override {
        Some(h) => h.to_owned(),
        None if ssh => resolve_ssh_host(&host),
        None => host,
    };
    Some(RemoteRepo { host, path })
}

/// Which kind of forge a host runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    /// Gitea and Forgejo (Codeberg).
    Gitea,
}

impl Forge {
    /// The forge of a well-known public host.
    fn of_known_host(host: &str) -> Option<Self> {
        match host {
            "github.com" => Some(Forge::GitHub),
            "gitlab.com" => Some(Forge::GitLab),
            "gitea.com" | "codeberg.org" => Some(Forge::Gitea),
            _ => None,
        }
    }

    /// The command that logs the forge's CLI in to `host`.
    fn login_command(self, host: &str) -> String {
        match self {
            Forge::GitHub => format!("gh auth login --hostname {host}"),
            Forge::GitLab => format!("glab auth login --hostname {host}"),
            Forge::Gitea => format!("tea login add --url https://{host}"),
        }
    }
}

/// The hint shown on a repository whose host none of the CLIs that could
/// serve it is logged in to.
fn hint(commands: &[String]) -> String {
    let commands: Vec<String> = commands.iter().map(|c| format!("`{c}`")).collect();
    format!(
        "Log in with {} for pull request badges",
        commands.join(" or ")
    )
}

/// The hint for one forge's CLI.
pub fn login_hint(forge: Forge, host: &str) -> String {
    hint(&[forge.login_command(host)])
}

/// Where the forge CLIs are; the defaults look them up on `PATH`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeClis {
    pub gh: PathBuf,
    pub glab: PathBuf,
    pub tea: PathBuf,
}

impl Default for ForgeClis {
    fn default() -> Self {
        Self {
            gh: "gh".into(),
            glab: "glab".into(),
            tea: "tea".into(),
        }
    }
}

/// One CLI binary: whether it runs, and its login per host, both cached.
/// Logins are re-checked every few minutes so a fresh login is picked up
/// without a restart.
struct Cli {
    bin: PathBuf,
    installed: OnceLock<bool>,
    /// Host to the login to use (`tea` names its logins), or `None`.
    logins: Mutex<HashMap<String, (Option<String>, Instant)>>,
}

impl Cli {
    const LOGIN_TTL: Duration = Duration::from_secs(300);

    fn new(bin: PathBuf) -> Self {
        Self {
            bin,
            installed: OnceLock::new(),
            logins: Mutex::default(),
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.stdin(Stdio::null()).env("NO_COLOR", "1");
        cmd
    }

    fn installed(&self) -> bool {
        *self.installed.get_or_init(|| {
            self.command()
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    /// Whether `args` exits successfully, output discarded.
    fn succeeds(&self, args: &[&str]) -> bool {
        self.command()
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// The cached login for `host`, or `check`'s answer.
    fn login(&self, host: &str, check: impl FnOnce(&Self) -> Option<String>) -> Option<String> {
        if !self.installed() {
            return None;
        }
        if let Some((login, at)) = self.logins.lock().unwrap().get(host)
            && at.elapsed() < Self::LOGIN_TTL
        {
            return login.clone();
        }
        let login = check(self);
        self.logins
            .lock()
            .unwrap()
            .insert(host.to_owned(), (login.clone(), Instant::now()));
        login
    }

    /// Stdout of `cmd` when it succeeds.
    fn output(mut cmd: Command) -> Option<String> {
        let out = cmd.stderr(Stdio::null()).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// The forge CLIs, asked per repository for its pull requests.
pub struct Forges {
    gh: Cli,
    glab: Cli,
    tea: Cli,
}

impl Forges {
    pub fn new(clis: ForgeClis) -> Self {
        Self {
            gh: Cli::new(clis.gh),
            glab: Cli::new(clis.glab),
            tea: Cli::new(clis.tea),
        }
    }

    fn cli(&self, forge: Forge) -> &Cli {
        match forge {
            Forge::GitHub => &self.gh,
            Forge::GitLab => &self.glab,
            Forge::Gitea => &self.tea,
        }
    }

    /// Whether any forge CLI is installed. Blocking.
    pub fn any_installed(&self) -> bool {
        [Forge::GitHub, Forge::GitLab, Forge::Gitea]
            .into_iter()
            .any(|f| self.cli(f).installed())
    }

    /// The login `forge`'s CLI has for `host`, if any. Blocking.
    fn login(&self, forge: Forge, host: &str) -> Option<String> {
        self.cli(forge).login(host, |cli| match forge {
            Forge::GitHub => cli
                .succeeds(&["auth", "status", "--hostname", host])
                .then(|| host.to_owned()),
            Forge::GitLab => cli
                .succeeds(&["auth", "status", "--hostname", host])
                .then(|| host.to_owned()),
            Forge::Gitea => {
                let mut cmd = cli.command();
                cmd.args(["logins", "list", "--output", "json"]);
                gitea::login_for_host(&Cli::output(cmd)?, host)
            }
        })
    }

    /// Pull request state for each `(worktree, branch)` of `remote`.
    /// `Err(Some(hint))` when no CLI that could serve the host is logged in
    /// to it, `Err(None)` when none is even installed. Blocking.
    pub fn pull_requests(
        &self,
        remote: &RemoteRepo,
        branches: &[(PathBuf, String)],
    ) -> Result<Vec<(PathBuf, Option<PrInfo>)>, Option<String>> {
        let candidates = match Forge::of_known_host(&remote.host) {
            Some(forge) => vec![forge],
            None => vec![Forge::GitHub, Forge::GitLab, Forge::Gitea],
        };
        for &forge in &candidates {
            if let Some(login) = self.login(forge, &remote.host) {
                return Ok(self.fetch(forge, &login, remote, branches));
            }
        }
        let commands: Vec<String> = candidates
            .into_iter()
            .filter(|f| self.cli(*f).installed())
            .map(|f| f.login_command(&remote.host))
            .collect();
        Err((!commands.is_empty()).then(|| hint(&commands)))
    }

    fn fetch(
        &self,
        forge: Forge,
        login: &str,
        remote: &RemoteRepo,
        branches: &[(PathBuf, String)],
    ) -> Vec<(PathBuf, Option<PrInfo>)> {
        match forge {
            Forge::GitHub => branches
                .iter()
                .map(|(path, branch)| {
                    let mut cmd = self.gh.command();
                    cmd.args(github::pr_list_args(remote, branch));
                    let pr = Cli::output(cmd).and_then(|out| github::parse_pr_list(&out));
                    (path.clone(), pr)
                })
                .collect(),
            Forge::GitLab => branches
                .iter()
                .map(|(path, branch)| {
                    let gitlab_command = |args: Vec<String>| {
                        let mut cmd = self.glab.command();
                        cmd.env("GITLAB_HOST", &remote.host)
                            .env("NO_PROMPT", "1")
                            .args(args);
                        Cli::output(cmd)
                    };
                    let mr = gitlab_command(gitlab::mr_list_args(remote, branch))
                        .and_then(|out| gitlab::parse_mr_list(&out))
                        .map(|mut mr| {
                            if mr.state == crate::PrState::Open
                                && let Some(out) =
                                    gitlab_command(gitlab::mr_view_args(remote, mr.number))
                            {
                                mr.checks = gitlab::parse_pipeline(&out);
                            }
                            mr
                        });
                    (path.clone(), mr)
                })
                .collect(),
            Forge::Gitea => {
                let mut cmd = self.tea.command();
                cmd.args(gitea::pr_list_args(login, remote));
                let out = Cli::output(cmd).unwrap_or_default();
                branches
                    .iter()
                    .map(|(path, branch)| (path.clone(), gitea::pr_for_branch(&out, branch)))
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_urls() {
        let p = |u| parse_remote_url(u);
        let r = |h: &str, ssh, path: &str| Some((h.to_owned(), ssh, path.to_owned()));
        assert_eq!(
            p("https://github.com/org/repo.git"),
            r("github.com", false, "org/repo")
        );
        assert_eq!(
            p("https://user@GHE.example.com:8443/org/repo/"),
            r("ghe.example.com", false, "org/repo")
        );
        assert_eq!(
            p("git@github-work:org/repo.git"),
            r("github-work", true, "org/repo")
        );
        assert_eq!(
            p("ssh://git@github.example.com:2222/org/repo.git"),
            r("github.example.com", true, "org/repo")
        );
        assert_eq!(
            p("https://gitlab.com/group/sub/proj.git"),
            r("gitlab.com", false, "group/sub/proj")
        );
        assert_eq!(p("/local/path/repo.git"), None);
        assert_eq!(p("../relative"), None);
    }

    #[test]
    fn reads_the_real_host_of_an_ssh_alias() {
        let out = "user git\nhostname github.example.com\nport 22\n";
        assert_eq!(parse_ssh_config(out).as_deref(), Some("github.example.com"));
        assert_eq!(
            parse_ssh_config("hostname ssh.github.com\nport 443\n").as_deref(),
            Some("github.com")
        );
        assert_eq!(parse_ssh_config("port 22\n"), None);
    }

    #[test]
    fn picks_the_remote_gh_would_use() {
        let r = |name: &str, resolved: Option<&str>| chda_git::RemoteInfo {
            name: name.into(),
            url: format!("https://github.com/{name}/repo"),
            gh_resolved: resolved.map(Into::into),
        };
        let pick = |list: &[chda_git::RemoteInfo]| pick_remote(list).map(|r| r.name.clone());
        assert_eq!(
            pick(&[r("fork", None), r("origin", None)]).as_deref(),
            Some("origin")
        );
        assert_eq!(
            pick(&[r("origin", None), r("upstream", None)]).as_deref(),
            Some("upstream")
        );
        assert_eq!(
            pick(&[r("origin", Some("base")), r("upstream", None)]).as_deref(),
            Some("origin")
        );
        assert_eq!(pick(&[r("mine", None)]).as_deref(), Some("mine"));
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn repo_remote_resolves_an_override_and_reads_the_remote() {
        let dir = std::env::temp_dir().join(format!("chda-gh-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&dir)
                    .status()
                    .unwrap()
                    .success()
            )
        };
        git(&["init", "-q"]);
        assert_eq!(repo_remote(&dir, None), None);
        git(&["remote", "add", "origin", "https://github.com/org/repo.git"]);
        let remote = repo_remote(&dir, None).unwrap();
        assert_eq!(remote.slug(), "github.com/org/repo");
        assert_eq!(
            repo_remote(&dir, Some("github.example.com"))
                .unwrap()
                .slug(),
            "github.example.com/org/repo"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hints_name_the_commands_that_could_log_in() {
        assert_eq!(
            login_hint(Forge::GitHub, "ghe.example.com"),
            "Log in with `gh auth login --hostname ghe.example.com` for pull request badges"
        );
        assert_eq!(
            hint(&[
                Forge::GitLab.login_command("git.example.com"),
                Forge::Gitea.login_command("git.example.com")
            ]),
            "Log in with `glab auth login --hostname git.example.com` or \
             `tea login add --url https://git.example.com` for pull request badges"
        );
        assert_eq!(Forge::of_known_host("codeberg.org"), Some(Forge::Gitea));
        assert_eq!(Forge::of_known_host("git.example.com"), None);
    }
}
