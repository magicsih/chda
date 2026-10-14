//! Nested status is bound to exact provider/session identities, never cwd.
use super::harness::{Harness, wait_until};
use chda_core::agents::{ChildEvent, ChildState, HookEvent, HookKind, ipc};
use gpui::{TestAppContext, VisualContext};

/// Right-click a parent's child summary, as on a real Sessions row.
fn child_menu(cx: &mut gpui::VisualTestContext, parent: chda_core::PaneId) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    let selector: &'static str = Box::leak(format!("children-{}", parent.raw()).into_boxed_str());
    let at = cx.debug_bounds(selector).expect(selector).center();
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
    cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
    cx.run_until_parked();
}

fn send(h: &Harness, pane: u64, parent: &str, id: &str, state: ChildState, at: u64) {
    send_from(h, "claude", pane, parent, id, state, at);
}

fn send_from(
    h: &Harness,
    agent: &str,
    pane: u64,
    parent: &str,
    id: &str,
    state: ChildState,
    at: u64,
) {
    ipc::send(
        &ipc::socket_path(&h.home.data),
        &HookEvent {
            agent: agent.into(),
            session_id: parent.into(),
            cwd: h.home.home.clone(),
            kind: HookKind::Stopped,
            timestamp: at,
            pane: Some(pane),
            child: Some(ChildEvent {
                id: id.into(),
                label: Some("Explore".into()),
                state,
                started: state == ChildState::Working,
            }),
        },
    )
    .unwrap();
}

#[gpui::test]
fn children_remain_under_their_parent_after_a_turn_and_never_focus_a_shared_directory(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "nested-children", |_| {});
    h.wait_prompt();
    let first = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(first.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "p1",
    );
    h.wait_for("first parent identity", |v, _| {
        v.ws.pane(first)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "p1")
    });
    h.keys("cmd-t");
    h.wait_prompt();
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(second.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "p2",
    );
    h.wait_for("second parent identity", |v, _| {
        v.ws.pane(second)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "p2")
    });
    let at = crate::terminal_view::now_ms();
    // The inherited pane hint deliberately names the other conversation.
    send(&h, second.raw(), "p1", "child1", ChildState::Working, at);
    send(
        &h,
        first.raw(),
        "p2",
        "child2",
        ChildState::WaitingInput,
        at + 1,
    );
    h.wait_for("two exact child groups", |v, cx| {
        v.sidebar.read(cx).child_groups.len() == 2
    });
    h.hook_session(Some(first.raw()), &h.home.home, HookKind::Stopped, "p1");
    h.wait_for("parent turn settled", |v, _| {
        v.ws.pane(first).unwrap().agent.as_ref().unwrap().status == chda_core::AgentStatus::Review
    });
    h.read(|v, cx| {
        let groups = &v.sidebar.read(cx).child_groups;
        assert_eq!(
            groups.iter().find(|g| g.pane == first).unwrap().children[0]
                .child
                .state,
            ChildState::Working
        );
        assert_eq!(
            groups.iter().find(|g| g.pane == second).unwrap().children[0]
                .child
                .state,
            ChildState::WaitingInput
        );
    });
    // One line per parent: the summary sits on the parent's own row (#246).
    let row =
        h.cx.debug_bounds(Box::leak(
            format!("session-pane-{}", first.raw()).into_boxed_str(),
        ))
        .unwrap();
    let summary =
        h.cx.debug_bounds(Box::leak(
            format!("children-{}", first.raw()).into_boxed_str(),
        ))
        .unwrap();
    assert!(row.top() <= summary.top() && summary.bottom() <= row.bottom());
    // Right-click lists the children; choosing one shows its details.
    child_menu(&mut h.cx, first);
    let item = h.read(|v, _| v.context_menu.as_ref().unwrap().items[0].0.clone());
    assert!(item.starts_with("Explore \u{2014} Working"), "{item}");
    let item_at = h.cx.debug_bounds("menu-item-0").unwrap().center();
    h.cx.simulate_click(item_at, gpui::Modifiers::default());
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(second));
    assert!(h.cx.debug_bounds("child-details").is_some());
    h.keys("escape");
    assert!(h.read(|v, _| v.child_detail.is_none()));
    send(
        &h,
        first.raw(),
        "p1",
        "child1",
        ChildState::TurnComplete,
        at + 2,
    );
    send(&h, first.raw(), "p1", "child1", ChildState::Working, at - 1);
    h.wait_for("late event cannot revive child", |v, _| {
        v.env.windows.borrow().children.children("claude", "p1")[0]
            .child
            .state
            == ChildState::TurnComplete
    });
    send(&h, first.raw(), "p1", "child1", ChildState::Ended, at + 3);
    h.wait_for("confirmed end", |v, _| {
        v.env.windows.borrow().children.children("claude", "p1")[0]
            .child
            .state
            == ChildState::Ended
    });
    let notifications = h.read(|v, _| v.notifications.newest().len());
    send(
        &h,
        second.raw(),
        "p2",
        "child2",
        ChildState::WaitingInput,
        at + 4,
    );
    // Process a subsequent top-level event as a routing barrier.
    h.hook_session(
        Some(second.raw()),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p2",
    );
    h.wait_for("parent routing barrier", |v, _| {
        v.ws.pane(second).unwrap().agent.as_ref().unwrap().status == chda_core::AgentStatus::Working
    });
    assert_eq!(h.read(|v, _| v.notifications.newest().len()), notifications);
    let (cold, view) = h.reopen();
    cold.run_until_parked();
    assert!(view.read_with(&cold, |v, cx| v.sidebar.read(cx).child_groups.is_empty()));
}

