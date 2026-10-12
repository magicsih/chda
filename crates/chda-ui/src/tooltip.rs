//! A plain text tooltip for hover hints.

use gpui::prelude::*;
use gpui::{
    AnyView, App, AppContext, Context, Global, IntoElement, Render, SharedString, Window, WindowId,
    div,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
struct Gate {
    epoch: u64,
    blocked: bool,
}
#[derive(Default)]
struct Gates(HashMap<WindowId, Gate>);
impl Global for Gates {}

/// Invalidate active hints without replacing GPUI's pointer tracking.
pub fn dismiss(window: &Window, cx: &mut App) {
    let gate = cx
        .default_global::<Gates>()
        .0
        .entry(window.window_handle().window_id())
        .or_default();
    gate.epoch = gate.epoch.wrapping_add(1);
}
pub fn block(window: &Window, blocked: bool, cx: &mut App) {
    let gate = cx
        .default_global::<Gates>()
        .0
        .entry(window.window_handle().window_id())
        .or_default();
    if gate.blocked != blocked {
        gate.blocked = blocked;
        gate.epoch = gate.epoch.wrapping_add(1);
    }
}
pub fn forget(window: WindowId, cx: &mut App) {
    cx.default_global::<Gates>().0.remove(&window);
}

struct TextTooltip {
    text: SharedString,
    window: WindowId,
    epoch: u64,
}

impl Render for TextTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let gate = cx
            .default_global::<Gates>()
            .0
            .get(&self.window)
            .copied()
            .unwrap_or_default();
        if gate.blocked || gate.epoch != self.epoch {
            return div().into_any_element();
        }
        let text = self.text.clone();
        div()
            .debug_selector(move || format!("tooltip: {text}"))
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(gpui::rgb(0x1e1e2e))
            .border_1()
            .border_color(gpui::rgb(0x45475a))
            .text_xs()
            .text_color(gpui::rgb(0xcdd6f4))
            .flex()
            .flex_col()
            .children(
                self.text
                    .split('\n')
                    .map(|line| div().child(SharedString::from(line.to_owned()))),
            )
            .into_any_element()
    }
}

/// A tooltip builder for `.tooltip(...)` showing `text`, one line per `\n`.
pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| {
        let id = window.window_handle().window_id();
        let epoch = cx.default_global::<Gates>().0.entry(id).or_default().epoch;
        cx.new(|_| TextTooltip {
            text: text.clone(),
            window: id,
            epoch,
        })
        .into()
    }
}
