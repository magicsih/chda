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
    let background = h.focused_terminal();
    h.keys("cmd-t");
    h.wait_prompt();
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

    // ACTIVE still learns the new activity time for its next age tick.
    let pane = h.read(|v, _| v.ws.tabs()[0].panes()[0]);
    let at = h.read(|v, _| v.ws.pane(pane).unwrap().last_activity);
    assert!(at > 0);
    assert!(h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .active_tabs
            .iter()
            .any(|t| t.last_activity == at)
    }));

    // The turn ending is a real change again.
    let before = workspace.get();
    emit(&mut h, TerminalEvent::Title("codex | Ready".into()));
    assert!(workspace.get() > before, "a finished turn shows");
}
