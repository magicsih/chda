//! Scenarios verifying audible cues when an agent hands control back to the user (#310).

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
fn handoff_sound_plays_for_turn_complete_and_waiting_in_focused_pane(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "handoff-sound-focused", |_| {});
    h.wait_prompt();
    h.cx.update(|window, _| window.activate_window());
    h.cx.run_until_parked();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pane_raw = pane.raw();

    // 1. Agent starts working: no sound
    h.hook_session(
        Some(pane_raw),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("agent working", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });
    assert_eq!(h.system.0.borrow().beeps, 0);

    // 2. Agent finishes turn (Stopped -> Review) while pane is focused:
    // Looking is true, so desktop notification banner is suppressed, but handoff sound (beep) plays!
    // Since the user is already looking at the pane, reviewed_focused immediately acknowledges the turn to Idle.
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::Stopped, "p1");
    h.wait_for("agent acknowledged to idle", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Idle))
    });
    assert_eq!(
        h.system.0.borrow().notifications.len(),
        0,
        "focused pane suppresses desktop banner"
    );
    assert_eq!(
        h.system.0.borrow().beeps,
        1,
        "focused pane still plays audible cue"
    );

    // 3. User submits prompt -> Working: no sound
    h.hook_session(
        Some(pane_raw),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("agent working again", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });
    assert_eq!(h.system.0.borrow().beeps, 1);

    // 4. Agent transitions to WaitingInput:
    // On systems where notification does not play sound (e.g. test harness / Linux), beep plays directly.
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::WaitingInput, "p1");
    h.wait_for("agent waiting", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::WaitingInput))
    });
    assert_eq!(
        h.system.0.borrow().notifications.len(),
        1,
        "waiting sends desktop notification"
    );
    assert_eq!(
        h.system.0.borrow().notification_sounds,
        vec![true],
        "notification requests sound"
    );
    assert_eq!(
        h.system.0.borrow().beeps,
        2,
        "beep played when notification system doesn't play sound"
    );
}

#[gpui::test]
fn handoff_sound_avoids_double_playback_when_desktop_notification_supplies_sound(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "handoff-sound-bundled-mac", |_| {});
    h.wait_prompt();
    let p1 = h.read(|v, _| v.ws.focused_pane().unwrap());

    // Configure system as bundled macOS where desktop notification plays its own sound
    h.system.0.borrow_mut().notification_plays_sound = true;

    // Open second pane in new tab and focus it
    h.keys("cmd-t");
    h.wait_prompt();
    let p2 = h.read(|v, _| v.ws.focused_pane().unwrap());
    assert_ne!(p1, p2);

    // Agent in background pane p1 finishes turn (Stopped -> Review)
    h.hook_session(
        Some(p1.raw()),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("p1 working", |v, cx| {
        v.sidebar.read(cx).model.sessions.iter().any(|r| {
            r.key == chda_core::SessionKey::Pane(p1) && r.status == Some(AgentStatus::Working)
        })
    });

    h.hook_session(Some(p1.raw()), &h.home.home, HookKind::Stopped, "p1");
    h.wait_for("p1 turn complete", |v, cx| {
        v.sidebar.read(cx).model.sessions.iter().any(|r| {
            r.key == chda_core::SessionKey::Pane(p1) && r.status == Some(AgentStatus::Review)
        })
    });

    // Background pane sent a desktop notification with sound=true
    assert_eq!(h.system.0.borrow().notifications.len(), 1);
    assert_eq!(h.system.0.borrow().notification_sounds, vec![true]);
    // Since the desktop notification already supplies the sound, extra beep is NOT called (no double playback)
    assert_eq!(
        h.system.0.borrow().beeps,
        0,
        "no extra beep when desktop notification supplies sound"
    );
}

