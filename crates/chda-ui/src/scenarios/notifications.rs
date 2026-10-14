//! In-app notifications retain independent events and preserve typing focus.
use gpui::TestAppContext;

use super::harness::Harness;
use crate::workspace_view::{ResumeSession, RunClaude};

#[gpui::test]
fn successive_errors_remain_in_the_title_bar_queue(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "notifications", |_| {});
    h.wait_prompt();
    h.cx.dispatch_action(ResumeSession);
    h.cx.run_until_parked();
    h.cx.dispatch_action(RunClaude);
    h.cx.run_until_parked();
    let toggle =
        h.cx.debug_bounds("notifications-toggle")
            .expect("operational messages have a title-bar notification control");
    h.cx.simulate_click(toggle.center(), Default::default());
    h.cx.run_until_parked();
    assert!(h.cx.debug_bounds("notifications-popup").is_some());
    assert!(h.cx.debug_bounds("notification-entry-1").is_some());
    assert!(h.cx.debug_bounds("notification-entry-2").is_some());
    assert_eq!(h.read(|v, _| v.notifications.unread()), 0);
    h.cx.dispatch_action(RunClaude);
    h.cx.run_until_parked();
    assert_eq!(
        h.read(|v, _| v.notifications.unread()),
        1,
        "arrival while open remains unread"
    );
    assert!(h.cx.debug_bounds("notification-entry-3").is_some());
    h.keys("escape");
    assert!(h.cx.debug_bounds("notifications-popup").is_none());
    assert_eq!(
        h.read(|v, _| v.notifications.newest().len()),
        3,
        "Escape keeps history"
    );
    h.type_text("echo notification-focus");
    h.keys("enter");
    h.wait_for("typing returns to the same terminal", |v, cx| {
        v.focused_text(cx)
            .lines()
            .any(|line| line.trim() == "notification-focus")
    });
}

#[gpui::test]
fn notifications_survive_hidden_sidebar_and_are_local_to_each_execution_window(
    cx: &mut TestAppContext,
) {
    use super::harness::wait_until;
    let mut h = Harness::open(cx, "notification-windows", |_| {});
    h.wait_prompt();
    h.cx.dispatch_action(ResumeSession);
    h.cx.run_until_parked();
    let env = h.read(|v, _| v.env.clone());
    let handle = h.cx.update(|_, cx| {
        crate::open_workspace_window(chda_config::load(&env.ghostty, None), None, env.clone(), cx)
    });
    let mut other_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let other = handle.root(&mut other_cx).unwrap();
    wait_until(&mut other_cx, &other, "second shell", |v, cx| {
        v.focused_text(cx).contains("test%")
    });
    assert_eq!(
        other.read_with(&other_cx, |v, _| v.notifications.unread()),
        0
    );
    assert_eq!(h.read(|v, _| v.notifications.unread()), 1);
    h.view.update(&mut h.cx, |v, cx| {
        v.sidebar_visible = false;
        v.folder_apps.clear();
        cx.notify();
    });
    h.cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(420.0)));
    h.cx.run_until_parked();
    let toggle = h.cx.debug_bounds("notifications-toggle").unwrap().center();
    h.cx.simulate_click(toggle, Default::default());
    h.cx.run_until_parked();
    let bounds = h.cx.debug_bounds("notifications-popup").unwrap();
    assert!(
        bounds.left() >= gpui::px(0.0)
            && bounds.right() <= gpui::px(640.0)
            && bounds.bottom() <= gpui::px(420.0)
    );
    h.keys("escape");
    let (reopened_cx, reopened) = h.reopen();
    reopened_cx.run_until_parked();
    assert_eq!(
        reopened.read_with(&reopened_cx, |v, _| v.notifications.newest().len()),
        0,
        "normal restart clears history"
    );
}

