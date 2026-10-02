//! GitLab merge requests through `glab`. The host goes in `GITLAB_HOST`,
//! the project path in `--repo`.

use crate::forge::RemoteRepo;
use crate::sidebar::{CheckState, PrInfo, PrState};

/// `glab` arguments for the newest merge request from `branch`.
pub(crate) fn mr_list_args(remote: &RemoteRepo, branch: &str) -> Vec<String> {
    [
        "mr",
        "list",
        "--repo",
        &remote.path,
        "--source-branch",
        branch,
        "--all",
        "--per-page",
        "1",
        "--output",
        "json",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// `glab` arguments for one merge request, which carries its pipeline.
pub(crate) fn mr_view_args(remote: &RemoteRepo, iid: u64) -> Vec<String> {
    [
        "mr",
        "view",
        &iid.to_string(),
        "--repo",
        &remote.path,
        "--output",
        "json",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// The first merge request of `glab mr list --output json` (the GitLab
/// API's merge request objects). Checks are filled in separately.
pub fn parse_mr_list(json: &str) -> Option<PrInfo> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let mr = list.first()?;
    Some(PrInfo {
        number: mr.get("iid")?.as_u64()?,
        url: mr.get("web_url")?.as_str()?.to_owned(),
        state: match mr.get("state")?.as_str()? {
            "opened" | "locked" => PrState::Open,
            "merged" => PrState::Merged,
            _ => PrState::Closed,
        },
        checks: CheckState::None,
    })
}

/// The check state from `glab mr view --output json`: its head pipeline.
pub fn parse_pipeline(json: &str) -> CheckState {
    let Ok(mr) = serde_json::from_str::<serde_json::Value>(json) else {
        return CheckState::None;
    };
    let status = mr
        .get("head_pipeline")
        .or_else(|| mr.get("pipeline"))
        .and_then(|p| p.get("status"))
        .and_then(|s| s.as_str());
    match status {
        None => CheckState::None,
        Some("success" | "skipped") => CheckState::Success,
        Some("failed" | "canceled") => CheckState::Failure,
        // created, waiting_for_resource, preparing, pending, running,
        // manual, scheduled.
        Some(_) => CheckState::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shapes from the GitLab merge requests API, which `glab --output json`
    // prints as is.
    #[test]
    fn parses_merge_requests_and_pipelines() {
        let list = r#"[{"id":1,"iid":12,"project_id":3,"title":"Add x","state":"opened",
            "source_branch":"feat","target_branch":"main",
            "web_url":"https://gitlab.example.com/group/proj/-/merge_requests/12"}]"#;
        let mr = parse_mr_list(list).unwrap();
        assert_eq!(
            (mr.number, mr.state, mr.url.as_str()),
            (
                12,
                PrState::Open,
                "https://gitlab.example.com/group/proj/-/merge_requests/12"
            )
        );
        let merged = list.replace("\"opened\"", "\"merged\"");
        assert_eq!(parse_mr_list(&merged).unwrap().state, PrState::Merged);
        let closed = list.replace("\"opened\"", "\"closed\"");
        assert_eq!(parse_mr_list(&closed).unwrap().state, PrState::Closed);
        assert_eq!(parse_mr_list("[]"), None);

        let view = |status: &str| {
            format!(
                r#"{{"iid":12,"state":"opened","head_pipeline":{{"id":7,"status":"{status}"}}}}"#
            )
        };
        assert_eq!(parse_pipeline(&view("success")), CheckState::Success);
        assert_eq!(parse_pipeline(&view("failed")), CheckState::Failure);
        assert_eq!(parse_pipeline(&view("running")), CheckState::Pending);
        assert_eq!(
            parse_pipeline(r#"{"iid":12,"head_pipeline":null}"#),
            CheckState::None
        );
    }

    #[test]
    fn arguments_name_the_project_path() {
        let remote = RemoteRepo {
            host: "gitlab.example.com".into(),
            path: "group/sub/proj".into(),
        };
        assert_eq!(
            mr_list_args(&remote, "feat").join(" "),
            "mr list --repo group/sub/proj --source-branch feat --all --per-page 1 --output json"
        );
        assert_eq!(
            mr_view_args(&remote, 12).join(" "),
            "mr view 12 --repo group/sub/proj --output json"
        );
    }
}
