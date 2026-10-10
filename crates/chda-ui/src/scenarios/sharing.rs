//! Sharing starts with an actual VT selection and never copies another pane.
use super::harness::Harness;
use gpui::{ClipboardItem, Modifiers, MouseButton, Pixels, Point, TestAppContext, point, px, size};

fn selected(h: &mut Harness) {
    h.wait_prompt();
    h.run(
        "printf '\\033[2J\\033[HShared draft | literal pipe\\r\\n'",
        "Shared draft",
    );
    let term = h.focused_terminal();
    h.wait_for("a standalone output line", {
        let term = term.clone();
        move |_, cx| {
            term.read(cx)
                .frame()
                .row_text(0)
                .starts_with("Shared draft |")
        }
    });
    let (first, last) = h.read(|_, cx| {
        let g = term.read(cx).geometry.unwrap();
        (
            point(
                g.origin.x + g.cell_width * 0.5,
                g.origin.y + g.line_height * 0.5,
            ),
            point(
                g.origin.x + g.cell_width * 26.5,
                g.origin.y + g.line_height * 0.5,
            ),
        )
    });
    select(h, first, last);
}

/// Drag over the focused terminal's cells and open its context menu.
fn select(h: &mut Harness, first: Point<Pixels>, last: Point<Pixels>) {
    let term = h.focused_terminal();
    h.cx.simulate_mouse_down(first, MouseButton::Left, Modifiers::none());
    h.cx.simulate_mouse_move(last, MouseButton::Left, Modifiers::none());
    h.cx.simulate_mouse_up(last, MouseButton::Left, Modifiers::none());
    h.wait_for("selected cells", move |_, cx| {
        term.read(cx).frame().cells.iter().any(|c| c.selected)
    });
    h.cx.simulate_mouse_down(last, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(last, MouseButton::Right, Modifiers::none());
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v.context_menu.as_ref().is_some_and(|m| m
            .items
            .iter()
            .any(|(label, _)| label == "Copy for sharing…"))),
        "the selected draft has a discoverable sharing action"
    );
}

fn action(h: &mut Harness, label: &str) -> crate::workspace_view::MenuAction {
    h.read(|v, _| {
        v.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|(l, _)| l == label)
            .unwrap()
            .1
            .clone()
    })
}
fn run(h: &mut Harness, action: crate::workspace_view::MenuAction) {
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.cx.run_until_parked();
}
fn clipboard(h: &Harness) -> Option<String> {
    h.read(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
}
fn preview(h: &mut Harness) {
    let action = action(h, "Copy for sharing…");
    run(h, action);
    h.wait_for("editable sharing preview", |v, _| {
        v.share_sheet.as_ref().is_some_and(|s| s.input.is_some())
    });
}
#[gpui::test]
fn sharing_edits_copy_plain_text_and_cancel_restores_terminal_input(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sharing", |_| {});
    selected(&mut h);
    h.cx.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("existing clipboard".into()))
    });
    preview(&mut h);
    assert_eq!(clipboard(&h).as_deref(), Some("existing clipboard"));
    h.keys("cmd-a");
    h.type_text("안녕 🧭 | preserved");
    h.keys("shift-enter");
    h.type_text("    code | pipe");
    h.keys("enter");
    assert_eq!(
        clipboard(&h).as_deref(),
        Some("안녕 🧭 | preserved\n    code | pipe")
    );
    assert!(h.read(|v, _| v.share_sheet.is_none()));
    selected(&mut h);
    preview(&mut h);
    h.keys("escape");
    assert!(h.read(|v, _| v.share_sheet.is_none()));
    h.run("echo restored-sharing-focus", "restored-sharing-focus");
}
#[gpui::test]
fn a_long_last_line_wraps_in_the_preview_and_copies_without_added_newlines(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "sharing-wrap", |_| {});
    h.cx.simulate_resize(size(px(1200.0), px(800.0)));
    h.wait_prompt();
    let words: Vec<_> = (1..=40).map(|n| format!("word{n:02}")).collect();
    let long = format!("Long line {}", words.join(" "));
    h.run(
        &format!("printf '\\033[2J\\033[HShared draft | first\\r\\nsecond line\\r\\n{long}\\r\\n'"),
        "Shared draft",
    );
    let term = h.focused_terminal();
    let prompt =
        |f: &chda_term::Frame| (3..f.size.rows).find(|&r| f.row_text(r).starts_with("test%"));
    h.wait_for(
        "the draft, its soft-wrapped last line and the next prompt",
        {
            let term = term.clone();
            move |_, cx| {
                let t = term.read(cx);
                let f = t.frame();
                f.row_text(0).starts_with("Shared draft |") && prompt(&f).is_some()
            }
        },
    );
    let (first, last) = h.read(|_, cx| {
        let t = term.read(cx);
        let f = t.frame();
        let prompt = prompt(&f).unwrap();
        assert!(prompt > 3, "the long line also wraps in the terminal");
        let g = t.geometry.unwrap();
        (
            point(
                g.origin.x + g.cell_width * 0.5,
                g.origin.y + g.line_height * 0.5,
            ),
            point(
                g.origin.x + g.cell_width * (f32::from(f.size.cols) - 0.5),
                g.origin.y + g.line_height * (f32::from(prompt) - 0.5),
            ),
        )
    });
    select(&mut h, first, last);
    preview(&mut h);
    let expected = format!("Shared draft | first\nsecond line\n{long}");
    let input = h.read(|v, _| v.share_sheet.as_ref().unwrap().input.clone().unwrap());
    for window in [size(px(1200.0), px(800.0)), size(px(640.0), px(600.0))] {
        h.cx.simulate_resize(window);
        h.cx.run_until_parked();
        let editor = h.cx.debug_bounds("sharing-editor").unwrap();
        let (text, painted) =
            input.read_with(&h.cx, |i, _| (i.text().to_owned(), i.painted_text()));
        let (start, rows) = painted.unwrap();
        assert_eq!(text, expected, "the preview keeps the selected text");
        assert!(
            editor.contains(&start),
            "{window:?}: the first character is painted at {start:?}, outside the editor {editor:?}"
        );
        assert!(
            rows > expected.lines().count(),
            "{window:?}: the long last line wraps instead of scrolling sideways ({rows} rows)"
        );
    }
    h.keys("enter");
    assert_eq!(
        clipboard(&h).as_deref(),
        Some(expected.as_str()),
        "wrapping in the preview adds no newlines"
    );
}

