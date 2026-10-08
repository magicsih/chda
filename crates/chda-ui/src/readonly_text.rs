//! App-owned output history: virtualized text with no terminal input channel.
use crate::{settings::Settings, terminal_element::hsla};
use gpui::{
    ClipboardItem, Context, ListHorizontalSizingBehavior, Render, UniformListScrollHandle, Window,
    div, prelude::*, uniform_list,
};
use std::{ops::Range, sync::Arc};

pub(crate) struct ReadOnlyText {
    text: Arc<str>,
    lines: Vec<Range<usize>>,
    scroll: UniformListScrollHandle,
    pub(crate) settings: Settings,
}
impl ReadOnlyText {
    pub(crate) fn new(text: String, settings: Settings) -> Self {
        let mut offset = 0;
        let lines = text
            .split_inclusive('\n')
            .map(|line| {
                let start = offset;
                offset += line.len();
                start..offset
            })
            .collect();
        Self {
            text: text.into(),
            lines,
            scroll: UniformListScrollHandle::new(),
            settings,
        }
    }
}
impl Render for ReadOnlyText {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(hsla(self.settings.colors.background.unwrap_or_default()))
            .text_color(fg)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .child("Previous run · read only")
                    .child(
                        div()
                            .id("previous-copy")
                            .debug_selector(|| "previous-copy".into())
                            .cursor_pointer()
                            .child("Copy all")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.text.to_string(),
                                ))
                            })),
                    ),
            )
            .child(
                uniform_list(
                    "previous-output",
                    self.lines.len(),
                    cx.processor(|this, range: Range<usize>, _, _| {
                        range
                            .map(|index| {
                                div()
                                    .px_3()
                                    .whitespace_nowrap()
                                    .font_family(this.settings.font_family.clone())
                                    .text_size(this.settings.font_size)
                                    .child(
                                        this.text[this.lines[index].clone()]
                                            .trim_end_matches('\n')
                                            .to_owned(),
                                    )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .flex_1()
                .min_h_0()
                .w_full()
                .track_scroll(&self.scroll),
            )
    }
}
