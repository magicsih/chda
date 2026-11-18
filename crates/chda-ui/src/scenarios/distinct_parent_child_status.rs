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

#[gpui::test]
fn concentric_status_reflects_working_and_waiting_children_across_sessions_tabs_and_worktrees(
    cx: &mut TestAppContext,
) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "concentric-child-status", |home| {
        repo = home.repo("app");
        std::fs::write(
            &home.config,
            format!("repos = [{}]\n", serde_json::to_string(&repo).unwrap()),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.run(
        &format!("cd {}", chda_term::shell_quote(&repo.to_string_lossy())),
        "test%",
    );
    let repo_ready = repo.clone();
    h.wait_for("pane cwd enters repo", move |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .and_then(|p| p.cwd.as_ref())
            == Some(&repo_ready)
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pane_raw = pane.raw();

    h.hook_session(Some(pane_raw), &repo, HookKind::SessionStart, "p1");
    h.wait_for("parent idle in repo", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Idle))
    });
    super::sidebar::draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("concentric-child-working").is_none());
    assert!(h.cx.debug_bounds("concentric-child-waiting").is_none());
    let status_selector: &'static str =
        Box::leak(format!("session-status-pane-{pane_raw}").into_boxed_str());
    let row_selector: &'static str = Box::leak(format!("session-pane-{pane_raw}").into_boxed_str());
    let slot_size = h.cx.debug_bounds(status_selector).unwrap().size;
    let row_size = h.cx.debug_bounds(row_selector).unwrap().size;

    // 1. Child starts working while parent is Idle: concentric working ring appears
    // across Sessions row, tab bar, and worktree dot without changing slot/row size.
    let at = crate::terminal_view::now_ms();
    send_child(&h, "claude", pane_raw, "p1", "c1", ChildState::Working, at);
    h.wait_for("child working registered", |v, cx| {
        v.sidebar
            .read(cx)
            .child_groups
            .first()
            .is_some_and(|g| g.children.len() == 1)
    });
    super::sidebar::draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("concentric-child-working").is_some());
    assert!(h.cx.debug_bounds("concentric-parent-dot").is_some());
    assert_eq!(h.cx.debug_bounds(status_selector).unwrap().size, slot_size);
    assert_eq!(h.cx.debug_bounds(row_selector).unwrap().size, row_size);
    h.read(|v, cx| {
        let (tab_ring, tab_lines) = v.tab_child_status(&v.ws.tabs()[0]);
        assert_eq!(tab_ring, Some(AgentStatus::Working));
        assert_eq!(tab_lines, vec!["Claude Code: 1 child working.".to_owned()]);

        let wt = &v.sidebar.read(cx).model.repos[0].worktrees[0];
        let (wt_dot, wt_ring, wt_tip) = v.sidebar.read(cx).status_dot(wt);
        assert_eq!(wt_dot, Some(AgentStatus::Idle));
        assert_eq!(wt_ring, Some(AgentStatus::Working));
        assert!(wt_tip.contains("Claude Code: 1 child working."), "{wt_tip}");
        assert!(!wt_tip.contains("ready for another task"), "{wt_tip}");
    });

    // 2. Parent finishes turn (Review) while child still works: outer ring remains Working.
    h.hook_session(Some(pane_raw), &repo, HookKind::Stopped, "p1");
    h.wait_for("parent turn complete", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Review))
    });
    super::sidebar::draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("concentric-child-working").is_some());
    h.read(|v, cx| {
        let wt = &v.sidebar.read(cx).model.repos[0].worktrees[0];
        let (wt_dot, wt_ring, wt_tip) = v.sidebar.read(cx).status_dot(wt);
        assert_eq!(wt_dot, Some(AgentStatus::Review));
        assert_eq!(wt_ring, Some(AgentStatus::Working));
        assert!(wt_tip.contains("Claude Code: 1 child working."), "{wt_tip}");
    });

    // 3. Parent also works while child works: concentric ring stays instead of wave.
    h.hook_session(Some(pane_raw), &repo, HookKind::PromptSubmitted, "p1");
    h.wait_for("parent working", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });
    super::sidebar::draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("concentric-child-working").is_some());
    assert!(h.cx.debug_bounds("working-wave").is_none());

    // 4. Another child waits for input: outer ring upgrades to WaitingInput.
    send_child(
        &h,
        "claude",
        pane_raw,
        "p1",
        "c2",
        ChildState::WaitingInput,
        at + 1,
    );
    h.wait_for("waiting child present", |v, cx| {
        v.sidebar
            .read(cx)
            .child_groups
            .first()
            .is_some_and(|g| g.children.len() == 2)
    });
    super::sidebar::draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("concentric-child-waiting").is_some());
    assert!(h.cx.debug_bounds("concentric-child-working").is_none());
    h.read(|v, cx| {
        let (tab_ring, _) = v.tab_child_status(&v.ws.tabs()[0]);
        assert_eq!(tab_ring, Some(AgentStatus::WaitingInput));
        let wt = &v.sidebar.read(cx).model.repos[0].worktrees[0];
        let (_, wt_ring, _) = v.sidebar.read(cx).status_dot(wt);
        assert_eq!(wt_ring, Some(AgentStatus::WaitingInput));
    });

    // 5. Both children finish: concentric ring clears and parent's working wave returns.
    send_child(
        &h,
        "claude",
        pane_raw,
        "p1",
        "c1",
        ChildState::Ended,
        at + 2,
    );
    send_child(
        &h,
        "claude",
        pane_raw,
        "p1",
        "c2",
        ChildState::Ended,
        at + 3,
    );
    h.wait_for("all children ended", |v, cx| {
        let counts =
            crate::sidebar_view::ChildCounts::of(&v.sidebar.read(cx).child_groups[0].children);
        counts.working == 0 && counts.waiting == 0
    });
    super::sidebar::draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("concentric-child-working").is_none());
    assert!(h.cx.debug_bounds("concentric-child-waiting").is_none());
    assert!(h.cx.debug_bounds("working-wave").is_some());
    h.read(|v, cx| {
        assert_eq!(v.tab_child_status(&v.ws.tabs()[0]), (None, Vec::new()));
        let wt = &v.sidebar.read(cx).model.repos[0].worktrees[0];
        let (wt_dot, wt_ring, _) = v.sidebar.read(cx).status_dot(wt);
        assert_eq!(wt_dot, Some(AgentStatus::Working));
        assert_eq!(wt_ring, None);
    });
}
