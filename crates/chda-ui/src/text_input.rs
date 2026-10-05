//! A minimal text field for sheets (branch names, paths, notes). Single
//! line unless `multiline`, where shift-enter starts a new line.

use std::ops::Range;

use gpui::{
    App, Bounds, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Hsla,
    KeyDownEvent, MouseButton, Pixels, Point, Render, UTF16Selection, Window, div, fill,
    prelude::*, px,
};

gpui::actions!(text_input, [SelectAll]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    Submit(String),
    Cancel,
}

pub struct TextInput {
    text: String,
    /// Cursor as a char index.
    cursor: usize,
    anchor: usize,
    bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    marked: Option<Range<usize>>,
    focus_handle: FocusHandle,
    pub placeholder: String,
    pub fg: Hsla,
    pub bg: Hsla,
    /// Shift-enter inserts a line break; enter still submits.
    pub multiline: bool,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: &str, fg: Hsla, bg: Hsla, cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: 0,
            bounds: None,
            scroll_x: px(0.0),
            marked: None,
            focus_handle: cx.focus_handle(),
            placeholder: placeholder.into(),
            fg,
            bg,
            multiline: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replace the text and put the cursor at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.text = text.to_owned();
        self.cursor = self.text.chars().count();
        self.anchor = self.cursor;
        self.marked = None;
        cx.notify();
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn char_to_utf16(&self, index: usize) -> usize {
        self.text.chars().take(index).map(char::len_utf16).sum()
    }

    fn utf16_to_char(&self, index: usize) -> usize {
        let mut units = 0;
        self.text
            .chars()
            .take_while(|c| {
                units += c.len_utf16();
                units <= index
            })
            .count()
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let text = if self.multiline {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.split(['\r', '\n', '\t'])
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let start = self.byte_at(range.start);
        let end = self.byte_at(range.end);
        self.text.replace_range(start..end, &text);
        self.cursor = range.start + text.chars().count();
        self.anchor = self.cursor;
        self.marked = None;
        cx.notify();
    }

    fn insert(&mut self, s: &str, cx: &mut Context<Self>) {
        self.replace(self.selection(), s, cx);
    }

    fn paste(&mut self, _: &crate::terminal_view::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            if !text.is_empty() {
                self.insert(&text, cx);
            }
        }
        cx.stop_propagation();
    }

