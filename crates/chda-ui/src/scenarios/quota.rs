//! Startup, missing reports, account-safe retry and app-wide quota ownership.
use super::harness::{Harness, wait_until};
use chda_core::agents::{control::Request, ipc, quota::claude_quota};
use gpui::TestAppContext;
use serde_json::json;

#[gpui::test]
fn details_open_above_both_left_controls_and_visualize_only_reported_windows(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "quota-details", |_| {});
    h.wait_prompt();
    for provider in ["claude", "codex"] {
        h.cx.run_until_parked();
        let selector = Box::leak(format!("usage-provider-{provider}").into_boxed_str());
        let control = h.cx.debug_bounds(selector).unwrap();
        h.cx.simulate_click(control.center(), gpui::Modifiers::none());
        h.cx.run_until_parked();
        let popup = h.cx.debug_bounds("usage-popup").unwrap();
        assert!(
            popup.left() <= gpui::px(16.0),
            "popup is anchored to the left provider area: {popup:?}"
        );
        assert!(
            popup.bottom() <= control.top() + gpui::px(2.0),
            "trigger remains visible"
        );
        assert!(
            h.cx.debug_bounds("quota-window-0-0").is_none(),
            "unknown quota creates no bar"
        );
        let close = h.cx.debug_bounds("usage-close").unwrap().center();
        h.cx.simulate_click(close, gpui::Modifiers::none());
        h.cx.run_until_parked();
    }
    let report=claude_quota(&json!({"session_id":"details", "rate_limits":{"five_hour":{"used_percentage":0},"seven_day":{"used_percentage":99}}}),None,crate::terminal_view::now_ms()-300_001).unwrap();
    h.view.update(&mut h.cx, |v, cx| {
        v.status_bar.quotas.borrow_mut().report(report);
        cx.notify();
    });
    h.cx.run_until_parked();
    let at = h.cx.debug_bounds("usage-provider-claude").unwrap().center();
    h.cx.simulate_click(at, gpui::Modifiers::none());
    h.cx.run_until_parked();
    for index in 0..2 {
        assert!(
            h.cx.debug_bounds(Box::leak(
                format!("quota-window-0-{index}").into_boxed_str()
            ))
            .is_some()
        );
    }
    h.cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(420.0)));
    h.cx.run_until_parked();
    let popup = h.cx.debug_bounds("usage-popup").unwrap();
    assert!(
        popup.left() >= gpui::px(0.0)
            && popup.right() <= gpui::px(640.0)
            && popup.top() >= gpui::px(0.0)
    );
    let updated = claude_quota(
        &json!({"session_id":"details","rate_limits":{"five_hour":{"used_percentage":100}}}),
        None,
        crate::terminal_view::now_ms(),
    )
    .unwrap();
    h.view.update(&mut h.cx, |v, cx| {
        v.status_bar.quotas.borrow_mut().report(updated);
        cx.notify();
    });
    h.cx.run_until_parked();
    assert!(h.cx.debug_bounds("quota-window-0-0").is_some());
    assert!(
        h.cx.debug_bounds("quota-window-0-1").is_none(),
        "live update replaces the reported windows"
    );
}

fn text(view: &crate::workspace_view::WorkspaceView, provider: &str) -> String {
    view.status_bar.quotas.borrow().text(
        provider,
        view.ws.focused_pane(),
        crate::terminal_view::now_ms(),
    )
}

#[gpui::test]
fn startup_and_restart_wait_for_usage_and_empty_reports_keep_the_last_reading(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "quota-startup", |_| {});
    h.wait_prompt();
    assert_eq!(h.read(|v, _| text(v, "claude")), "Waiting for usage");
    assert_eq!(h.read(|v, _| text(v, "codex")), "Loading");
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let at = crate::terminal_view::now_ms();
    let report = claude_quota(
        &json!({"session_id":"s", "rate_limits":{"five_hour":{"used_percentage":25}}}),
        Some(pane.raw()),
        at,
    )
    .unwrap();
    ipc::send_quota(&ipc::socket_path(&h.home.data), &report).unwrap();
    h.wait_for("reported Claude quota", |v, _| {
        text(v, "claude") == "25% · 5h"
    });
    let empty = claude_quota(&json!({"session_id":"s"}), Some(pane.raw()), at + 1).unwrap();
    ipc::send_quota(&ipc::socket_path(&h.home.data), &empty).unwrap();
    let cwd = h.home.home.clone();
    super::mcp::ask(&mut h, Request::ListWorktrees { cwd });
    assert_eq!(h.read(|v, _| text(v, "claude")), "25% · 5h");
    assert_eq!(
        h.read(|v, _| v
            .status_bar
            .quotas
            .borrow()
            .provider("claude", Some(pane))
            .unwrap()
            .observed_at),
        at
    );
    let (mut reopened_cx, reopened) = h.reopen();
    wait_until(&mut reopened_cx, &reopened, "restored pane", |v, _| {
        v.ws.focused_pane().is_some()
    });
    reopened.read_with(&reopened_cx, |v, _| {
        // The bridge in tests sends no configuration folder, so nothing is
        // kept across the restart.
        assert_eq!(text(v, "claude"), "Waiting for usage");
        assert_eq!(text(v, "codex"), "Loading");
    });
}