#[gpui::test]
fn sharing_revalidates_menu_and_preview_conversation_before_copying(cx: &mut TestAppContext) {
    use chda_core::agents::HookKind;
    let mut h = Harness::open(cx, "sharing-scope", |_| {});
    selected(&mut h);
    let old = action(&mut h, "Copy for sharing…");
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let cwd = h.home.home.clone();
    h.hook_session(Some(pane.raw()), &cwd, HookKind::SessionStart, "first");
    h.wait_for("first conversation", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "first")
    });
    run(&mut h, old);
    assert!(h.read(|v, _| v.share_sheet.is_none()));
    selected(&mut h);
    preview(&mut h);
    h.cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("keep clipboard".into())));
    h.hook_session(Some(pane.raw()), &cwd, HookKind::SessionStart, "second");
    h.wait_for("second conversation", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "second")
    });
    h.keys("enter");
    assert!(h.read(|v, _| v.share_sheet.is_none()));
    assert_eq!(clipboard(&h).as_deref(), Some("keep clipboard"));
}

#[gpui::test]
fn ordinary_copy_keeps_the_terminal_selection_behavior(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sharing-copy", |_| {});
    selected(&mut h);
    let copy = action(&mut h, "Copy");
    run(&mut h, copy);
    h.wait_for("ordinary clipboard reply", |_, cx| {
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref()
            == Some("Shared draft | literal pipe")
    });
}

#[gpui::test]
fn an_open_sharing_preview_defers_update_and_never_copies_after_tab_navigation(
    cx: &mut TestAppContext,
) {
    use chda_core::self_update::UpdateProgress;
    let mut h = Harness::open(cx, "sharing-update", |home| {
        std::fs::write(&home.latest_release, r#"{"tag_name":"v99.0.0","html_url":"https://github.com/magicsih/chda/releases/tag/v99.0.0"}"#).unwrap();
        let job = home.data.join("update-fixture");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(job.join("progress.jsonl"), "{\"phase\":\"ready\"}\n").unwrap();
    });
    selected(&mut h);
    preview(&mut h);
    h.wait_for("latest release", |v, _| v.release_notice.is_some());
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.start_update(window, cx)));
    h.wait_for("fixture updater launched", |v, _| {
        v.env.windows.borrow().update.job.is_some()
    });
    h.cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    h.wait_for("update waits for preview", |v, _| {
        let registry = v.env.windows.borrow();
        registry.update.progress == UpdateProgress::Waiting && !registry.update.frozen
    });
    assert!(h.read(|v, _| v.share_sheet.as_ref().is_some_and(|s| s.input.is_some())));
    h.cx.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("keep after navigation".into()))
    });
    h.keys("cmd-t");
    h.wait_prompt();
    let focused = h.read(|v, _| v.ws.focused_pane().unwrap());
    let button = h.cx.debug_bounds("sharing-copy").unwrap().center();
    h.cx.simulate_click(button, Modifiers::none());
    h.cx.run_until_parked();
    assert_eq!(clipboard(&h).as_deref(), Some("keep after navigation"));
    assert!(h.read(|v, _| v.share_sheet.is_none()));
    assert_eq!(h.read(|v, _| v.ws.focused_pane().unwrap()), focused);
}