#[gpui::test]
fn agent_notifications_keep_the_exact_source_and_do_not_steal_focus(cx: &mut TestAppContext) {
    use chda_core::agents::HookKind;
    let mut h = Harness::open(cx, "notification-agent", |_| {});
    h.wait_prompt();
    let source = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.keys("cmd-t");
    h.wait_prompt();
    let focused = h.read(|v, _| v.ws.focused_pane().unwrap());
    let home = h.home.home.clone();
    h.hook(Some(source.raw()), &home, HookKind::PromptSubmitted);
    h.hook(Some(source.raw()), &home, HookKind::WaitingInput);
    h.wait_for("the in-app waiting event", |v, _| {
        v.notifications.unread() == 1
    });
    let entry = h.read(|v, _| v.notifications.newest()[0].clone());
    assert_eq!(entry.context.pane, Some(source.raw()));
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(focused));
    let at = h.cx.debug_bounds("notifications-toggle").unwrap().center();
    h.cx.simulate_click(at, Default::default());
    h.cx.run_until_parked();
    let selector = Box::leak(format!("notification-entry-{}", entry.id).into_boxed_str());
    let at = h.cx.debug_bounds(selector).unwrap().center();
    h.cx.simulate_click(at, Default::default());
    h.cx.run_until_parked();
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(source));
    assert!(!h.read(|v, _| v.notifications_open));
}

fn notice_count(h: &Harness, message: &str) -> usize {
    h.read(|v, _| {
        v.notifications
            .newest()
            .iter()
            .filter(|e| e.message == message)
            .count()
    })
}

/// Programs ask for desktop notifications with OSC 9 and OSC 777 (#250).
#[gpui::test]
fn program_notifications_reach_the_history_and_their_exact_pane(cx: &mut TestAppContext) {
    use chda_core::agents::HookKind;
    let mut h = Harness::open(cx, "osc-notify", |_| {});
    h.wait_prompt();
    let first = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.cx.update(|window, _| window.activate_window());
    h.cx.run_until_parked();
    // In view: a history entry with the pane, no OS notification.
    h.run(
        "printf '\\033]9;in-view\\007'; printf 'sent-%s\\n' 1",
        "sent-1",
    );
    h.wait_for("the in-view entry", |v, _| {
        v.notifications
            .newest()
            .iter()
            .any(|e| e.message == "in-view" && e.context.pane == Some(first.raw()))
    });
    assert!(h.system.0.borrow().notifications.is_empty());

    // Out of view: another pane asks while the first one has focus.
    h.keys("cmd-d");
    h.wait_prompt();
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.type_text("sleep 1; printf '\\033]777;notify;Build;done\\007'");
    h.keys("enter");
    h.keys("cmd-alt-left");
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(first));
    h.wait_for("the out-of-view entry", |v, _| {
        v.notifications
            .newest()
            .iter()
            .any(|e| e.message == "Build: done" && e.context.pane == Some(second.raw()))
    });
    let (title, body, target) = h.system.0.borrow().notifications[0].clone();
    assert_eq!((title.as_str(), body.as_str()), ("Build", "done"));
    assert_eq!(target.pane, Some(second.raw()));
    assert!(h.read(|v, _| v.ws.pane(second).unwrap().bell), "tab mark");
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.on_notification_click(target, window, cx))
    });
    h.cx.run_until_parked();
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(second));

    // A flood keeps one a second (in the first pane, whose last request
    // was long ago).
    h.keys("cmd-alt-left");
    h.run(
        "printf '\\033]9;flood\\007\\033]9;flood\\007\\033]9;flood\\007'; printf 'sent-%s\\n' 2",
        "sent-2",
    );
    assert_eq!(notice_count(&h, "flood"), 1);
    h.keys("cmd-alt-right");

    // An agent that reports through hooks is not notified twice.
    let home = h.home.home.clone();
    h.type_text("sleep 1; printf '\\033]9;from-agent\\007'; printf 'agent-%s\\n' done; sleep 1");
    h.keys("enter");
    h.hook(Some(second.raw()), &home, HookKind::SessionStart);
    h.wait_for("the agent's output", |v, cx| {
        v.focused_text(cx).contains("agent-done")
    });
    assert_eq!(notice_count(&h, "from-agent"), 0);
}

#[gpui::test]
fn program_notifications_follow_the_notifications_setting(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "osc-notify-off", |home| {
        std::fs::write(&home.config, "notifications = false\n").unwrap();
    });
    h.wait_prompt();
    h.run(
        "printf '\\033]9;quiet\\007'; printf 'sent-%s\\n' 3",
        "sent-3",
    );
    assert_eq!(notice_count(&h, "quiet"), 0);
    assert!(h.system.0.borrow().notifications.is_empty());
}