#[gpui::test]
fn a_restart_keeps_the_last_claude_reading_until_a_new_one(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "quota-cache", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let mut report = claude_quota(
        &json!({"session_id":"s", "rate_limits":{"five_hour":{"used_percentage":25}}}),
        Some(pane.raw()),
        crate::terminal_view::now_ms(),
    )
    .unwrap();
    report.config_dir = Some("/home/.claude".into());
    ipc::send_quota(&ipc::socket_path(&h.home.data), &report).unwrap();
    h.wait_for("reported Claude quota", |v, _| {
        text(v, "claude") == "25% · 5h"
    });
    let (mut reopened_cx, reopened) = h.reopen();
    wait_until(&mut reopened_cx, &reopened, "the kept reading", |v, _| {
        text(v, "claude") == "25% · 5h · previous run"
    });
}

#[gpui::test]
fn claude_quota_routed_from_another_window_keeps_both_windows_running(cx: &mut TestAppContext) {
    use chda_core::agents::control::Incoming;
    use std::{cell::Cell, rc::Rc};

    let mut h = Harness::open(cx, "claude-quota-windows", |_| {});
    h.wait_prompt();
    let first_pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let env = h.read(|v, _| v.env.clone());
    let handle =
        h.cx.update(|_, cx| crate::open_workspace_window(None, env.clone(), cx));
    let mut second_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let second = handle.root(&mut second_cx).unwrap();
    wait_until(&mut second_cx, &second, "second shell", |v, cx| {
        v.focused_prompt_ready(cx)
    });
    let second_pane = second.read_with(&second_cx, |v, _| v.ws.focused_pane().unwrap());
    h.cx.run_until_parked();
    let redraws = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
    h.cx.update(|_, cx| {
        for (view, count) in [(&h.view, redraws[0].clone()), (&second, redraws[1].clone())] {
            cx.observe(view, move |_, _| count.set(count.get() + 1))
                .detach();
        }
    });

    // Either window can drain the shared queue. Wake only the other window
    // to reproduce the nested route deterministically, without an IPC race.
    let deliver = |report, receiver: usize| {
        let wake = {
            let mut registry = env.windows.borrow_mut();
            registry.replay.push(Incoming::Quota(report));
            registry.wake.clone()
        };
        wake.lock().unwrap()[receiver].unbounded_send(()).unwrap();
    };
    let at = crate::terminal_view::now_ms();
    for (pane, session, used, receiver) in
        [(second_pane, "second", 25, 0), (first_pane, "first", 47, 1)]
    {
        let mut report = claude_quota(
            &json!({"session_id": session, "rate_limits": {"five_hour": {"used_percentage": used}}}),
            Some(pane.raw()),
            at + receiver as u64,
        )
        .unwrap();
        report.config_dir = Some(h.home.home.join(".claude").to_string_lossy().into_owned());
        let before = redraws.each_ref().map(|count| count.get());
        deliver(report, receiver);
        let expected = format!("{used}% · 5h");
        h.wait_for("quota shared with the first window", |v, _| {
            text(v, "claude") == expected
        });
        wait_until(
            &mut second_cx,
            &second,
            "quota shared with the second window",
            |v, _| text(v, "claude") == expected,
        );
        for (index, count) in redraws.iter().enumerate() {
            assert!(
                count.get() > before[index],
                "window {index} must refresh its displayed quota"
            );
        }
        assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(first_pane));
        assert_eq!(
            second.read_with(&second_cx, |v, _| v.ws.focused_pane()),
            Some(second_pane)
        );
    }
    assert_eq!(
        h.read(|v, _| v
            .ws
            .pane(first_pane)
            .unwrap()
            .agent_session
            .as_ref()
            .unwrap()
            .session
            .clone()),
        "first"
    );
    assert_eq!(
        second.read_with(&second_cx, |v, _| v
            .ws
            .pane(second_pane)
            .unwrap()
            .agent_session
            .as_ref()
            .unwrap()
            .session
            .clone()),
        "second"
    );
    h.type_text("printf 'first-still-running\\n'");
    h.keys("enter");
    h.wait_for("first shell still runs", |v, cx| {
        v.focused_text(cx)
            .lines()
            .any(|line| line.trim_end() == "first-still-running")
    });
    second_cx.simulate_input("printf 'second-still-running\\n'");
    second_cx.simulate_keystrokes("enter");
    wait_until(
        &mut second_cx,
        &second,
        "second shell still runs",
        |v, cx| {
            v.focused_text(cx)
                .lines()
                .any(|line| line.trim_end() == "second-still-running")
        },
    );
}

