//! Nested status is bound to exact provider/session identities, never cwd.
use super::harness::{Harness, wait_until};
use chda_core::agents::{ChildEvent, ChildState, HookEvent, HookKind, ipc};
use gpui::{TestAppContext, VisualContext};

fn send(h: &Harness, pane: u64, parent: &str, id: &str, state: ChildState, at: u64) {
    ipc::send(
        &ipc::socket_path(&h.home.data),
        &HookEvent {
            agent: "claude".into(),
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
    let child_selector: &'static str =
        Box::leak(format!("child-{}-child1", first.raw()).into_boxed_str());
    let toggle_selector: &'static str =
        Box::leak(format!("children-toggle-{}", first.raw()).into_boxed_str());
    let row = h.cx.debug_bounds(child_selector).unwrap();
    h.cx.simulate_click(row.center(), gpui::Modifiers::default());
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(second));
    assert!(h.cx.debug_bounds("child-details").is_some());
    h.keys("escape");
    assert!(h.read(|v, _| v.child_detail.is_none()));
    let toggle = h.cx.debug_bounds(toggle_selector).unwrap();
    h.cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    assert!(h.cx.debug_bounds(child_selector).is_none());
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
    let handle = h.cx.update(|_, cx| {
        crate::open_workspace_window(chda_config::load(&env.ghostty, None), None, env.clone(), cx)
    });
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
    let selector: &'static str =
        Box::leak(format!("child-{}-dedicated-child", parent.raw()).into_boxed_str());
    let row = second_cx.debug_bounds(selector).unwrap();
    second_cx.simulate_click(row.center(), gpui::Modifiers::default());
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
