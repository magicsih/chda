//! New-version check: at most once a day, ask GitHub for the latest
//! release and tell the user when it is newer than the running version.
//! Only the request itself leaves the machine; nothing about the user is
//! sent.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The one request the check makes.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/magicsih/chda/releases/latest";

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Without the leading `v`, e.g. `0.2.0`.
    pub version: String,
    /// The release notes page.
    pub url: String,
}

/// What the check remembers between runs (`release-check.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ReleaseCheck {
    /// When GitHub last answered, in milliseconds since the Unix epoch.
    pub checked_at: u64,
    pub latest: Option<Release>,
    /// The version whose notice the user dismissed.
    pub dismissed: Option<String>,
}

impl ReleaseCheck {
    /// The saved state, or a fresh one when the file is missing or broken.
    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, serde_json::to_vec_pretty(self)?)
    }

    /// Whether GitHub never answered or a day has passed since it did. A
    /// clock that went backwards counts as due.
    pub fn due(&self, now_ms: u64) -> bool {
        self.checked_at == 0 || now_ms < self.checked_at || now_ms - self.checked_at >= DAY_MS
    }

    /// Remember GitHub's answer.
    pub fn record(&mut self, now_ms: u64, latest: Release) {
        self.checked_at = now_ms;
        self.latest = Some(latest);
    }

    /// The release to tell the user about: newer than `current` and not
    /// dismissed.
    pub fn notice(&self, current: &str) -> Option<&Release> {
        self.latest
            .as_ref()
            .filter(|r| is_newer(&r.version, current))
            .filter(|r| self.dismissed.as_deref() != Some(r.version.as_str()))
    }

    /// Hide the notice until a newer release than the one shown.
    pub fn dismiss(&mut self) {
        self.dismissed = self.latest.as_ref().map(|r| r.version.clone());
    }
}

/// The release in a `releases/latest` response from the GitHub API.
pub fn parse_latest(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v.get("tag_name")?.as_str()?;
    let url = v.get("html_url")?.as_str()?;
    Some(Release {
        version: tag.strip_prefix('v').unwrap_or(tag).to_owned(),
        url: url.to_owned(),
    })
}

/// The newest stable published release containing this exact platform asset.
/// GitHub's global latest release may belong to another operating system.
pub fn parse_latest_with_asset(json: &str, platform: &str) -> Option<Release> {
    let releases: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    releases
        .iter()
        .filter(|value| value.get("draft").and_then(|v| v.as_bool()) == Some(false))
        .filter(|value| value.get("prerelease").and_then(|v| v.as_bool()) == Some(false))
        .filter_map(|value| {
            let tag = value.get("tag_name")?.as_str()?;
            let version = tag.strip_prefix('v')?;
            if !is_newer(version, "0.0.0") {
                return None;
            }
            let expected = format!("chda-{version}-{platform}");
            if !value
                .get("assets")?
                .as_array()?
                .iter()
                .any(|asset| asset.get("name").and_then(|v| v.as_str()) == Some(expected.as_str()))
            {
                return None;
            }
            Some(Release {
                version: version.to_owned(),
                url: value.get("html_url")?.as_str()?.to_owned(),
            })
        })
        .reduce(|latest, candidate| {
            if is_newer(&candidate.version, &latest.version) {
                candidate
            } else {
                latest
            }
        })
}

/// Whether version `a` is newer than `b`, comparing `major.minor.patch`
/// numerically. Anything else (a pre-release suffix, a typo) is not newer.
pub fn is_newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> Option<[u64; 3]> {
        let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
        let parts = [it.next()??, it.next()??, it.next()??];
        it.next().is_none().then_some(parts)
    }
    matches!((parts(a), parts(b)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            url: format!("https://github.com/magicsih/chda/releases/tag/v{version}"),
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.9", "0.1.9"));
        assert!(!is_newer("0.1.8", "0.1.9"));
        assert!(!is_newer("0.2.0-rc.1", "0.1.9"));
        assert!(!is_newer("0.2", "0.1.9"));
    }

    #[test]
    fn parses_the_latest_release() {
        let json = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/magicsih/chda/releases/tag/v0.2.0","draft":false}"#;
        assert_eq!(parse_latest(json), Some(release("0.2.0")));
        assert_eq!(parse_latest(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_latest("<html>"), None);
    }

    #[test]
    fn release_discovery_requires_a_published_matching_platform_asset() {
        let entry = |version: &str, platform: &str, draft: bool, prerelease: bool| {
            serde_json::json!({
                "tag_name": format!("v{version}"), "html_url": format!("https://example.test/v{version}"),
                "draft": draft, "prerelease": prerelease,
                "assets": [{"name": format!("chda-{version}-{platform}")}]
            })
        };
        let suffix = "linux-x86_64.AppImage";
        let data = serde_json::json!([
            entry("9.0.0", "macos.zip", false, false),
            entry("2.0.0", suffix, true, false),
            entry("1.2.0-rc.1", suffix, false, true),
            entry("1.1.9", suffix, false, false),
            entry("1.1.10", suffix, false, false)
        ]);
        assert_eq!(
            parse_latest_with_asset(&data.to_string(), suffix)
                .unwrap()
                .version,
            "1.1.10"
        );
        assert!(parse_latest_with_asset("[]", suffix).is_none());
        assert!(
            parse_latest_with_asset(&entry("1.1.0", suffix, false, false).to_string(), suffix)
                .is_none()
        );
        assert!(parse_latest_with_asset("<html>", suffix).is_none());
    }

    #[test]
    fn daily_checks_and_dismissal() {
        let mut check = ReleaseCheck::default();
        assert!(check.due(5));
        check.record(DAY_MS, release("0.2.0"));
        assert!(!check.due(DAY_MS + 1));
        assert!(check.due(2 * DAY_MS));
        assert!(check.due(DAY_MS - 1), "clock went backwards");

        assert_eq!(check.notice("0.1.9"), Some(&release("0.2.0")));
        assert_eq!(check.notice("0.2.0"), None);
        check.dismiss();
        assert_eq!(check.notice("0.1.9"), None);
        check.record(3 * DAY_MS, release("0.2.1"));
        assert_eq!(check.notice("0.1.9"), Some(&release("0.2.1")));

        let dir = std::env::temp_dir().join(format!("chda-release-{}", std::process::id()));
        let file = dir.join("release-check.json");
        check.save(&file).unwrap();
        assert_eq!(ReleaseCheck::load(&file), check);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(ReleaseCheck::load(&file), ReleaseCheck::default());
    }
}