#[gpui::test]
fn children_route_to_exact_parents_across_windows_and_focus_only_a_known_child_pane(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "child-windows", |_| {});
    h.wait_prompt();
    let original = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(original.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "p1",
    );
    h.wait_for("original parent", |v, _| {
        v.ws.pane(original).unwrap().agent_session.is_some()
    });
    let env = h.read(|v, _| v.env.clone());
    let handle =
        h.cx.update(|_, cx| crate::open_workspace_window(None, env.clone(), cx));
    let mut second_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let second = handle.root(&mut second_cx).unwrap();
    wait_until(&mut second_cx, &second, "second shell", |v, cx| {
        v.focused_text(cx).contains("test%")
    });
    let parent = second.read_with(&second_cx, |v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(parent.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "p2",
    );
    wait_until(&mut second_cx, &second, "second parent", |v, _| {
        v.ws.pane(parent).unwrap().agent_session.is_some()
    });
    let at = crate::terminal_view::now_ms();
    send(
        &h,
        original.raw(),
        "p2",
        "dedicated-child",
        ChildState::Working,
        at,
    );
    wait_until(
        &mut second_cx,
        &second,
        "child follows exact parent in another window",
        |v, cx| {
            v.sidebar
                .read(cx)
                .child_groups
                .iter()
                .any(|g| g.session == "p2")
        },
    );
    assert!(h.read(|v, cx| v.sidebar.read(cx).child_groups.is_empty()));
    // This pane has the child's actual session identity, not merely its cwd.
    h.hook_session(
        Some(original.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "dedicated-child",
    );
    h.wait_for("known dedicated child", |v, _| {
        v.ws.pane(original)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "dedicated-child")
    });
    child_menu(&mut second_cx, parent);
    let at = second_cx.debug_bounds("menu-item-0").unwrap().center();
    second_cx.simulate_click(at, gpui::Modifiers::default());
    h.cx.run_until_parked();
    assert_eq!(
        env.windows.borrow().active,
        Some(h.cx.window_handle().window_id())
    );
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(original));
    assert!(second.read_with(&second_cx, |v, _| v.child_detail.is_none()));
    assert_eq!(
        second.read_with(&second_cx, |v, _| v.ws.focused_pane()),
        Some(parent)
    );
}

