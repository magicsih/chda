//! Scenarios verifying that parent and child agent statuses are distinctly
//! presented and cannot be confused with one another (#313).

use super::harness::Harness;
use chda_core::{
    AgentStatus,
    agents::{ChildEvent, ChildState, HookEvent, HookKind, ipc},
};
use gpui::TestAppContext;

fn send_child(
    h: &Harness,
    agent: &str,
    pane: u64,
    parent: &str,
    child_id: &str,
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
                id: child_id.into(),
                label: Some("Task".into()),
                state,
                started: state == ChildState::Working,
            }),
        },
    )
    .unwrap();
}

#[gpui::test]
fn parent_working_idle_and_turn_complete_distinct_from_child_working(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "parent-child-distinct-states", |_| {});
    h.wait_prompt();
    let parent = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pane_raw = parent.raw();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };

    // 1. Parent idle with no children
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::SessionStart, "p1");
    h.wait_for("parent idle", |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
        rows.first()
            .is_some_and(|r| r.status == Some(AgentStatus::Idle))
    });
    assert!(
        h.cx.debug_bounds(leak(format!("children-{pane_raw}")))
            .is_none()
    );

    // 2. Child starts working while parent is idle:
    // Parent remains Idle, child indicator shows ChildWorking.
    let at = crate::terminal_view::now_ms();
    send_child(&h, "claude", pane_raw, "p1", "c1", ChildState::Working, at);
    h.wait_for("child working", |v, cx| {
        v.sidebar
            .read(cx)
            .child_groups
            .first()
            .is_some_and(|g| g.children.len() == 1)
    });

    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
    }

    // Parent is still Idle, NOT Working!
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).model.sessions[0].status),
        Some(AgentStatus::Idle)
    );
    let counts = h.read(|v, cx| {
        crate::sidebar_view::ChildCounts::of(&v.sidebar.read(cx).child_groups[0].children)
    });
    assert_eq!(counts.working, 1);
    assert!(
        h.cx.debug_bounds(leak(format!("children-{pane_raw}")))
            .is_some()
    );

    // 3. Parent also starts working: now BOTH are working
    h.hook_session(
        Some(pane_raw),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("parent working", |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
        rows.first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).model.sessions[0].status),
        Some(AgentStatus::Working)
    );

    // 4. Parent finishes turn while child continues working
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::Stopped, "p1");
    h.wait_for("parent turn complete", |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
        rows.first()
            .is_some_and(|r| r.status == Some(AgentStatus::Review))
    });

    // Parent status is Review (Turn complete), NOT Working.
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).model.sessions[0].status),
        Some(AgentStatus::Review)
    );
    let counts = h.read(|v, cx| {
        crate::sidebar_view::ChildCounts::of(&v.sidebar.read(cx).child_groups[0].children)
    });
    assert_eq!(counts.working, 1);

    // 5. Child finishes working (neither working)
    send_child(
        &h,
        "claude",
        pane_raw,
        "p1",
        "c1",
        ChildState::Ended,
        at + 10,
    );
    h.wait_for("child ended", |v, cx| {
        let counts =
            crate::sidebar_view::ChildCounts::of(&v.sidebar.read(cx).child_groups[0].children);
        counts.working == 0
    });

    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).model.sessions[0].status),
        Some(AgentStatus::Review)
    );
}

#[gpui::test]
fn parent_and_child_waiting_and_unknown_states_remain_distinct(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "parent-child-waiting-unknown", |_| {});
    h.wait_prompt();
    let parent = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pane_raw = parent.raw();

    // Parent is waiting for input
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::WaitingInput, "p1");
    h.wait_for("parent waiting", |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
        rows.first()
            .is_some_and(|r| r.status == Some(AgentStatus::WaitingInput))
    });

    let at = crate::terminal_view::now_ms();
    // Child 1 is working, Child 2 is waiting for input, Child 3 is unknown
    send_child(&h, "claude", pane_raw, "p1", "c1", ChildState::Working, at);
    send_child(
        &h,
        "claude",
        pane_raw,
        "p1",
        "c2",
        ChildState::WaitingInput,
        at + 1,
    );
    send_child(
        &h,
        "claude",
        pane_raw,
        "p1",
        "c3",
        ChildState::Unknown,
        at + 2,
    );

    h.wait_for("three children", |v, cx| {
        v.sidebar
            .read(cx)
            .child_groups
            .first()
            .is_some_and(|g| g.children.len() == 3)
    });

    // Parent status must NOT be replaced or overridden by child states
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).model.sessions[0].status),
        Some(AgentStatus::WaitingInput)
    );

    let counts = h.read(|v, cx| {
        crate::sidebar_view::ChildCounts::of(&v.sidebar.read(cx).child_groups[0].children)
    });
    assert_eq!(counts.working, 1);
    assert_eq!(counts.waiting, 1);
    assert_eq!(counts.unknown, 1);
    assert_eq!(counts.total, 3);

    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
    }

    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    assert!(
        h.cx.debug_bounds(leak(format!("children-{pane_raw}")))
            .is_some()
    );
}

#[gpui::test]
fn multiple_parents_in_same_worktree_maintain_isolated_child_indicators(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "multiple-parents-isolated", |_| {});
    h.wait_prompt();
    let p1 = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(Some(p1.raw()), &h.home.home, HookKind::SessionStart, "p1");

    // Open second terminal tab in same worktree
    h.keys("cmd-t");
    h.wait_prompt();
    let p2 = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(Some(p2.raw()), &h.home.home, HookKind::SessionStart, "p2");

    let at = crate::terminal_view::now_ms();
    // Parent 1 has working child
    send_child(&h, "claude", p1.raw(), "p1", "c1", ChildState::Working, at);
    // Parent 2 has waiting child
    send_child(
        &h,
        "claude",
        p2.raw(),
        "p2",
        "c2",
        ChildState::WaitingInput,
        at + 1,
    );

    h.wait_for("both child groups present", |v, cx| {
        let groups = &v.sidebar.read(cx).child_groups;
        groups.len() == 2
    });

    let counts_p1 = h.read(|v, cx| {
        let g = v
            .sidebar
            .read(cx)
            .child_groups
            .iter()
            .find(|g| g.pane == p1)
            .unwrap()
            .clone();
        crate::sidebar_view::ChildCounts::of(&g.children)
    });
    assert_eq!(counts_p1.working, 1);
    assert_eq!(counts_p1.waiting, 0);

    let counts_p2 = h.read(|v, cx| {
        let g = v
            .sidebar
            .read(cx)
            .child_groups
            .iter()
            .find(|g| g.pane == p2)
            .unwrap()
            .clone();
        crate::sidebar_view::ChildCounts::of(&g.children)
    });
    assert_eq!(counts_p2.working, 0);
    assert_eq!(counts_p2.waiting, 1);
}
