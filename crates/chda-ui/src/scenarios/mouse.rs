//! Mouse reporting: a running program owns the mouse, and once the shell is
//! back at its prompt the pointer selects and scrolls again instead of typing
//! reports into the command line (#263).

use std::sync::Arc;

use chda_term::Frame;
use gpui::{Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext, point, px};

use super::harness::Harness;
use crate::terminal_view::TerminalView;

/// A program that turns every mouse tracking mode on, shows the first report
/// it receives and exits without turning them off, as an interrupted or
/// crashed full-screen program does. The marker is built from two words so
/// that only the program's own output matches it, not the typed command.
const MOUSE_PROGRAM: &str = "clear; stty -icanon -echo; \
    printf '\\033[?1000h\\033[?1002h\\033[?1003h\\033[?1006h'; \
    printf '\\n%s%s\\n' MOUSE _READY; \
    read -rs -d M report; stty sane; esc=$'\\e'; \
    print -r -- \"REPORT:${report//$esc/ESC}\"";

fn frame(h: &Harness, term: &Entity<TerminalView>) -> Arc<Frame> {
    h.read(|_, cx| term.read(cx).frame())
}

/// The center of the cell in column `x` and row `y` of the terminal.
fn cell(h: &Harness, term: &Entity<TerminalView>, x: u16, y: u16) -> Point<Pixels> {
    let g = h.read(|_, cx| term.read(cx).geometry.expect("the terminal is drawn"));
    point(
        g.origin.x + g.cell_width * (f32::from(x) + 0.5),
        g.origin.y + g.line_height * (f32::from(y) + 0.5),
    )
}

fn has_row(frame: &Frame, text: &str) -> bool {
    (0..frame.size.rows).any(|y| frame.row_text(y) == text)
}

fn wait_for_row(h: &mut Harness, term: &Entity<TerminalView>, text: &str) {
    let term = term.clone();
    let text = text.to_owned();
    h.wait_for(&format!("the row {text:?}"), move |_, cx| {
        has_row(&term.read(cx).frame(), &text)
    });
}

/// Wait for the shell to show a new prompt after a command ended.
fn wait_for_prompt(h: &mut Harness, term: &Entity<TerminalView>) {
    let term = term.clone();
    h.wait_for("the shell prompt", move |_, cx| {
        term.read(cx).frame().at_prompt()
    });
}

/// Start [`MOUSE_PROGRAM`] and wait until it has asked for the mouse.
fn start_program(h: &mut Harness, term: &Entity<TerminalView>) {
    h.type_text(MOUSE_PROGRAM);
    h.keys("enter");
    wait_for_row(h, term, "MOUSE_READY");
}

#[gpui::test]
fn mouse_reports_belong_to_the_program_and_end_when_the_shell_returns(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "mouse-reports", |_| {});
    h.wait_prompt();
    let term = h.focused_terminal();
    let none = Modifiers::none();

    // While the program runs, the pointer reaches it: the move at column 10,
    // row 5 arrives as an SGR report for cell 11;6.
    start_program(&mut h, &term);
    assert!(frame(&h, &term).mouse_tracking);
    h.cx.simulate_mouse_move(cell(&h, &term, 10, 5), None, none);
    wait_for_row(&mut h, &term, "REPORT:ESC[<35;11;6");

    // The program ended without turning the mouse off; the prompt takes it back.
    wait_for_prompt(&mut h, &term);
    assert!(
        !frame(&h, &term).mouse_tracking,
        "the shell prompt ends mouse reporting"
    );

    // Movement, clicks and the wheel go nowhere near the command line.
    let at = cell(&h, &term, 20, 8);
    let line_height = h.read(|_, cx| term.read(cx).geometry.unwrap().line_height);
    for step in 0..4 {
        h.cx.simulate_mouse_move(cell(&h, &term, 20 + step, 8), None, none);
    }
    h.cx.simulate_click(at, none);
    h.cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), line_height * 2.0)),
        modifiers: none,
        touch_phase: gpui::TouchPhase::Moved,
    });
    h.cx.run_until_parked();

    // Typing still works: the command arrives whole, not behind report text.
    // The screen is cleared first, since the program's own report is on it.
    h.type_text("clear; echo AF''TER");
    h.keys("enter");
    wait_for_row(&mut h, &term, "AFTER");
    let screen = frame(&h, &term);
    assert!(
        !(0..screen.size.rows).any(|y| screen.row_text(y).contains("35;")),
        "no mouse report reached the shell's input"
    );

    // Selection works again: dragging over the output row selects its cells.
    let row = (0..screen.size.rows)
        .find(|&y| screen.row_text(y) == "AFTER")
        .expect("the output row");
    h.cx.simulate_mouse_down(cell(&h, &term, 0, row), MouseButton::Left, none);
    h.cx.simulate_mouse_move(cell(&h, &term, 4, row), MouseButton::Left, none);
    h.cx.simulate_mouse_up(cell(&h, &term, 4, row), MouseButton::Left, none);
    h.wait_for("the selected cells", {
        let term = term.clone();
        move |_, cx| term.read(cx).frame().cells.iter().any(|c| c.selected)
    });

    // So does scrollback: the wheel moves the viewport into history.
    h.type_text("seq 1 200");
    h.keys("enter");
    wait_for_row(&mut h, &term, "200");
    let bottom = frame(&h, &term).scrollbar;
    assert_eq!(bottom.offset + bottom.len, bottom.total, "at the bottom");
    h.cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), line_height * 5.0)),
        modifiers: none,
        touch_phase: gpui::TouchPhase::Moved,
    });
    h.wait_for("the viewport to move into history", {
        let term = term.clone();
        move |_, cx| {
            let bar = term.read(cx).frame().scrollbar;
            bar.offset + bar.len < bar.total
        }
    });

    // A later program can ask for the mouse again, and gets it.
    start_program(&mut h, &term);
    assert!(frame(&h, &term).mouse_tracking);
    h.cx.simulate_mouse_move(cell(&h, &term, 10, 5), None, none);
    wait_for_row(&mut h, &term, "REPORT:ESC[<35;11;6");
    wait_for_prompt(&mut h, &term);
    assert!(!frame(&h, &term).mouse_tracking);
}
