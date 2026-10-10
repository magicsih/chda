//! The agent status icon shared by the tab bar and the sidebar: a spinning
//! ring while an agent works, an "!" badge while it waits for input, and a
//! small dot otherwise.

use crate::sidebar_view::status_color;
use chda_core::AgentStatus;
use gpui::{
    AnyElement, FontWeight, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Point, Styled,
    canvas, div, point, px,
};
use std::f32::consts::PI;
use std::time::Duration;

/// Time between spinner frames; the timer only runs while an agent works.
pub const SPIN_STEP: Duration = Duration::from_millis(125);
/// Frames per spinner turn.
pub const SPIN_STEPS: u32 = 8;

/// Every icon takes the same square so labels line up.
const BOX: f32 = 12.0;
const DOT: f32 = 6.0;
const RING: f32 = 10.0;
const STROKE: f32 = 2.0;

/// The icon for `status`, or the gray dot of a row with no live agent, on
/// `bg` in a theme whose text is `fg`. `frame` turns the spinner of a working
/// agent.
pub fn status_icon(status: Option<AgentStatus>, frame: u32, fg: Hsla, bg: Hsla) -> AnyElement {
    let color = status_color(status, fg, bg);
    let icon = match status {
        Some(AgentStatus::Working) => canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let turn = (frame % SPIN_STEPS) as f32 / SPIN_STEPS as f32;
                let start = 2.0 * PI * turn - PI / 2.0;
                let r = (RING - STROKE) / 2.0;
                if let Some(path) = arc(bounds.center(), r, start, start + 1.5 * PI) {
                    window.paint_path(path, color);
                }
            },
        )
        .size(px(RING))
        .into_any_element(),
        Some(AgentStatus::WaitingInput) => div()
            .size(px(BOX))
            .rounded_full()
            .bg(color)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(9.0))
            .line_height(px(BOX))
            .font_weight(FontWeight::BOLD)
            // Cut out in the background color, which the badge stands out from.
            .text_color(bg)
            .child("!")
            .into_any_element(),
        _ => div()
            .size(px(DOT))
            .rounded_full()
            .bg(color)
            .into_any_element(),
    };
    div()
        .flex_shrink_0()
        .size(px(BOX))
        .flex()
        .items_center()
        .justify_center()
        .child(icon)
        .into_any_element()
}

/// A stroked arc around `c` from angle `a` to `b`, clockwise in screen space.
fn arc(c: Point<Pixels>, r: f32, a: f32, b: f32) -> Option<gpui::Path<Pixels>> {
    let at = |t: f32| point(c.x + px(r * t.cos()), c.y + px(r * t.sin()));
    let mut path = PathBuilder::stroke(px(STROKE));
    path.move_to(at(a));
    // Quarter-turn segments keep every arc flag unambiguous.
    let mut t = a;
    while t < b {
        let next = (t + PI / 2.0).min(b);
        path.arc_to(point(px(r), px(r)), px(0.0), false, true, at(next));
        t = next;
    }
    path.build().ok()
}
