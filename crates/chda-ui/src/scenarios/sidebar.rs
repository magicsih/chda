//! Resizing and collapsing the sidebar.

use gpui::{Modifiers, MouseButton, TestAppContext, point, px};

use super::harness::Harness;

fn width(h: &Harness) -> u32 {
    h.read(|v, _| v.config.sidebar_width)
}

fn saved(h: &Harness) -> String {
    std::fs::read_to_string(&h.home.config).unwrap_or_default()
}

fn drag_grip_to(h: &mut Harness, x: f32) {
    h.cx.run_until_parked();
    let grip =
        h.cx.debug_bounds("sidebar-grip")
            .expect("the grip is drawn");
    let start = grip.center();
    h.cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    for step in 1..=4 {
        let x = f32::from(start.x) + (x - f32::from(start.x)) * step as f32 / 4.0;
        h.cx.simulate_mouse_move(point(px(x), start.y), MouseButton::Left, Modifiers::none());
    }
    h.cx.simulate_mouse_up(point(px(x), start.y), MouseButton::Left, Modifiers::none());
    h.cx.run_until_parked();
}

#[gpui::test]
fn dragging_the_border_resizes_the_sidebar_within_bounds(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sidebar-drag", |_| {});
    h.wait_prompt();
    assert_eq!(width(&h), 280);

    drag_grip_to(&mut h, 400.0);
    assert_eq!(width(&h), 400);
    assert!(saved(&h).contains("sidebar-width = 400"), "{}", saved(&h));
    // The panes follow the border.
    let grip = h.cx.debug_bounds("sidebar-grip").unwrap();
    assert!((f32::from(grip.right()) - 400.0).abs() <= 2.0, "{grip:?}");

    // Past either end it stops at the bounds.
    drag_grip_to(&mut h, 20.0);
    assert_eq!(width(&h), 180);
    drag_grip_to(&mut h, 5000.0);
    let window =
        h.cx.update(|window, _| f32::from(window.viewport_size().width));
    assert_eq!(width(&h) as f32, 600f32.min(window - 320.0).max(180.0));

    // Moving the pointer without a drag changes nothing.
    let before = width(&h);
    h.cx.simulate_mouse_move(point(px(250.), px(300.)), None, Modifiers::none());
    assert_eq!(width(&h), before);

    // Double-click resets the default.
    let grip = h.cx.debug_bounds("sidebar-grip").unwrap().center();
    h.cx.simulate_event(gpui::MouseDownEvent {
        position: grip,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    h.cx.simulate_event(gpui::MouseUpEvent {
        position: grip,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
    });
    h.cx.run_until_parked();
    assert_eq!(width(&h), 280);
}

#[gpui::test]
fn the_title_bar_button_collapses_and_expands_the_sidebar(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sidebar-toggle", |_| {});
    h.wait_prompt();
    drag_grip_to(&mut h, 350.0);
    let click = |h: &mut Harness| {
        h.cx.run_until_parked();
        let at =
            h.cx.debug_bounds("sidebar-toggle")
                .expect("the button is drawn")
                .center();
        h.cx.simulate_click(at, Modifiers::none());
        h.cx.run_until_parked();
    };

    click(&mut h);
    assert!(h.read(|v, _| !v.sidebar_visible));
    assert!(h.cx.debug_bounds("sidebar-grip").is_none());
    assert!(
        saved(&h).contains("sidebar-visible = false"),
        "{}",
        saved(&h)
    );

    click(&mut h);
    assert!(h.read(|v, _| v.sidebar_visible));
    assert_eq!(width(&h), 350, "the width survives collapsing");

    // cmd-b and the button stay in step.
    h.keys("cmd-b");
    h.cx.run_until_parked();
    assert!(h.read(|v, _| !v.sidebar_visible));
}
