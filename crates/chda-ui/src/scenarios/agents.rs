//! Agent status per pane: tab dots, the ACTIVE list, the Dock badge,
//! notifications, "go to waiting agent" and notification clicks.

use chda_core::AgentStatus;
use chda_core::agents::HookKind;
use gpui::TestAppContext;

use super::harness::Harness;

#[gpui::test]
fn hook_events_drive_status_badge_jump_and_notification_click(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "agents", |_| {});
    h.wait_prompt();
    h.keys("cmd-t");
    h.wait_prompt();
    let (first, second) = h.read(|v, _| {
        let tabs = v.ws.tabs();
        (tabs[0].panes()[0], tabs[1].panes()[0])
    });
    let home = h.home.home.clone();

    h.hook(Some(first.raw()), &home, HookKind::PromptSubmitted);
    h.hook(Some(first.raw()), &home, HookKind::WaitingInput);
    h.wait_for("the first tab to wait for input", move |v, _| {
        v.ws.tab_agent(&v.ws.tabs()[0])
            .is_some_and(|a| a.status == AgentStatus::WaitingInput)
    });
    h.read(|v, cx| {
        let active = &v.sidebar.read(cx).model.active_tabs;
        let row = active.iter().find(|t| t.tab == v.ws.tabs()[0].id).unwrap();
        assert_eq!(row.status, AgentStatus::WaitingInput, "ACTIVE list agrees");
    });
    assert_eq!(h.system.0.borrow().badge, 1, "one unseen waiting agent");
    {
        let recorded = h.system.0.borrow();
        let (title, _, target) = recorded.notifications.last().unwrap();
        assert!(title.contains("is waiting"), "{title}");
        assert_eq!(target.pane, Some(first.raw()));
    }

    // Going to the waiting agent focuses its pane and clears the badge.
    h.keys("cmd-shift-a");
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(first));
    assert_eq!(h.system.0.borrow().badge, 0);

    // A finished turn in the other tab is reviewed by clicking the
    // notification for it.
    h.hook(Some(second.raw()), &home, HookKind::Stopped);
    h.wait_for("the second tab to finish", move |v, _| {
        v.ws.pane(second)
            .and_then(|p| p.agent.as_ref())
            .is_some_and(|a| a.status == AgentStatus::Review)
    });
    let target = h.system.0.borrow().notifications.last().unwrap().2.clone();
    let click = h.system.0.borrow_mut().click.take().unwrap();
    click(target);
    h.wait_for("the notification to focus the second pane", move |v, _| {
        v.ws.focused_pane() == Some(second) && v.ws.pane(second).unwrap().agent.is_none()
    });
}

#[gpui::test]
fn gemini_copilot_and_opencode_report_like_claude_code(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "moreagents", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.tabs()[0].panes()[0]);
    let home = h.home.home.clone();
    for (agent, name) in [
        ("gemini", "Gemini CLI"),
        ("copilot", "Copilot CLI"),
        ("opencode", "OpenCode"),
    ] {
        h.hook_from(
            agent,
            Some(pane.raw()),
            &home,
            HookKind::PromptSubmitted,
            "s",
        );
        h.wait_for("the agent to work", move |v, _| {
            v.ws.pane(pane)
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.agent == agent && a.status == AgentStatus::Working)
        });
        h.hook_from(agent, Some(pane.raw()), &home, HookKind::WaitingInput, "s");
        h.wait_for("the agent to wait", move |v, _| {
            v.ws.pane(pane)
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.status == AgentStatus::WaitingInput)
        });
        let title = h.system.0.borrow().notifications.last().unwrap().0.clone();
        assert_eq!(title, format!("{name} is waiting"));
        h.hook_from(agent, Some(pane.raw()), &home, HookKind::SessionEnd, "s");
        h.wait_for("the session to end", move |v, _| {
            v.ws.pane(pane).is_some_and(|p| p.agent.is_none())
        });
    }
}
