//! Fixed-size status marks shared by the sidebar and tab bar.

use crate::sidebar_view::status_color;
use chda_core::AgentStatus;
use gpui::{
    Animation, AnimationExt, AnyElement, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Styled, div, px,
};
use std::time::Duration;

const BOX: f32 = 12.0;

/// Three dots brighten in sequence without moving or changing row metrics.
/// GPUI's animation wrapper uses its clock, shares phase across rows and
/// automatically stops scheduling frames when Reduce Motion is enabled.
pub fn status_icon(status: Option<AgentStatus>, fg: Hsla, bg: Hsla) -> AnyElement {
    let color = status_color(status, fg, bg);
    let icon = match status {
        Some(AgentStatus::Working) => div()
            .debug_selector(|| "working-wave".into())
            .flex()
            .items_center()
            .gap(px(1.5))
            .children((0..3_usize).map(move |dot| {
                div().size(px(3.0)).rounded_full().bg(color).with_animation(
                    ("working-dot", dot),
                    Animation::new(Duration::from_millis(1200))
                        .repeat_synced()
                        .with_max_fps(24.0),
                    move |d, phase| d.opacity(wave_brightness(phase, dot)),
                )
            }))
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
            .text_color(bg)
            .child("!")
            .into_any_element(),
        _ => div()
            .size(px(6.0))
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

fn wave_brightness(phase: f32, dot: usize) -> f32 {
    let wave = ((phase - dot as f32 / 3.0) * std::f32::consts::TAU).cos();
    let peak = (0..3)
        .map(|i| ((phase - i as f32 / 3.0) * std::f32::consts::TAU).cos())
        .fold(-1.0_f32, f32::max);
    // At least one dot keeps the fitted 3:1 status color on every frame.
    (0.4 + 0.6 * (wave + 1.0) / (peak + 1.0)).clamp(0.4, 1.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn dots_brighten_in_order_without_disappearing() {
        for dot in 0..3 {
            let phase = dot as f32 / 3.0;
            assert!((super::wave_brightness(phase, dot) - 1.0).abs() < 0.001);
            for other in 0..3 {
                if other != dot {
                    assert!(super::wave_brightness(phase, other) < 0.6);
                }
            }
        }
        for frame in 0..120 {
            let peak = (0..3)
                .map(|dot| super::wave_brightness(frame as f32 / 120.0, dot))
                .fold(0.0_f32, f32::max);
            assert!((peak - 1.0).abs() < 0.001);
            for dot in 0..3 {
                assert!((0.4..=1.0).contains(&super::wave_brightness(frame as f32 / 120.0, dot)));
            }
        }
    }
}
