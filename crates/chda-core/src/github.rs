//! Pull request badges through the `gh` CLI. chda never reads the token;
//! it only runs `gh` and parses its JSON.

use std::path::Path;
use std::process::Command;

use crate::sidebar::{CheckState, PrInfo, PrState};

/// Whether `gh` is installed and logged in. Cached by callers.
pub fn gh_available() -> bool {
    Command::new("gh")
        .args(["auth", "status"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The newest pull request for `branch` in `repo`, if any. Blocking.
pub fn pr_for_branch(repo: &Path, branch: &str) -> Option<PrInfo> {
    let out = Command::new("gh")
        .args([
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "all",
            "--limit",
            "1",
            "--json",
            "number,state,url,statusCheckRollup",
        ])
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_pr_list(&String::from_utf8_lossy(&out.stdout))
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
}
