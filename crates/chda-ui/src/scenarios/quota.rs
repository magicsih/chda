//! Startup, missing reports, account-safe retry and app-wide quota ownership.
use super::harness::{Harness, wait_until};
use chda_core::agents::{control::Request, ipc, quota::claude_quota};
use gpui::TestAppContext;
use serde_json::json;

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
        assert_eq!(text(v, "claude"), "Waiting for usage");
        assert_eq!(text(v, "codex"), "Loading");
    });
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
    let handle = h.cx.update(|_, cx| {
        crate::open_workspace_window(chda_config::load(&env.ghostty, None), None, env.clone(), cx)
    });
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
