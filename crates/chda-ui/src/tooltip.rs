//! A plain text tooltip for hover hints.

use gpui::prelude::*;
use gpui::{AnyView, App, AppContext, Context, IntoElement, Render, SharedString, Window, div};

struct TextTooltip(SharedString);

impl Render for TextTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = self.0.clone();
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
                self.0
                    .split('\n')
                    .map(|line| div().child(SharedString::from(line.to_owned()))),
            )
    }
}

/// A tooltip builder for `.tooltip(...)` showing `text`, one line per `\n`.
pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| TextTooltip(text.clone())).into()
}
