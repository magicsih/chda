//! Background panes do not redraw the window (#134): an agent animating its
//! title or printing output in another tab leaves the workspace and the
//! sidebar alone unless something they show changes.

use super::harness::Harness;
use crate::terminal_view::{TerminalEvent, now_ms};
use gpui::TestAppContext;
use std::{cell::Cell, rc::Rc};

#[gpui::test]
fn background_title_spinners_and_output_do_not_redraw_the_window(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "redraw", |home| {
        repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.run(
        "stty -echo; printf '\nredraw-background-ready\n'; exec cat",
        "\nredraw-background-ready\n",
    );
    let background = h.focused_terminal();
    h.keys("cmd-t");
    h.wait_prompt();
    h.run(
        "stty -echo; printf '\nredraw-foreground-ready\n'; exec cat",
        "\nredraw-foreground-ready\n",
    );
    assert_ne!(h.focused_terminal(), background);

    let workspace = Rc::new(Cell::new(0));
    let sidebar = Rc::new(Cell::new(0));
    let sidebar_entity = h.read(|v, _| v.sidebar.clone());
    h.cx.update(|_, cx| {
        let count = workspace.clone();
        cx.observe(&h.view, move |_, _| count.set(count.get() + 1))
            .detach();
        let count = sidebar.clone();
        cx.observe(&sidebar_entity, move |_, _| count.set(count.get() + 1))
            .detach();
    });
    let emit = |h: &mut Harness, event: TerminalEvent| {
        h.cx.update(|_, cx| background.update(cx, |_, cx| cx.emit(event)));
        h.cx.run_until_parked();
    };

    // Codex starts working: a real status change redraws.
    emit(&mut h, TerminalEvent::Title("⠋ codex | Working".into()));
    assert!(workspace.get() > 0, "a working agent shows");
    let tab_title = h.read(|v, _| v.ws.tab_title(&v.ws.tabs()[0]));

    // Its spinner frames and output do not.
    let (w, s) = (workspace.get(), sidebar.get());
    for frame in ["⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"] {
        emit(
            &mut h,
            TerminalEvent::Title(format!("{frame} codex | Working")),
        );
        emit(&mut h, TerminalEvent::Activity(now_ms()));
    }
    assert_eq!(
        workspace.get(),
        w,
        "spinner and output redrew the workspace"
    );
    assert_eq!(sidebar.get(), s, "spinner and output redrew the sidebar");
    assert_eq!(h.read(|v, _| v.ws.tab_title(&v.ws.tabs()[0])), tab_title);

    // Sessions still learns the new activity time for its next age tick.
    let pane = h.read(|v, _| v.ws.tabs()[0].panes()[0]);
    let at = h.read(|v, _| v.ws.pane(pane).unwrap().last_activity);
    assert!(at > 0);
    assert!(h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .any(|r| r.last_activity == at)
    }));

    // The turn ending is a real change again.
    let before = workspace.get();
    emit(&mut h, TerminalEvent::Title("codex | Ready".into()));
    assert!(workspace.get() > before, "a finished turn shows");
}

#[gpui::test]
fn session_age_waits_a_minute_hides_work_and_clears_when_output_resumes(cx: &mut TestAppContext) {
    use super::sidebar::draw_sidebar_frame;
    use chda_core::agents::HookKind;
    use chda_core::{AgentStatus, SessionKey, session_activity_age};
    let mut h = Harness::open(cx, "quiet-session-age", |_| {});
    h.wait_prompt();
    h.run(
        "stty -echo; printf '\nquiet-age-ready\n'; exec cat",
        "\nquiet-age-ready\n",
    );
    let terminal = h.focused_terminal();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let home = h.home.home.clone();
    h.hook(Some(pane.raw()), &home, HookKind::WaitingInput);
    h.wait_for("waiting row", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .any(|r| r.key == SessionKey::Pane(pane) && r.status == Some(AgentStatus::WaitingInput))
    });
    let now = now_ms();
    let age = |h: &Harness| {
        h.read(|v, cx| {
            let row = v
                .sidebar
                .read(cx)
                .model
                .sessions
                .iter()
                .find(|r| r.key == SessionKey::Pane(pane))
                .unwrap();
            session_activity_age(row.status, now, row.last_activity)
        })
    };
    let slot: &'static str =
        Box::leak(format!("activity-age-pane-{}", pane.raw()).into_boxed_str());
    draw_sidebar_frame(&mut h);
    let size = h.cx.debug_bounds(slot).unwrap().size;
    for (at, expected) in [
        (0, ""),
        (now - 59_999, ""),
        (now - 60_000, "1m"),
        (now - 120_000, "2m"),
        (now - 3_600_000, "1h"),
    ] {
        h.cx.update(|_, cx| terminal.update(cx, |_, cx| cx.emit(TerminalEvent::Activity(at))));
        h.cx.run_until_parked();
        assert_eq!(age(&h), expected);
        draw_sidebar_frame(&mut h);
        assert_eq!(h.cx.debug_bounds(slot).unwrap().size, size);
    }
    h.hook(Some(pane.raw()), &home, HookKind::PromptSubmitted);
    h.wait_for("working row", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .any(|r| r.key == SessionKey::Pane(pane) && r.status == Some(AgentStatus::Working))
    });
    assert_eq!(age(&h), "");
    h.hook(Some(pane.raw()), &home, HookKind::WaitingInput);
    h.wait_for("waiting again", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .any(|r| r.key == SessionKey::Pane(pane) && r.status == Some(AgentStatus::WaitingInput))
    });
    h.type_text("output-resumed");
    h.keys("enter");
    h.wait_for("actual resumed output", |v, cx| {
        v.focused_text(cx).contains("output-resumed")
            && v.ws.pane(pane).unwrap().last_activity >= now
    });
    assert_eq!(age(&h), "");
    draw_sidebar_frame(&mut h);
    assert_eq!(h.cx.debug_bounds(slot).unwrap().size, size);
}
