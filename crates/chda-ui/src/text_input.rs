//! A minimal single-line text field for sheets (branch names, paths).

use std::ops::Range;

use gpui::{
    App, Bounds, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Hsla,
    KeyDownEvent, Pixels, Point, Render, UTF16Selection, Window, div, fill, prelude::*, px,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    Submit(String),
    Cancel,
}

pub struct TextInput {
    text: String,
    /// Cursor as a char index.
    cursor: usize,
    marked: Option<String>,
    focus_handle: FocusHandle,
    pub placeholder: String,
    pub fg: Hsla,
    pub bg: Hsla,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: &str, fg: Hsla, bg: Hsla, cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            marked: None,
            focus_handle: cx.focus_handle(),
            placeholder: placeholder.into(),
            fg,
            bg,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    fn insert(&mut self, s: &str, cx: &mut Context<Self>) {
        let at = self.byte_at(self.cursor);
        self.text.insert_str(at, s);
        self.cursor += s.chars().count();
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if event.prefer_character_input && ks.key_char.is_some() {
            return;
        }
        match ks.key.as_str() {
            "enter" => cx.emit(TextInputEvent::Submit(self.text.clone())),
            "escape" => cx.emit(TextInputEvent::Cancel),
            "backspace" => {
                if self.cursor > 0 {
                    let end = self.byte_at(self.cursor);
                    let start = self.byte_at(self.cursor - 1);
                    self.text.replace_range(start..end, "");
                    self.cursor -= 1;
                    cx.notify();
                }
            }
            "left" => {
                self.cursor = self.cursor.saturating_sub(1);
                cx.notify();
            }
            "right" => {
                self.cursor = (self.cursor + 1).min(self.text.chars().count());
                cx.notify();
            }
            "home" => {
                self.cursor = 0;
                cx.notify();
            }
            "end" => {
                self.cursor = self.text.chars().count();
                cx.notify();
            }
            _ => {
                if ks.modifiers.control || ks.modifiers.platform || ks.modifiers.alt {
                    return;
                }
                // Keys that type nothing (up, down, tab, ...) go on to the
                // field's container, such as the palette's list.
                let Some(c) = ks
                    .key_char
                    .as_deref()
                    .filter(|c| !c.chars().any(char::is_control))
                else {
                    return;
                };
                let c = c.to_owned();
                self.insert(&c, cx);
            }
        }
        cx.stop_propagation();
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let chars: Vec<char> = self.text.chars().collect();
        Some(chars.get(range)?.iter().collect())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.cursor..self.cursor,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|m| self.cursor..self.cursor + m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        self.insert(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct Field {
    input: gpui::Entity<TextInput>,
    focus: FocusHandle,
}

impl IntoElement for Field {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for Field {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.set_focus_handle(&self.focus, cx);
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            gpui::ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let (text, cursor, marked, fg, placeholder) = {
            let i = self.input.read(cx);
            (
                i.text.clone(),
                i.cursor,
                i.marked.clone(),
                i.fg,
                i.placeholder.clone(),
            )
        };
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let mut shown = text.clone();
        let cursor_byte = shown
            .char_indices()
            .nth(cursor)
            .map(|(i, _)| i)
            .unwrap_or(shown.len());
        if let Some(m) = &marked {
            shown.insert_str(cursor_byte, m);
        }
        let empty = shown.is_empty();
        let display = if empty { placeholder } else { shown };
        let color = if empty { fg.opacity(0.4) } else { fg };
        let run = gpui::TextRun {
            len: display.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(display.into(), font_size, &[run], None);
        let _ = line.paint(
            bounds.origin,
            window.line_height(),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
        if self.focus.is_focused(window) {
            let x = if empty {
                px(0.0)
            } else {
                line.x_for_index(cursor_byte + marked.map(|m| m.len()).unwrap_or(0))
            };
            window.paint_quad(fill(
                Bounds::new(
                    gpui::point(bounds.origin.x + x, bounds.origin.y),
                    gpui::size(px(1.5), bounds.size.height),
                ),
                fg,
            ));
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .px_2()
            .py_1()
            .bg(self.bg)
            .rounded_sm()
            .text_color(self.fg)
            .track_focus(&self.focus_handle)
            .key_context("TextInput")
            .on_key_down(cx.listener(Self::key_down))
            .child(Field {
                input: cx.entity(),
                focus: self.focus_handle.clone(),
            })
    }
}