    fn copy(&mut self, _: &crate::terminal_view::Copy, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection();
        if !range.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                self.text[self.byte_at(range.start)..self.byte_at(range.end)].into(),
            ));
        }
        cx.stop_propagation();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.cursor = self.text.chars().count();
        cx.stop_propagation();
        cx.notify();
    }

    fn clicked(
        &mut self,
        position: Point<Pixels>,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if let Some(bounds) = self.bounds {
            let row = ((position.y - bounds.top()) / window.line_height())
                .floor()
                .max(0.0) as usize;
            let mut start = 0;
            for (index, line) in self.text.split('\n').enumerate() {
                if index == row {
                    let style = window.text_style();
                    let run = gpui::TextRun {
                        len: line.len(),
                        font: style.font(),
                        color: self.fg,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped = window.text_system().shape_line(
                        line.to_owned().into(),
                        style.font_size.to_pixels(window.rem_size()),
                        &[run],
                        None,
                    );
                    let byte =
                        shaped.closest_index_for_x(position.x - bounds.left() + self.scroll_x);
                    self.cursor = start + line[..byte].chars().count();
                    break;
                }
                start += line.chars().count() + 1;
            }
        }
        if !shift {
            self.anchor = self.cursor;
        }
        self.marked = None;
        cx.notify();
        cx.stop_propagation();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if event.prefer_character_input && ks.key_char.is_some() {
            return;
        }
        match ks.key.as_str() {
            "enter" if self.multiline && ks.modifiers.shift => self.insert("\n", cx),
            "enter" => cx.emit(TextInputEvent::Submit(self.text.clone())),
            "escape" => cx.emit(TextInputEvent::Cancel),
            "backspace" => {
                let mut range = self.selection();
                if range.is_empty() && self.cursor > 0 {
                    range.start -= 1;
                }
                self.replace(range, "", cx);
            }
            "delete" => {
                let mut range = self.selection();
                if range.is_empty() {
                    range.end = (range.end + 1).min(self.text.chars().count());
                }
                self.replace(range, "", cx);
            }
            "left" | "right" | "home" | "end" => {
                let selecting = ks.modifiers.shift;
                self.cursor = match ks.key.as_str() {
                    "left" if !selecting && !self.selection().is_empty() => self.selection().start,
                    "right" if !selecting && !self.selection().is_empty() => self.selection().end,
                    "left" => self.cursor.saturating_sub(1),
                    "right" => (self.cursor + 1).min(self.text.chars().count()),
                    "home" => 0,
                    _ => self.text.chars().count(),
                };
                if !selecting {
                    self.anchor = self.cursor;
                }
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
        let range = self.utf16_to_char(range.start)..self.utf16_to_char(range.end);
        Some(self.text[self.byte_at(range.start)..self.byte_at(range.end)].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.char_to_utf16(self.selection().start)
                ..self.char_to_utf16(self.selection().end),
            reversed: self.cursor < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|m| self.char_to_utf16(m.start)..self.char_to_utf16(m.end))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.utf16_to_char(r.start)..self.utf16_to_char(r.end))
            .unwrap_or_else(|| self.marked.clone().unwrap_or_else(|| self.selection()));
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.utf16_to_char(r.start)..self.utf16_to_char(r.end))
            .unwrap_or_else(|| self.marked.clone().unwrap_or_else(|| self.selection()));
        let start = range.start;
        self.replace(range, new_text, cx);
        let end = self.cursor;
        self.marked = (start != end).then_some(start..end);
        if let Some(selected) = selected {
            let offset = self.char_to_utf16(start);
            self.anchor = self.utf16_to_char(offset + selected.start).min(end);
            self.cursor = self.utf16_to_char(offset + selected.end).min(end);
        }
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
        let lines = {
            let i = self.input.read(cx);
            (i.text.lines().count().max(1) + usize::from(i.text.ends_with('\n')))
                .max(if i.multiline { 3 } else { 1 })
        };
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = (window.line_height() * lines as f32).into();
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
        self.input.update(cx, |i, _| i.bounds = Some(bounds));
        let (text, cursor, selection, fg, placeholder) = {
            let i = self.input.read(cx);
            (
                i.text.clone(),
                i.cursor,
                i.selection(),
                i.fg,
                i.placeholder.clone(),
            )
        };
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let shown = text.clone();
        let cursor_byte = shown
            .char_indices()
            .nth(cursor)
            .map(|(i, _)| i)
            .unwrap_or(shown.len());
        let empty = shown.is_empty();
        let display = if empty { placeholder } else { shown };
        let color = if empty { fg.opacity(0.4) } else { fg };
        let font = style.font();
        let line_height = window.line_height();
        // Byte offset of the cursor in `display`, after any marked text.
        let cursor_at = if empty { 0 } else { cursor_byte };
        let selection_bytes =
            text_char_byte(&text, selection.start)..text_char_byte(&text, selection.end);
        let mut start = 0;
        let mut cursor_pos = None;
        for (row, text) in display.split('\n').enumerate() {
            let run = gpui::TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line =
                window
                    .text_system()
                    .shape_line(text.to_owned().into(), font_size, &[run], None);
            let end = start + text.len();
            if self.focus.is_focused(window) && cursor_at >= start && cursor_at <= end {
                let caret_x = line.x_for_index(cursor_at - start);
                self.input.update(cx, |input, _| {
                    input.scroll_x = input
                        .scroll_x
                        .min(caret_x)
                        .max(caret_x - (bounds.size.width - px(2.0)).max(px(0.0)))
                        .max(px(0.0));
                });
            }
            let scroll_x = self.input.read(cx).scroll_x;
            let origin = gpui::point(
                bounds.origin.x - scroll_x,
                bounds.origin.y + line_height * row as f32,
            );
            if !selection.is_empty() && !empty {
                let from = selection_bytes.start.saturating_sub(start).min(text.len());
                let to = selection_bytes.end.saturating_sub(start).min(text.len());
                if from < to {
                    window.paint_quad(fill(
                        Bounds::new(
                            gpui::point(origin.x + line.x_for_index(from), origin.y),
                            gpui::size(line.x_for_index(to) - line.x_for_index(from), line_height),
                        ),
                        fg.opacity(0.2),
                    ));
                }
            }
            let _ = line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx);
            let end = start + text.len();
            if cursor_pos.is_none() && cursor_at <= end {
                let x = if empty {
                    px(0.0)
                } else {
                    line.x_for_index(cursor_at - start)
                };
                cursor_pos = Some(gpui::point(origin.x + x, origin.y));
            }
            start = end + 1;
        }
        if self.focus.is_focused(window)
            && let Some(at) = cursor_pos
        {
            window.paint_quad(fill(Bounds::new(at, gpui::size(px(1.5), line_height)), fg));
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .overflow_hidden()
            .px_2()
            .py_1()
            .bg(self.bg)
            .rounded_sm()
            .text_color(self.fg)
            .track_focus(&self.focus_handle)
            .key_context("TextInput")
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    this.clicked(e.position, e.modifiers.shift, window, cx)
                }),
            )
            .border_1()
            .border_color(if self.focus_handle.is_focused(_window) {
                self.fg.opacity(0.65)
            } else {
                self.fg.opacity(0.18)
            })
            .child(Field {
                input: cx.entity(),
                focus: self.focus_handle.clone(),
            })
    }
}

fn text_char_byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}
