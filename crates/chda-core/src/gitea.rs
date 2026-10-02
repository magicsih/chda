//! Gitea and Forgejo pull requests through `tea`. `tea` names its logins,
//! so the login whose URL is on the repository's host is passed along.

use crate::forge::RemoteRepo;
use crate::sidebar::{CheckState, PrInfo, PrState};

/// The name of the login for `host` in `tea logins list --output json`.
pub(crate) fn login_for_host(json: &str, host: &str) -> Option<String> {
    let logins: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    logins.iter().find_map(|login| {
        let url = login.get("url")?.as_str()?;
        let login_host = url.split_once("://").map_or(url, |(_, rest)| rest);
        let login_host = login_host.split(['/', ':']).next()?;
        login_host
            .eq_ignore_ascii_case(host)
            .then(|| login.get("name")?.as_str().map(str::to_owned))?
    })
}

/// `tea` arguments listing the repository's pull requests, newest first.
pub(crate) fn pr_list_args(login: &str, remote: &RemoteRepo) -> Vec<String> {
    [
        "pulls",
        "list",
        "--login",
        login,
        "--repo",
        &remote.path,
        "--state",
        "all",
        "--limit",
        "50",
        "--fields",
        "index,state,head,url,ci",
        "--output",
        "json",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// The newest pull request from `branch` in `tea pulls list --output json`
/// output, whose values are all strings.
pub fn pr_for_branch(json: &str, branch: &str) -> Option<PrInfo> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let str_of = |pr: &serde_json::Value, key: &str| pr.get(key)?.as_str().map(str::to_owned);
    let pr = list
        .iter()
        .find(|pr| str_of(pr, "head").as_deref() == Some(branch))?;
    Some(PrInfo {
        number: str_of(pr, "index")?.parse().ok()?,
        url: str_of(pr, "url")?,
        state: match str_of(pr, "state")?.as_str() {
            "open" => PrState::Open,
            "merged" => PrState::Merged,
            _ => PrState::Closed,
        },
        // The combined commit status.
        checks: match str_of(pr, "ci").unwrap_or_default().as_str() {
            "" => CheckState::None,
            "success" | "warning" => CheckState::Success,
            "pending" => CheckState::Pending,
            _ => CheckState::Failure,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // tea prints JSON as an array of objects with snake_case header keys and
    // string values (modules/print/table.go).
    #[test]
    fn finds_the_login_for_a_host() {
        let logins = r#"[{"name":"work","url":"https://git.example.com","ssh_host":"git.example.com","user":"me","default":"true"},
            {"name":"codeberg","url":"https://codeberg.org/","ssh_host":"codeberg.org","user":"me","default":"false"}]"#;
        assert_eq!(
            login_for_host(logins, "codeberg.org").as_deref(),
            Some("codeberg")
        );
        assert_eq!(
            login_for_host(logins, "git.example.com").as_deref(),
            Some("work")
        );
        assert_eq!(login_for_host(logins, "gitea.com"), None);
        assert_eq!(login_for_host("not json", "gitea.com"), None);
    }

    #[test]
    fn finds_the_pull_request_of_a_branch() {
        let prs = r#"[{"index":"9","state":"open","head":"feat","url":"https://git.example.com/org/repo/pulls/9","ci":"pending"},
            {"index":"7","state":"merged","head":"old","url":"https://git.example.com/org/repo/pulls/7","ci":"success"},
            {"index":"6","state":"closed","head":"me:fork","url":"u","ci":""}]"#;
        let pr = pr_for_branch(prs, "feat").unwrap();
        assert_eq!(
            (pr.number, pr.state, pr.checks),
            (9, PrState::Open, CheckState::Pending)
        );
        let pr = pr_for_branch(prs, "old").unwrap();
        assert_eq!(
            (pr.state, pr.checks),
            (PrState::Merged, CheckState::Success)
        );
        assert_eq!(pr_for_branch(prs, "fork"), None);
        assert_eq!(pr_for_branch(prs, "none"), None);
        let failed = prs.replace("\"pending\"", "\"failure\"");
        assert_eq!(
            pr_for_branch(&failed, "feat").unwrap().checks,
            CheckState::Failure
        );
    }
}
