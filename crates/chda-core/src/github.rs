//! Pull request badges through the `gh` CLI. chda never reads the token;
//! it only runs `gh` and parses its JSON.
//!
//! The host comes from each repository's remote, so GitHub Enterprise
//! Server and SSH host aliases work, and the login is checked per host, so
//! one expired token only affects that host's repositories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::sidebar::{CheckState, PrInfo, PrState};

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

/// The `gh` CLI, with its login state cached per host and re-checked every
/// few minutes so a fresh `gh auth login` is picked up without a restart.
pub struct Gh {
    bin: PathBuf,
    logins: Mutex<HashMap<String, (bool, Instant)>>,
}

impl Gh {
    const LOGIN_TTL: Duration = Duration::from_secs(300);

    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            logins: Mutex::default(),
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.stdin(Stdio::null());
        cmd
    }

    /// Whether the binary runs at all. Blocking.
    pub fn installed(&self) -> bool {
        self.command()
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Whether `gh` is logged in to `host`. Blocking on a cache miss.
    pub fn logged_in(&self, host: &str) -> bool {
        if let Some((ok, at)) = self.logins.lock().unwrap().get(host)
            && at.elapsed() < Self::LOGIN_TTL
        {
            return *ok;
        }
        let ok = self
            .command()
            .args(["auth", "status", "--hostname", host])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        self.logins
            .lock()
            .unwrap()
            .insert(host.to_owned(), (ok, Instant::now()));
        ok
    }

    /// The newest pull request for `branch` in `remote`, if any. Blocking.
    pub fn pr_for_branch(&self, remote: &RemoteRepo, branch: &str) -> Option<PrInfo> {
        let out = self
            .command()
            .args([
                "pr",
                "list",
                "--repo",
                &remote.slug(),
                "--head",
                branch,
                "--state",
                "all",
                "--limit",
                "1",
                "--json",
                "number,state,url,statusCheckRollup",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_pr_list(&String::from_utf8_lossy(&out.stdout))
    }
}

/// The hint shown on a repository whose host `gh` is not logged in to.
pub fn login_hint(host: &str) -> String {
    format!("Log in with `gh auth login --hostname {host}` for PR badges")
}

/// Parse `gh pr list --json number,state,url,statusCheckRollup` output.
pub fn parse_pr_list(json: &str) -> Option<PrInfo> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let pr = list.first()?;
    let number = pr.get("number")?.as_u64()?;
    let url = pr.get("url")?.as_str()?.to_owned();
    let state = match pr.get("state")?.as_str()? {
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        _ => PrState::Closed,
    };
    let mut checks = CheckState::None;
    if let Some(rollup) = pr.get("statusCheckRollup").and_then(|v| v.as_array()) {
        let mut pending = false;
        let mut failed = false;
        for check in rollup {
            // Check runs report status/conclusion; commit statuses report state.
            let status = check.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let conclusion = check
                .get("conclusion")
                .and_then(|v| v.as_str())
                .or_else(|| check.get("state").and_then(|v| v.as_str()))
                .unwrap_or("");
            if status != "COMPLETED" && !status.is_empty() {
                pending = true;
            } else {
                match conclusion {
                    "SUCCESS" | "NEUTRAL" | "SKIPPED" => {}
                    "PENDING" | "EXPECTED" | "" => pending = true,
                    _ => failed = true,
                }
            }
        }
        checks = if rollup.is_empty() {
            CheckState::None
        } else if failed {
            CheckState::Failure
        } else if pending {
            CheckState::Pending
        } else {
            CheckState::Success
        };
    }
    Some(PrInfo {
        number,
        url,
        state,
        checks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pr_list_output() {
        let json = r#"[{"number":5,"state":"MERGED","url":"https://github.com/x/y/pull/5",
            "statusCheckRollup":[{"__typename":"CheckRun","name":"macOS","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"CheckRun","name":"Linux","status":"IN_PROGRESS","conclusion":""}]}]"#;
        let pr = parse_pr_list(json).unwrap();
        assert_eq!(
            (pr.number, pr.state, pr.checks),
            (5, PrState::Merged, CheckState::Pending)
        );
        let json = r#"[{"number":6,"state":"OPEN","url":"u","statusCheckRollup":[{"__typename":"StatusContext","context":"ci","state":"FAILURE"}]}]"#;
        assert_eq!(parse_pr_list(json).unwrap().checks, CheckState::Failure);
        assert_eq!(parse_pr_list("[]"), None);
    }

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
}
