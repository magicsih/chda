//! Scrollback search (cmd-f).

use gpui::TestAppContext;

use super::harness::Harness;

#[gpui::test]
fn search_finds_steps_and_closes(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "search", |_| {});
    h.wait_prompt();
    h.run("for i in 1 2 3; do echo needle-$i; done", "needle-3");
    h.keys("cmd-f");
    h.type_text("NEEDLE");
    let term = h.focused_terminal();
    let status = |h: &mut Harness| {
        let term = term.clone();
        h.cx.run_until_parked();
        h.read(move |_, cx| term.read(cx).search_status())
    };
    // The typed command line matches too: four matches, newest current.
    h.wait_for("four matches", {
        let term = term.clone();
        move |_, cx| term.read(cx).search_status().is_some_and(|s| s.total == 4)
    });
    assert_eq!(status(&mut h).unwrap().current, Some(1));
    h.keys("enter");
    h.wait_for("the second newest match", {
        let term = term.clone();
        move |_, cx| term.read(cx).search_status().unwrap().current == Some(2)
    });
    h.keys("shift-enter shift-enter");
    h.wait_for("wrap to the oldest match", {
        let term = term.clone();
        move |_, cx| term.read(cx).search_status().unwrap().current == Some(4)
    });
    h.keys("alt-c");
    h.wait_for("no case-sensitive match", {
        let term = term.clone();
        move |_, cx| term.read(cx).search_status().unwrap().total == 0
    });
    h.keys("escape");
    assert!(status(&mut h).is_none(), "esc closes the bar");
    let frame = h.read(move |_, cx| term.read(cx).frame());
    assert!(
        frame
            .cells
            .iter()
            .all(|c| c.search == chda_term::SearchMark::None),
        "closing clears the highlights"
    );
}
