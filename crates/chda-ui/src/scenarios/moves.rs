//! Reordering tabs and moving running panes between tabs and windows (#248).
use gpui::{Modifiers, MouseButton, TestAppContext, point, px};

use super::harness::{Harness, wait_until};
use crate::workspace_view::PaneDestination;

fn tab_center(h: &mut Harness, index: usize) -> gpui::Point<gpui::Pixels> {
    h.cx.run_until_parked();
    let selector: &'static str = Box::leak(format!("tab-{index}").into_boxed_str());
    h.cx.debug_bounds(selector).expect(selector).center()
}

/// The shell's PID, printed in the focused pane.
fn shell_pid(h: &mut Harness, tag: &str) -> String {
    h.run(&format!("printf '{tag}-%s\\n' $$"), &format!("{tag}-"));
    let text = h.read(|v, cx| v.focused_text(cx));
    let line = text
        .lines()
        .rev()
        .find(|l| l.starts_with(&format!("{tag}-")))
        .unwrap();
    line[tag.len() + 1..].trim().to_owned()
}

#[gpui::test]
fn dragging_a_tab_reorders_it_and_the_order_is_restored(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "tab-drag", |_| {});
    h.wait_prompt();
    h.keys("cmd-t");
    h.wait_prompt();
    h.keys("cmd-t");
    h.wait_prompt();
    let order =
        |h: &Harness| h.read(|v, _| v.ws.tabs().iter().map(|t| t.panes()[0]).collect::<Vec<_>>());
    let before = order(&h);
    let (from, to) = (tab_center(&mut h, 2), tab_center(&mut h, 0));
    let none = Modifiers::none();
    h.cx.simulate_mouse_down(from, MouseButton::Left, none);
    for step in 1..=6 {
        let t = step as f32 / 6.0;
        let at = point(from.x + (to.x - from.x) * t, from.y + px(1.0));
        h.cx.simulate_mouse_move(at, MouseButton::Left, none);
    }
    h.cx.simulate_mouse_up(to, MouseButton::Left, none);
    h.cx.run_until_parked();
    let after = order(&h);
    assert_eq!(after, [before[2], before[0], before[1]]);
    let (mut cx2, view2) = h.reopen();
    wait_until(&mut cx2, &view2, "restored tabs", |v, _| {
        v.ws.tabs().len() == 3
    });
    let titles = |v: &crate::workspace_view::WorkspaceView| {
        v.ws.tabs()
            .iter()
            .map(|t| v.ws.tab_title(t))
            .collect::<Vec<_>>()
    };
    let saved = h.read(|v, _| titles(v));
    view2.read_with(&cx2, |v, _| assert_eq!(titles(v), saved));
}

#[gpui::test]
fn a_running_pane_moves_between_tabs_and_windows_without_restarting(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "pane-move", |_| {});
    h.wait_prompt();
    h.keys("cmd-d");
    h.wait_prompt();
    let moving = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.run("printf 'before-%s\\n' move", "before-move");
    let pid = shell_pid(&mut h, "pid");
    let first_tab = h.read(|v, _| v.ws.active_tab().unwrap().id);
    h.keys("cmd-t");
    h.wait_prompt();
    let second_tab = h.read(|v, _| v.ws.active_tab().unwrap().id);

    // Through the tab's menu: into the other tab, beside its pane.
    let tab = tab_center(&mut h, 0);
    h.cx.simulate_mouse_down(tab, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(tab, MouseButton::Right, Modifiers::none());
    h.cx.run_until_parked();
    let index = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|(label, _)| label.starts_with("Move to Tab 2"))
            .unwrap()
    });
    let item: &'static str = Box::leak(format!("menu-item-{index}").into_boxed_str());
    let at = h.cx.debug_bounds(item).unwrap().center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
    h.read(|v, _| {
        let tab = v.ws.tabs().iter().find(|t| t.id == second_tab).unwrap();
        assert!(
            tab.panes().contains(&moving),
            "moved beside the other tab's pane"
        );
        assert_eq!(v.ws.focused_pane(), Some(moving));
        let first = v.ws.tabs().iter().find(|t| t.id == first_tab).unwrap();
        assert_eq!(first.panes().len(), 1, "its old tab keeps the other pane");
    });
    assert_eq!(
        shell_pid(&mut h, "same"),
        pid,
        "the same shell keeps running"
    );

    // Out of the split into a tab of its own.
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.move_pane(moving, PaneDestination::NewTab, window, cx)
        })
    });
    h.cx.run_until_parked();
    h.read(|v, _| {
        assert_eq!(v.ws.active_tab().unwrap().panes(), [moving]);
        assert_eq!(v.ws.tabs().len(), 3);
    });

    // To another window: the same process, with its scrollback.
    let env = h.read(|v, _| v.env.clone());
    let handle =
        h.cx.update(|_, cx| crate::open_workspace_window(None, env.clone(), cx));
    let mut second_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let second = handle.root(&mut second_cx).unwrap();
    wait_until(&mut second_cx, &second, "second window shell", |v, _| {
        v.ws.focused_pane().is_some()
    });
    let target = Some(handle.window_id());
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.move_pane(moving, PaneDestination::Window(target), window, cx)
        })
    });
    wait_until(&mut second_cx, &second, "the moved pane", |v, cx| {
        v.ws.pane(moving).is_some() && v.pane_text(moving, cx).contains("before-move")
    });
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v.ws.pane(moving).is_none()),
        "left the first window"
    );
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        2,
        "its emptied tab closed"
    );
    assert_eq!(
        second.read_with(&second_cx, |v, _| v.ws.focused_pane()),
        Some(moving),
        "the moved pane has focus there"
    );
    second_cx.simulate_input("printf 'after-%s\\n' $$");
    second_cx.simulate_keystrokes("enter");
    let expected = format!("after-{pid}");
    wait_until(
        &mut second_cx,
        &second,
        "the same shell answers",
        move |v, cx| v.pane_text(moving, cx).contains(&expected),
    );
}
