//! Actual workspace input and real service processes, under temporary homes.
use super::harness::{Harness, wait_until};
use gpui::{Modifiers, TestAppContext, VisualTestContext};

#[gpui::test]
fn live_agent_attachment_keeps_the_reported_turn_state(cx: &mut TestAppContext) {
    let mut h = Harness::open_service(cx, "service-agent-attach", |home| home.fake_claude());
    h.wait_prompt();
    h.cx.dispatch_action(crate::workspace_view::RunClaude);
    h.cx.run_until_parked();
    h.confirm_agent_launch();
    h.wait_for("managed CLI", |v, cx| {
        v.focused_text(cx).contains("fake-claude")
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &h.home.home,
        chda_core::agents::HookKind::WaitingInput,
        "live-exact",
    );
    h.wait_for("waiting state", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == chda_core::AgentStatus::WaitingInput)
    });
    let (mut restored_cx, restored) = h.reopen();
    wait_until(
        &mut restored_cx,
        &restored,
        "reported state after attachment",
        |v, _| {
            v.ws.pane(pane)
                .unwrap()
                .agent
                .as_ref()
                .is_some_and(|a| a.status == chda_core::AgentStatus::WaitingInput && a.seen)
        },
    );
    assert_eq!(
        h.read(|v, cx| v.panes[&pane].0.read(cx).child_pid()),
        restored.read_with(&restored_cx, |v, cx| v.panes[&pane].0.read(cx).child_pid())
    );
}

#[gpui::test]
fn window_close_detaches_and_sessions_reconnects_the_same_shell(cx: &mut TestAppContext) {
    let mut h = Harness::open_service(cx, "service-window", |_| {});
    h.wait_prompt();
    h.run(
        "printf '\\nwindow-survived-marker\\n'",
        "\nwindow-survived-marker\n",
    );
    let (pane, pid, env) = h.read(|v, cx| {
        let p = v.ws.focused_pane().unwrap();
        (p, v.panes[&p].0.read(cx).child_pid(), v.env.clone())
    });
    h.keys("cmd-shift-n");
    let handle = env.windows.borrow().entries.last().unwrap().window;
    let mut other_cx = VisualTestContext::from_window(handle, cx);
    let other = env
        .windows
        .borrow()
        .entries
        .last()
        .unwrap()
        .view
        .upgrade()
        .unwrap();
    wait_until(&mut other_cx, &other, "second shell", |v, cx| {
        v.focused_prompt_ready(cx)
    });
    h.cx.update(|window, _| window.remove_window());
    h.cx.run_until_parked();
    other_cx
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    wait_until(&mut other_cx, &other, "detached Sessions row", |v, cx| {
        v.sidebar
            .read(cx)
            .detached_sessions
            .iter()
            .any(|r| r.0 == pane.raw())
    });
    let bounds = other_cx
        .debug_bounds(Box::leak(
            format!("detached-session-{}", pane.raw()).into_boxed_str(),
        ))
        .unwrap();
    other_cx.simulate_click(bounds.center(), Modifiers::default());
    other_cx.run_until_parked();
    let handle = env.windows.borrow().entries.last().unwrap().window;
    let mut restored_cx = VisualTestContext::from_window(handle, cx);
    let restored = env
        .windows
        .borrow()
        .entries
        .last()
        .unwrap()
        .view
        .upgrade()
        .unwrap();
    wait_until(
        &mut restored_cx,
        &restored,
        "same shell and output",
        |v, cx| {
            v.panes
                .get(&pane)
                .is_some_and(|(t, _)| t.read(cx).child_pid() == pid)
                && v.focused_text(cx).contains("window-survived-marker")
        },
    );
    restored_cx.simulate_input("printf '\\nafter-reconnect-marker\\n'");
    restored_cx.simulate_keystrokes("enter");
    wait_until(&mut restored_cx, &restored, "continued input", |v, cx| {
        v.focused_text(cx).contains("\nafter-reconnect-marker\n")
    });
    restored_cx.simulate_keystrokes(crate::platform::shortcut("cmd-w"));
    restored_cx.run_until_parked();
    let client = env.session_service.as_ref().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if client
            .state()
            .unwrap()
            .sessions
            .iter()
            .any(|r| r.pane == pane.raw() && !r.running && r.closed)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        client
            .state()
            .unwrap()
            .sessions
            .iter()
            .any(|r| r.pane != pane.raw() && r.running),
        "closing one pane must keep the other window's shell"
    );
}

