//! Number keys follow the sidebar's Sessions rows, including split agents.
use super::harness::{Harness, wait_until};
use crate::sidebar_view::SidebarEvent;
use chda_core::{SessionKey, agents::HookKind};
use gpui::{TestAppContext, VisualTestContext};

fn rows(h: &Harness) -> Vec<SessionKey> {
    h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .map(|r| r.key)
            .collect()
    })
}

fn assert_focused(h: &Harness, expected: SessionKey) {
    h.read(|v, _| match expected {
        SessionKey::Pane(pane) => assert_eq!(v.ws.focused_pane(), Some(pane)),
        SessionKey::Tab(tab) => assert_eq!(v.ws.active_tab().map(|t| t.id), Some(tab)),
    });
}

#[gpui::test]
fn number_shortcuts_follow_mixed_sessions_across_repositories_and_splits(cx: &mut TestAppContext) {
    let mut repos = Vec::new();
    let mut h = Harness::open(cx, "session-number-mixed", |home| {
        repos = vec![home.repo("alpha"), home.repo("beta")];
        std::fs::write(
            &home.config,
            format!("repos = {}\n", serde_json::to_string(&repos).unwrap()),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for("both repositories", |v, cx| {
        repos
            .iter()
            .all(|p| v.sidebar.read(cx).model.worktree_for_path(p).is_some())
    });
    let home = h.read(|v, _| v.ws.active_tab().unwrap().id);
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(repos[0].clone()), window, cx)
        })
    });
    h.wait_prompt();
    let first = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.keys("cmd-d");
    h.wait_prompt();
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook(Some(first.raw()), &repos[0], HookKind::PromptSubmitted);
    h.hook(Some(second.raw()), &repos[0], HookKind::WaitingInput);
    h.wait_for("split agent rows", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .filter(|r| matches!(r.key, SessionKey::Pane(_)))
            .count()
            == 2
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(repos[1].clone()), window, cx)
        })
    });
    h.wait_prompt();
    let beta = h.read(|v, _| v.ws.active_tab().unwrap().id);
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.open_git_graph(repos[1].clone(), window, cx))
    });
    let graph = h.read(|v, _| v.ws.active_tab().unwrap().id);
    let expected = vec![
        SessionKey::Pane(first),
        SessionKey::Pane(second),
        SessionKey::Tab(graph),
        SessionKey::Tab(home),
        SessionKey::Tab(beta),
    ];
    assert_eq!(rows(&h), expected);
    for (i, key) in expected.iter().enumerate() {
        h.keys(&format!("cmd-{}", i + 1));
        assert_focused(&h, *key);
    }
    h.hook(Some(second.raw()), &repos[0], HookKind::Stopped);
    h.wait_for("turn completion", |v, _| {
        v.ws.pane(second)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == chda_core::AgentStatus::Review)
    });
    assert_eq!(rows(&h), expected);
    h.keys("cmd-2");
    assert_focused(&h, SessionKey::Pane(second));

    // Navigation also works with the sidebar hidden and preserves typed input.
    h.keys("cmd-4");
    h.type_text("printf '\\nretained-session-input\\n'");
    h.keys("cmd-b cmd-1 cmd-4");
    assert_focused(&h, SessionKey::Tab(home));
    h.keys("enter");
    h.wait_for("input after switching sessions", |v, cx| {
        v.focused_text(cx).contains("\nretained-session-input\n")
    });
    h.keys("cmd-b");
    let toggle =
        h.cx.debug_bounds("sessions-section-toggle")
            .unwrap()
            .center();
    h.cx.simulate_click(toggle, gpui::Modifiers::none());
    h.cx.run_until_parked();
    assert!(h.read(|v, cx| v.sidebar.read(cx).sessions_collapsed));
    h.keys("cmd-2");
    assert_focused(&h, SessionKey::Pane(second));
    assert!(h.read(|v, cx| v.sidebar.read(cx).sessions_collapsed));
}

#[gpui::test]
fn all_ten_digits_select_exact_rows_and_missing_rows_leave_focus_and_font_alone(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "session-number-ten", |_| {});
    h.wait_prompt();
    for _ in 0..10 {
        h.keys("cmd-t");
        h.wait_prompt();
    }
    let expected = rows(&h);
    assert_eq!(expected.len(), 11);
    h.keys("cmd-=");
    let zoomed = h.read(|v, _| v.settings.font_size);
    for (i, digit) in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]
        .iter()
        .enumerate()
    {
        h.keys(&format!("cmd-{digit}"));
        assert_focused(&h, expected[i]);
        assert_eq!(h.read(|v, _| v.settings.font_size), zoomed);
    }
    // Closing rows changes numbering immediately, without a cached tab index.
    h.keys("cmd-1 cmd-w cmd-1");
    assert_focused(&h, expected[1]);
    h.keys("cmd-w cmd-1");
    assert_eq!(rows(&h).len(), 9);
    assert_focused(&h, expected[2]);
    h.keys("cmd-0");
    assert_focused(&h, expected[2]);
    assert_eq!(h.read(|v, _| v.settings.font_size), zoomed);
}

#[gpui::test]
fn number_shortcut_reconnects_a_detached_session_after_the_open_rows(cx: &mut TestAppContext) {
    let mut h = Harness::open_service(cx, "session-number-detached", |_| {});
    h.wait_prompt();
    h.run(
        "printf '\\ndetached-number-marker\\n'",
        "\ndetached-number-marker\n",
    );
    let (pane, pid, env) = h.read(|v, cx| {
        let p = v.ws.focused_pane().unwrap();
        (p, v.panes[&p].0.read(cx).child_pid(), v.env.clone())
    });
    h.keys("cmd-shift-n");
    let other = env.windows.borrow().entries.last().unwrap().clone();
    let other_view = other.view.upgrade().unwrap();
    let mut other_cx = VisualTestContext::from_window(other.window, cx);
    wait_until(&mut other_cx, &other_view, "other window shell", |v, cx| {
        v.focused_prompt_ready(cx)
    });
    h.cx.update(|window, _| window.remove_window());
    h.cx.run_until_parked();
    other_cx
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    wait_until(&mut other_cx, &other_view, "detached row", |v, cx| {
        v.sidebar
            .read(cx)
            .detached_sessions
            .iter()
            .any(|r| r.0 == pane.raw())
    });
    assert_eq!(
        other_view.read_with(&other_cx, |v, cx| v.sidebar.read(cx).model.sessions.len()),
        1
    );
    let runs = env
        .session_service
        .as_ref()
        .unwrap()
        .state()
        .unwrap()
        .sessions
        .len();
    other_cx.simulate_keystrokes(crate::platform::shortcut("cmd-2"));
    other_cx.run_until_parked();
    assert_eq!(env.windows.borrow().entries.len(), 2);
    let restored = env.windows.borrow().entries.last().unwrap().clone();
    let view = restored.view.upgrade().unwrap();
    let mut restored_cx = VisualTestContext::from_window(restored.window, cx);
    wait_until(&mut restored_cx, &view, "same detached shell", |v, cx| {
        v.panes
            .get(&pane)
            .is_some_and(|(t, _)| t.read(cx).child_pid() == pid)
            && v.focused_text(cx).contains("detached-number-marker")
    });
    assert_eq!(
        env.session_service
            .as_ref()
            .unwrap()
            .state()
            .unwrap()
            .sessions
            .len(),
        runs
    );
}
