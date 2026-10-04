//! Typed input: IME composition, the shell's locale, cmd-click links.

use gpui::{EntityInputHandler, Modifiers, TestAppContext, point};

use super::harness::Harness;

#[gpui::test]
fn ime_composition_is_placed_even_when_the_app_hides_the_cursor(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "ime", |_| {});
    h.wait_prompt();
    // Like Claude Code: hide the cursor and wait for input.
    h.run("printf '\\033[?25l'; read x", "read x");
    let term = h.focused_terminal();
    h.wait_for("the cursor to be hidden", {
        let term = term.clone();
        move |_, cx| term.read(cx).frame().cursor.is_none()
    });
    h.cx.update(|window, cx| {
        term.update(cx, |t, cx| {
            t.replace_and_mark_text_in_range(None, "하", None, window, cx)
        })
    });
    h.cx.run_until_parked();
    let (marked, anchor) = h.read(|_, cx| {
        let t = term.read(cx);
        (t.marked_text.clone(), t.cursor_bounds)
    });
    assert_eq!(marked.as_deref(), Some("하"));
    assert!(anchor.is_some(), "composition has a place to draw at");
}

#[gpui::test]
fn shells_get_the_locale(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "lang", |_| {});
    h.wait_prompt();
    h.run("echo L=$LANG", "L=en_US.UTF-8");
}

#[gpui::test]
fn cmd_hover_underlines_a_url_and_cmd_click_opens_it(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "link", |_| {});
    h.wait_prompt();
    h.type_text("clear; echo 'see https://example.com/x ok'");
    h.keys("enter");
    let term = h.focused_terminal();
    // The typed command contains the URL too; wait for the output row.
    h.wait_for("the output row", {
        let term = term.clone();
        move |_, cx| {
            let frame = term.read(cx).frame();
            (0..frame.size.rows).any(|y| frame.row_text(y).starts_with("see https"))
        }
    });
    let (geometry, row) = h.read(|_, cx| {
        let t = term.read(cx);
        let frame = t.frame();
        let row = (0..frame.size.rows)
            .find(|&y| frame.row_text(y).starts_with("see https"))
            .unwrap();
        (t.geometry.unwrap(), row)
    });
    let at = point(
        geometry.origin.x + geometry.cell_width * 10.5,
        geometry.origin.y + geometry.line_height * (f32::from(row) + 0.5),
    );
    let cmd = Modifiers::command();
    h.cx.simulate_mouse_move(at, None, cmd);
    let link = h.read(|_, cx| term.read(cx).hovered_link.clone());
    assert!(
        matches!(link.map(|l| l.open), Some(crate::terminal_view::LinkOpen::Url(u)) if u == "https://example.com/x")
    );
    h.cx.simulate_click(at, cmd);
    assert_eq!(h.cx.opened_url().as_deref(), Some("https://example.com/x"));
}

#[gpui::test]
fn escape_reaches_a_working_agent_but_closes_the_palette_first(cx: &mut TestAppContext) {
    use chda_core::{AgentStatus, agents::HookKind};

    for open_palette in [false, true] {
        let mut h = Harness::open(
            cx,
            if open_palette {
                "esc-menu"
            } else {
                "esc-agent"
            },
            |_| {},
        );
        h.wait_prompt();
        let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
        let home = h.home.home.clone();
        h.hook(Some(pane.raw()), &home, HookKind::PromptSubmitted);
        h.wait_for("the agent to work", move |v, _| {
            v.ws.pane(pane)
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.status == AgentStatus::Working)
        });
        // A stand-in reads one raw byte, just as a TUI receives Esc to cancel.
        // Wait for an output row so the typed command cannot satisfy readiness.
        h.type_text(r"stty -icanon -echo; printf '\nESC_READY\n'; dd bs=1 count=1 2>/dev/null | od -An -tu1; stty sane; printf '\nESC_DONE\n'");
        h.keys("enter");
        let term = h.focused_terminal();
        h.wait_for("the raw input reader", |_, cx| {
            let frame = term.read(cx).frame();
            (0..frame.size.rows).any(|y| frame.row_text(y).trim() == "ESC_READY")
        });
        if open_palette {
            h.keys("cmd-shift-p escape");
            assert!(h.read(|v, _| v.palette.is_none()));
            h.read(|_, cx| {
                let frame = term.read(cx).frame();
                assert!(!(0..frame.size.rows).any(|y| frame.row_text(y).trim() == "27"));
            });
        }
        h.keys("escape");
        h.wait_for("Esc byte 27 in the agent's PTY", |_, cx| {
            let frame = term.read(cx).frame();
            (0..frame.size.rows).any(|y| frame.row_text(y).trim() == "27")
        });
    }
}
