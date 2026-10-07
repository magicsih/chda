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
        h.cx.debug_bounds("update-menu")
            .expect("the options button is drawn");
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
    assert_eq!(labels, ["Release notes", "Dismiss until the next release"]);
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

#[gpui::test]
fn one_click_shares_progress_and_failed_handoff_restores_the_live_shell(cx: &mut TestAppContext) {
    use chda_core::self_update::UpdateProgress;
    let mut h = Harness::open(cx, "update-handoff", |home| {
        std::fs::write(&home.latest_release, LATEST).unwrap();
        let job = home.data.join("update-fixture");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(
            job.join("progress.jsonl"),
            "{\"phase\":\"downloading\",\"received\":5,\"total\":10}\n",
        )
        .unwrap();
    });
    h.wait_prompt();
    h.run("export CHDA_SURVIVAL=still-here", "test%");
    h.wait_for("release", |v, _| v.release_notice.is_some());
    h.cx.run_until_parked();
    let button = h.cx.debug_bounds("update-notice").unwrap();
    h.cx.simulate_click(button.center(), gpui::Modifiers::none());
    h.wait_for("updater launched", |v, _| {
        v.env.windows.borrow().update.job.is_some()
    });
    h.cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    h.wait_for("download", |v, _| {
        matches!(
            v.env.windows.borrow().update.progress,
            UpdateProgress::Downloading { .. }
        )
    });
    let first_window = h.cx.update(|window, _| window.window_handle());
    cx.simulate_window_resize(first_window, gpui::size(gpui::px(390.0), gpui::px(420.0)));
    h.cx.run_until_parked();
    let narrow =
        h.cx.debug_bounds("update-notice")
            .expect("progress is visible in a narrow window");
    assert!(
        narrow.right() <= gpui::px(390.0),
        "progress runs off the window"
    );
    let env = h.read(|v, _| v.env.clone());
    let handle = h.cx.update(|_, cx| {
        crate::open_workspace_window(chda_config::load(&env.ghostty, None), None, env.clone(), cx)
    });
    let mut second_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let second = handle.root(&mut second_cx).unwrap();
    super::harness::wait_until(&mut second_cx, &second, "second shell", |v, cx| {
        v.focused_text(cx).contains("test%")
    });
    assert_eq!(env.windows.borrow().entries.len(), 2);
    second_cx.update(|window, cx| {
        second.update(cx, |v, cx| {
            assert!(matches!(
                v.env.windows.borrow().update.progress,
                UpdateProgress::Downloading { .. }
            ));
            v.start_update(window, cx);
        })
    });
    h.cx.run_until_parked();
    let job = h.home.data.join("update-fixture");
    assert_eq!(
        std::fs::read_to_string(job.join("launches"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let operation = env.windows.borrow().update.operation();
    std::fs::write(job.join("progress.jsonl"), "{\"phase\":\"ready\"}\n").unwrap();
    h.cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    h.wait_for("current task before handoff", |v, _| {
        let registry = v.env.windows.borrow();
        registry.update.progress == UpdateProgress::Waiting && !registry.update.frozen
    });
    h.run(
        "printf 'waiting:%s\\n' \"$CHDA_SURVIVAL\"",
        "waiting:still-here",
    );
    assert!(!job.join("cancel").exists());
    drop(operation);
    h.cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    h.wait_for("failed broker rollback", |v, _| {
        let registry = v.env.windows.borrow();
        matches!(registry.update.progress, UpdateProgress::Failed { .. }) && !registry.update.frozen
    });
    assert!(job.join("cancel").exists());
    h.run(
        "printf 'rollback:%s\\n' \"$CHDA_SURVIVAL\"",
        "rollback:still-here",
    );
}