#[gpui::test]
fn codex_poll_failure_keeps_child_states(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::open(cx, "child-poll-failure", |home| {
        // The CLI is installed, but its local metadata query fails.
        std::fs::create_dir_all(&home.bin).unwrap();
        let codex = home.bin.join("codex");
        std::fs::write(
            &codex,
            "#!/bin/sh\necho poll >> \"$(dirname \"$0\")/polls\"\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Codex counts as installed only when the stand-in is on the PATH.
        std::fs::write(
            home.home.join("gui-path"),
            format!("{}:/usr/bin:/bin", home.bin.display()),
        )
        .unwrap();
    });
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_from(
        "codex",
        Some(pane.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "p1",
    );
    h.wait_for("live Codex parent", |v, _| {
        v.env
            .windows
            .borrow()
            .entries
            .iter()
            .flat_map(|e| e.sessions.values())
            .any(|(agent, session, live)| agent == "codex" && session == "p1" && *live)
    });
    // Earlier than the poll, so only the failure handling can keep them.
    let at = crate::terminal_view::now_ms() - 10;
    for (id, state) in [
        ("running", ChildState::Working),
        ("finished", ChildState::TurnComplete),
    ] {
        send_from(&h, "codex", pane.raw(), "p1", id, state, at);
    }
    let states = |v: &crate::workspace_view::WorkspaceView, cx: &gpui::App| {
        v.sidebar
            .read(cx)
            .child_groups
            .iter()
            .flat_map(|g| g.children.iter())
            .map(|c| (c.child.id.clone(), c.child.state))
            .collect::<Vec<_>>()
    };
    let expected = vec![
        ("running".to_owned(), ChildState::Working),
        ("finished".to_owned(), ChildState::TurnComplete),
    ];
    h.wait_for("both children reported", |v, cx| states(v, cx) == expected);
    h.cx.update(|_, cx| h.view.update(cx, |v, cx| v.poll_children(cx)));
    assert!(h.read(|v, _| v.env.windows.borrow().child_polling));
    h.wait_for("failed poll settled", |v, _| {
        !v.env.windows.borrow().child_polling
    });
    assert_eq!(
        std::fs::read_to_string(h.home.bin.join("polls"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(h.read(states), expected);
}

/// Each parent's children take no extra line: a count per state on the
/// parent's row, also in a narrow sidebar (#246).
#[gpui::test]
fn child_states_are_counted_on_the_parent_row(cx: &mut TestAppContext) {
    use crate::sidebar_view::ChildCounts;
    let mut h = Harness::open(cx, "child-summary", |home| {
        std::fs::write(&home.config, "sidebar-width = 180\n").unwrap();
    });
    h.wait_prompt();
    let parent = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(parent.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "p1",
    );
    h.keys("cmd-t");
    h.wait_prompt();
    let other = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(other.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "p2",
    );
    let at = crate::terminal_view::now_ms();
    for (i, state) in [
        ChildState::Working,
        ChildState::WaitingInput,
        ChildState::Unknown,
        ChildState::Ended,
    ]
    .into_iter()
    .enumerate()
    {
        send(
            &h,
            parent.raw(),
            "p1",
            &format!("c{i}"),
            state,
            at + i as u64,
        );
    }
    h.wait_for("four children", |v, cx| {
        v.sidebar
            .read(cx)
            .child_groups
            .first()
            .is_some_and(|g| g.children.len() == 4)
    });
    let counts = h.read(|v, cx| ChildCounts::of(&v.sidebar.read(cx).child_groups[0].children));
    assert_eq!(
        counts,
        ChildCounts {
            working: 1,
            waiting: 1,
            unknown: 1,
            total: 4
        }
    );
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let row =
        h.cx.debug_bounds(leak(format!("session-pane-{}", parent.raw())))
            .unwrap();
    let summary =
        h.cx.debug_bounds(leak(format!("children-{}", parent.raw())))
            .unwrap();
    assert!(row.top() <= summary.top() && summary.bottom() <= row.bottom());
    assert!(summary.right() <= row.right());
    let label =
        h.cx.debug_bounds(leak(format!("session-label-pane-{}", parent.raw())))
            .unwrap();
    assert!(label.size.width >= gpui::px(47.5), "{label:?}");
    // A parent without children gets nothing extra.
    assert!(
        h.cx.debug_bounds(leak(format!("children-{}", other.raw())))
            .is_none()
    );
    // Every child, with its state, is one right-click away.
    child_menu(&mut h.cx, parent);
    let items: Vec<String> = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|(text, _)| text.clone())
            .collect()
    });
    assert_eq!(items.len(), 4);
    assert!(
        items.iter().any(|i| i.contains("Waiting for input")),
        "{items:?}"
    );
    assert!(items.iter().any(|i| i.contains("Ended")), "{items:?}");
    h.keys("escape");
    assert!(h.read(|v, _| v.context_menu.is_none()));
}