#[gpui::test]
fn windows_share_one_codex_probe_and_it_survives_closing_the_first_window(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "quota-windows", |home| {
        use std::os::unix::fs::PermissionsExt;
        // A stand-in reads only this test home. Its first quota refresh fails;
        // the next succeeds. It never starts a conversation or model request.
        let script = r#"#!/bin/sh
cd "$(dirname "$0")"
echo probe >> probes
while IFS= read -r line; do
    case "$line" in
        *'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}' ;;
        *'"method":"account/read"'*) printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"test@example.test"}}}' ;;
        *'"method":"account/rateLimits/read"'*)
            if [ ! -e failed ]; then touch failed; exit 0; fi
            printf '%s\n' '{"id":3,"result":{"rateLimits":{"primary":{"usedPercent":35,"windowDurationMins":300}}}}'
            ;;
    esac
done
"#;
        std::fs::create_dir_all(&home.bin).unwrap();
        let executable = home.bin.join("codex");
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            home.home.join("gui-path"),
            format!("{}:/usr/bin:/bin", home.bin.display()),
        )
        .unwrap();
        std::fs::write(
            home.home.join(".zshrc"),
            "PS1='test%# '\nPROMPT_EOL_MARK=''\n",
        )
        .unwrap();
    });
    h.wait_prompt();
    let env = h.read(|v, _| v.env.clone());
    let handle =
        h.cx.update(|_, cx| crate::open_workspace_window(None, env.clone(), cx));
    let mut second_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let second = handle.root(&mut second_cx).unwrap();
    h.cx.update(|_, cx| h.view.update(cx, |v, cx| v.start_codex_quota(cx)));
    second_cx.update(|_, cx| second.update(cx, |v, cx| v.start_codex_quota(cx)));
    h.wait_for("first refresh to retry", |v, _| {
        text(v, "codex") == "Retrying"
    });
    assert_eq!(
        second.read_with(&second_cx, |v, _| text(v, "codex")),
        "Retrying"
    );
    let probes = || {
        std::fs::read_to_string(h.home.bin.join("probes"))
            .unwrap()
            .lines()
            .count()
    };
    assert_eq!(
        probes(),
        1,
        "opening another window does not duplicate probes"
    );
    h.cx.update(|window, _| window.remove_window());
    second_cx
        .executor()
        .advance_clock(std::time::Duration::from_secs(3));
    wait_until(
        &mut second_cx,
        &second,
        "quota after the owner window closes",
        |v, _| text(v, "codex") == "35% · codex 5h",
    );
    assert_eq!(
        probes(),
        2,
        "failed refresh retries promptly rather than after a minute"
    );
}

#[gpui::test]
fn codex_usage_appears_when_the_login_shell_is_slower_than_the_launch_wait(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "quota-slow-path", |home| {
        use std::os::unix::fs::PermissionsExt;
        let script = r#"#!/bin/sh
while IFS= read -r line; do
    case "$line" in
        *'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}' ;;
        *'"method":"account/read"'*) printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"test@example.test"}}}' ;;
        *'"method":"account/rateLimits/read"'*) printf '%s\n' '{"id":3,"result":{"rateLimits":{"primary":{"usedPercent":35,"windowDurationMins":300}}}}' ;;
    esac
done
"#;
        std::fs::create_dir_all(&home.bin).unwrap();
        let executable = home.bin.join("codex");
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        // A Dock launch inherits launchd's PATH; only the login shell adds
        // the CLI's directory, after more than the two-second launch wait.
        std::fs::write(home.home.join("gui-path"), "/usr/bin:/bin:/usr/sbin:/sbin").unwrap();
        std::fs::write(
            home.home.join(".zprofile"),
            format!("sleep 3\nexport PATH='{}':$PATH\n", home.bin.display()),
        )
        .unwrap();
        std::fs::write(
            home.home.join(".zshrc"),
            "PS1='test%# '\nPROMPT_EOL_MARK=''\n",
        )
        .unwrap();
    });
    h.cx.update(|_, cx| h.view.update(cx, |v, cx| v.start_codex_quota(cx)));
    h.wait_for("Codex usage through the late PATH", |v, _| {
        text(v, "codex") == "35% · codex 5h"
    });
}

#[test]
fn missing_codex_names_the_paths_searched_and_a_failed_capture() {
    use chda_core::agents::quota::CodexQuotaError;
    let missing = CodexQuotaError::NotInstalled {
        searched: "/opt/homebrew/bin:/usr/bin".into(),
    };
    assert_eq!(missing.label(), "Not installed");
    assert!(missing.details().contains("/opt/homebrew/bin:/usr/bin"));
    let unread = CodexQuotaError::PathUnavailable {
        error: "timed out".into(),
        searched: "/usr/bin:/bin".into(),
    };
    assert_eq!(unread.label(), "Could not read the shell PATH");
    assert!(unread.details().contains("timed out"));
    assert!(unread.details().contains("/usr/bin:/bin"));
}
