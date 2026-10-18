//! Release discovery and installation capabilities belong to each platform.

use chda_core::release::Release;

pub(crate) fn endpoint() -> &'static str {
    if cfg!(target_os = "linux") {
        "https://api.github.com/repos/magicsih/chda/releases?per_page=100"
    } else {
        chda_core::release::LATEST_RELEASE_URL
    }
}

pub(crate) fn parse(json: &str) -> Option<Release> {
    #[cfg(target_os = "linux")]
    {
        chda_core::release::parse_latest_with_asset(json, "linux-x86_64.AppImage")
    }
    #[cfg(not(target_os = "linux"))]
    {
        chda_core::release::parse_latest(json)
    }
}

pub(crate) fn can_install() -> bool {
    cfg!(target_os = "macos")
}