#[gpui::test]
fn server_restart_shows_read_only_output_until_start_shell_is_clicked(cx: &mut TestAppContext) {
    let mut h = Harness::open_service(cx, "service-reboot", |_| {});
    h.wait_prompt();
    h.run(
        "printf '\\nreboot-saved-marker\\n'",
        "\nreboot-saved-marker\n",
    );
    let client = h.read(|v, _| v.env.session_service.clone().unwrap());
    client.checkpoint(None).unwrap();
    let before = client.state().unwrap().sessions.remove(0);
    h.restart_service();
    let (mut restored_cx, restored) = h.reopen();
    wait_until(
        &mut restored_cx,
        &restored,
        "read-only recovery pane",
        |v, _| {
            v.panes.is_empty()
                && v.ws
                    .focused_pane()
                    .and_then(|p| v.ws.pane(p))
                    .and_then(|p| p.pane_run.as_ref())
                    .is_some_and(chda_core::PaneRun::stopped)
        },
    );
    let client = restored.read_with(&restored_cx, |v, _| v.env.session_service.clone().unwrap());
    let copy = restored_cx.debug_bounds("previous-copy").unwrap();
    restored_cx.simulate_click(copy.center(), Modifiers::default());
    restored_cx.update(|_, cx| {
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("reboot-saved-marker")
        )
    });
    assert!(client.state().unwrap().sessions.iter().all(|r| !r.running));
    restored_cx.simulate_keystrokes("tab shift-tab space");
    restored_cx.run_until_parked();
    restored_cx.update(|_, cx| {
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("reboot-saved-marker")
        )
    });
    assert!(client.state().unwrap().sessions.iter().all(|r| !r.running));
    restored_cx.simulate_input("echo no-automatic-input");
    restored_cx.simulate_keystrokes("enter");
    restored_cx.run_until_parked();
    assert!(client.state().unwrap().sessions.iter().all(|r| !r.running));
    let bounds = restored_cx.debug_bounds("start-shell").unwrap();
    restored_cx.simulate_click(bounds.center(), Modifiers::default());
    restored_cx.simulate_click(bounds.center(), Modifiers::default());
    wait_until(
        &mut restored_cx,
        &restored,
        "explicit new shell",
        |v, cx| v.focused_prompt_ready(cx),
    );
    let records = client.state().unwrap().sessions;
    assert_eq!(records.len(), 1);
    assert!(records[0].running);
    assert!(records[0].launch.is_none());
    assert_ne!(records[0].identity.run, before.identity.run);
    assert_eq!(records[0].identity.session, before.identity.session);
}

#[gpui::test]
fn exact_resume_and_missing_transcript_do_not_start_another_conversation(cx: &mut TestAppContext) {
    let mut h = Harness::open_service(cx, "service-resume", |home| {
        home.fake_claude();
        home.claude_session(&home.home, "exact-service-session", "fixture");
        std::fs::write(&home.claude_bin,"#!/bin/sh\nprintf 'launch %s\\n' \"$*\"\nprintf 'run\\n' >> \"$HOME/cli-runs\"\nexec cat >/dev/null\n").unwrap();
    });
    h.wait_prompt();
    h.cx.dispatch_action(crate::workspace_view::RunClaude);
    h.cx.run_until_parked();
    h.confirm_agent_launch();
    h.wait_for("managed CLI", |v, cx| {
        v.focused_text(cx).contains("launch --settings")
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let cwd = h.home.home.clone();
    h.hook_session(
        Some(pane.raw()),
        &cwd,
        chda_core::agents::HookKind::SessionStart,
        "exact-service-session",
    );
    h.wait_for("exact ID", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_launch
            .as_ref()
            .is_some_and(|c| {
                c.session
                    .as_ref()
                    .is_some_and(|s| s.0 == "exact-service-session")
            })
    });
    h.read(|v, _| {
        v.env
            .session_service
            .as_ref()
            .unwrap()
            .checkpoint(None)
            .unwrap()
    });
    h.restart_service();
    let (mut restored_cx, restored) = h.reopen();
    wait_until(
        &mut restored_cx,
        &restored,
        "stopped exact conversation",
        |v, _| {
            v.ws.pane(pane)
                .unwrap()
                .pane_run
                .as_ref()
                .is_some_and(chda_core::PaneRun::stopped)
        },
    );
    let transcript = h
        .home
        .claude_projects
        .join("project/exact-service-session.jsonl");
    let bytes = std::fs::read(&transcript).unwrap();
    std::fs::remove_file(&transcript).unwrap();
    let bounds = restored_cx.debug_bounds("agent-restart").unwrap();
    restored_cx.simulate_click(bounds.center(), Modifiers::default());
    restored_cx.run_until_parked();
    assert!(restored.read_with(&restored_cx, |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .pane_run
            .as_ref()
            .unwrap()
            .label()
            .contains("transcript is missing")
    }));
    assert_eq!(
        std::fs::read_to_string(cwd.join("cli-runs"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    std::fs::write(&transcript, bytes).unwrap();
    let bounds = restored_cx.debug_bounds("agent-restart").unwrap();
    restored_cx.simulate_click(bounds.center(), Modifiers::default());
    wait_until(
        &mut restored_cx,
        &restored,
        "same conversation resume",
        |v, cx| {
            v.focused_text(cx)
                .contains("--resume exact-service-session")
        },
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("cli-runs"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}