#[gpui::test]
fn duplicate_events_and_refreshes_do_not_replay_handoff_sound(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "handoff-sound-duplicate", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pane_raw = pane.raw();

    h.hook_session(
        Some(pane_raw),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("working", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });

    // Initial handoff
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::Stopped, "p1");
    h.wait_for("review", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Review))
    });
    assert_eq!(h.system.0.borrow().beeps, 1);

    // Duplicate Stopped hook: status already Review, must not replay sound
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::Stopped, "p1");
    h.cx.run_until_parked();
    assert_eq!(
        h.system.0.borrow().beeps,
        1,
        "duplicate Stopped did not play sound"
    );

    // Delayed Idle hook: delayed idle protection keeps Review, must not play sound
    h.hook_session(Some(pane_raw), &h.home.home, HookKind::Idle, "p1");
    h.cx.run_until_parked();
    assert_eq!(
        h.system.0.borrow().beeps,
        1,
        "delayed Idle did not play sound"
    );

    // Re-render: must not play sound
    h.view.update(&mut h.cx, |_, cx| {
        cx.notify();
    });
    h.cx.run_until_parked();
    assert_eq!(h.system.0.borrow().beeps, 1, "refresh did not play sound");
}

#[gpui::test]
fn handoff_sound_setting_operates_independently_of_desktop_banners(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "handoff-sound-settings", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pane_raw = pane.raw();

    // Case 1: notifications = false, handoff_sound = true
    // Audible cue must play even when desktop banners are completely disabled
    h.view.update(&mut h.cx, |v, _| {
        v.config.notifications = false;
        v.config.handoff_sound = true;
    });

    h.hook_session(
        Some(pane_raw),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("working 1", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });

    h.hook_session(Some(pane_raw), &h.home.home, HookKind::WaitingInput, "p1");
    h.wait_for("waiting 1", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::WaitingInput))
    });
    assert_eq!(
        h.system.0.borrow().notifications.len(),
        0,
        "no banner when notifications disabled"
    );
    assert_eq!(
        h.system.0.borrow().beeps,
        1,
        "audible cue plays even when notifications disabled"
    );

    // Case 2: notifications = true, handoff_sound = false
    // Desktop banner must be sent, but sound must be false and no beep
    h.view.update(&mut h.cx, |v, _| {
        v.config.notifications = true;
        v.config.handoff_sound = false;
    });

    h.hook_session(
        Some(pane_raw),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("working 2", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });

    h.hook_session(Some(pane_raw), &h.home.home, HookKind::WaitingInput, "p1");
    h.wait_for("waiting 2", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::WaitingInput))
    });
    assert_eq!(h.system.0.borrow().notifications.len(), 1, "banner sent");
    assert_eq!(
        h.system.0.borrow().notification_sounds,
        vec![false],
        "banner is silent"
    );
    assert_eq!(
        h.system.0.borrow().beeps,
        1,
        "no extra beep when handoff_sound is false"
    );

    // Case 3: notifications = false, handoff_sound = false
    // Completely silent and no banners
    h.view.update(&mut h.cx, |v, _| {
        v.config.notifications = false;
        v.config.handoff_sound = false;
    });

    h.hook_session(
        Some(pane_raw),
        &h.home.home,
        HookKind::PromptSubmitted,
        "p1",
    );
    h.wait_for("working 3", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Working))
    });

    h.hook_session(Some(pane_raw), &h.home.home, HookKind::Stopped, "p1");
    h.wait_for("review 3", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Review))
    });
    assert_eq!(h.system.0.borrow().notifications.len(), 1, "no new banner");
    assert_eq!(h.system.0.borrow().beeps, 1, "no beep when both disabled");
}

#[gpui::test]
fn child_activity_does_not_play_handoff_sound(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "handoff-sound-child", |_| {});
    h.wait_prompt();
    let parent = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pane_raw = parent.raw();

    h.hook_session(Some(pane_raw), &h.home.home, HookKind::SessionStart, "p1");
    h.wait_for("parent idle", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .first()
            .is_some_and(|r| r.status == Some(AgentStatus::Idle))
    });
    assert_eq!(h.system.0.borrow().beeps, 0);

    let at = crate::terminal_view::now_ms();
    // Child starts working
    send_child(&h, "claude", pane_raw, "p1", "c1", ChildState::Working, at);
    h.wait_for("child working", |v, cx| {
        let groups = &v.sidebar.read(cx).child_groups;
        !groups.is_empty() && groups[0].children.len() == 1
    });
    assert_eq!(
        h.system.0.borrow().beeps,
        0,
        "child working does not play handoff sound"
    );

    // Child finishes (Ended)
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
        h.system.0.borrow().beeps,
        0,
        "child ended does not play handoff sound"
    );
}
