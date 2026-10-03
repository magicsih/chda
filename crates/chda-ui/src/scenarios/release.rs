//! The new-version notice in the title bar.

use gpui::TestAppContext;

use super::harness::Harness;
use crate::workspace_view::MenuAction;

const LATEST: &str =
    r#"{"tag_name":"v99.0.0","html_url":"https://github.com/magicsih/chda/releases/tag/v99.0.0"}"#;

#[gpui::test]
fn a_newer_release_shows_a_notice_until_dismissed(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "release", |home| {
        std::fs::write(&home.latest_release, LATEST).unwrap();
    });
    h.wait_prompt();
    h.wait_for("the notice", |v, _| {
        v.release_notice.as_ref().map(|r| r.version.as_str()) == Some("99.0.0")
    });
    assert_eq!(h.home.release_requests(), 1);

    h.cx.run_until_parked();
    let bounds =
        h.cx.debug_bounds("update-notice")
            .expect("the notice is drawn");
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    h.cx.run_until_parked();
    let items = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .expect("its menu is open")
            .items
            .clone()
    });
    let labels: Vec<&str> = items.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Release notes",
            "Copy \"brew upgrade --cask chda\"",
            "Dismiss until the next release"
        ]
    );
    assert!(matches!(
        &items[0].1,
        MenuAction::OpenUrl(url) if url.ends_with("/releases/tag/v99.0.0")
    ));

    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(MenuAction::DismissUpdate, window, cx)
        })
    });
    assert!(h.read(|v, _| v.release_notice.is_none()));
    h.cx.run_until_parked();
    assert!(h.cx.debug_bounds("update-notice").is_none());

    // The next launch remembers both the answer and the dismissal: no
    // request within the day, no notice.
    let (cx2, view2) = h.reopen();
    cx2.run_until_parked();
    assert!(cx2.read(|cx| view2.read(cx).release_notice.is_none()));
    assert_eq!(h.home.release_requests(), 1);
}

#[gpui::test]
fn update_check_off_asks_nobody(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "release-off", |home| {
        std::fs::write(&home.latest_release, LATEST).unwrap();
        std::fs::write(&home.config, "update-check = false\n").unwrap();
    });
    h.wait_prompt();
    h.cx.run_until_parked();
    assert_eq!(h.home.release_requests(), 0);
    assert!(h.read(|v, _| v.release_notice.is_none()));
}
