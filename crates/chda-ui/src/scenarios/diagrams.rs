//! Mermaid diagrams in the output open in the offline viewer.

use std::rc::Rc;

use gpui::{Modifiers, TestAppContext, point};

use super::harness::Harness;

#[gpui::test]
fn mermaid_output_opens_in_the_offline_viewer(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "mermaid", |_| {});
    h.wait_prompt();
    h.type_text("clear; printf '```mermaid\\nflowchart LR\\n  A --> B\\n```\\n'");
    h.keys("enter");
    let term = h.focused_terminal();
    h.wait_for("the diagram to be found", {
        let term = term.clone();
        move |_, cx| !term.read(cx).frame().diagrams.is_empty()
    });

    // cmd-click inside the block.
    let (geometry, row) = h.read(|_, cx| {
        let t = term.read(cx);
        (t.geometry.unwrap(), t.frame().diagrams[0].first_row + 1)
    });
    let at = point(
        geometry.origin.x + geometry.cell_width * 3.5,
        geometry.origin.y + geometry.line_height * (f32::from(row) + 0.5),
    );
    h.cx.simulate_mouse_move(at, None, Modifiers::command());
    h.cx.simulate_click(at, Modifiers::command());
    let page = {
        let opened = &h.system.0.borrow().opened_files;
        assert_eq!(opened.len(), 1);
        opened[0].clone()
    };
    assert_eq!(page.extension().unwrap(), "html");
    let html = std::fs::read_to_string(&page).unwrap();
    assert!(html.contains("flowchart LR\n  A --&gt; B\n"), "{html}");
    let script = html
        .split("<script src=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap();
    assert!(page.parent().unwrap().join(script).is_file());

    // The palette finds the last diagram in the scrollback.
    h.keys("cmd-shift-p");
    h.type_text("View last diagram");
    h.keys("enter");
    let system = Rc::clone(&h.system);
    h.wait_for("the palette to open the viewer", move |_, _| {
        system.0.borrow().opened_files.len() == 2
    });
    assert_eq!(h.system.0.borrow().opened_files[1], page);
}
