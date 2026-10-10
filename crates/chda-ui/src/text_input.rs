//! A minimal text field for sheets (branch names, paths, notes). Single
//! line unless `multiline`, where shift-enter starts a new line. Long lines
//! scroll sideways unless `soft_wrap` wraps them at the field's width.

use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    Font, Hsla, KeyDownEvent, MouseButton, Pixels, Point, Render, UTF16Selection, Window,
    WrappedLine, div, fill, point, prelude::*, px,
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
    painted: Option<Painted>,
    scroll_x: Pixels,
    marked: Option<Range<usize>>,
    focus_handle: FocusHandle,
    pub placeholder: String,
    pub fg: Hsla,
    pub bg: Hsla,
    /// Shift-enter inserts a line break; enter still submits.
    pub multiline: bool,
    /// Wrap long lines at the field's width instead of scrolling sideways.
    /// Display only: the text keeps exactly its own line breaks.
    pub soft_wrap: bool,
}

/// What the field was last painted with, so clicks land on the same glyphs.
struct Painted {
    bounds: Bounds<Pixels>,
    font: Font,
    font_size: Pixels,
    line_height: Pixels,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: &str, fg: Hsla, bg: Hsla, cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: 0,
            painted: None,
            scroll_x: px(0.0),
            marked: None,
            focus_handle: cx.focus_handle(),
            placeholder: placeholder.into(),
            fg,
            bg,
            multiline: false,
            soft_wrap: false,
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

    /// Where the first character was last painted and how many rows the
    /// field showed, for scenarios that see no pixels.
    #[cfg(test)]
    pub(crate) fn painted_text(&self) -> Option<(Point<Pixels>, usize)> {
        let p = self.painted.as_ref()?;
        Some((
            point(p.bounds.left() - self.scroll_x, p.bounds.top()),
            (p.bounds.size.height / p.line_height).round() as usize,
        ))
    }

    /// The text as the field shows it, the placeholder while empty: one
    /// shaped entry per line, wrapped at `width` when `soft_wrap` is set.
    fn shape(
        &self,
        font: &Font,
        font_size: Pixels,
        width: Option<Pixels>,
        window: &Window,
    ) -> Vec<WrappedLine> {
        let (text, color) = if self.text.is_empty() {
            (&self.placeholder, self.fg.opacity(0.4))
        } else {
            (&self.text, self.fg)
        };
        let run = gpui::TextRun {
            len: text.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_text(
                text.clone().into(),
                font_size,
                &[run],
                width.filter(|_| self.soft_wrap),
                None,
            )
            .map(|lines| lines.into_vec())
            .unwrap_or_default()
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
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text())
            && !text.is_empty()
        {
            self.insert(&text, cx);
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
        if let Some(p) = &self.painted
            && !self.text.is_empty()
        {
            let lines = self.shape(&p.font, p.font_size, Some(p.bounds.size.width), window);
            let x = position.x - p.bounds.left() + self.scroll_x;
            let mut y = position.y - p.bounds.top();
            let mut start = 0;
            for line in &lines {
                let height = line.size(p.line_height).height;
                if y < height {
                    let byte = line
                        .closest_index_for_position(point(x, y), p.line_height)
                        .unwrap_or_else(|end| end);
                    self.cursor = self.text[..start + byte].chars().count();
                    break;
                }
                y -= height;
                start += line.len() + 1;
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
        _: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let text_style = window.text_style();
        let font = text_style.font();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let input = self.input.clone();
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.0).into();
        // Wrapped rows depend on the width the layout gives the field.
        let layout = window.request_measured_layout(style, move |known, available, window, cx| {
            let i = input.read(cx);
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let rows: usize = i
                .shape(&font, font_size, width, window)
                .iter()
                .map(|line| line.wrap_boundaries.len() + 1)
                .sum();
            gpui::size(
                known.width.unwrap_or_default(),
                line_height * rows.max(if i.multiline { 3 } else { 1 }) as f32,
            )
        });
        (layout, ())
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
        let text_style = window.text_style();
        let painted = Painted {
            bounds,
            font: text_style.font(),
            font_size: text_style.font_size.to_pixels(window.rem_size()),
            line_height: window.line_height(),
        };
        let line_height = painted.line_height;
        let focused = self.focus.is_focused(window);
        let (lines, text, cursor, selection, fg) = {
            let i = self.input.read(cx);
            (
                i.shape(
                    &painted.font,
                    painted.font_size,
                    Some(bounds.size.width),
                    window,
                ),
                i.text.clone(),
                i.cursor,
                i.selection(),
                i.fg,
            )
        };
        let empty = text.is_empty();
        // Byte offset of the cursor in the shown text.
        let cursor_at = if empty {
            0
        } else {
            text_char_byte(&text, cursor)
        };
        let selection_bytes =
            text_char_byte(&text, selection.start)..text_char_byte(&text, selection.end);
        // Each line's top and byte offset, and the caret, before scrolling.
        let mut placed = Vec::with_capacity(lines.len());
        let mut caret = None;
        let (mut top, mut start) = (px(0.0), 0);
        for line in &lines {
            if caret.is_none() && cursor_at <= start + line.len() {
                caret = line
                    .position_for_index(cursor_at - start, line_height)
                    .map(|at| point(at.x, top + at.y));
            }
            placed.push((top, start));
            top += line.size(line_height).height;
            start += line.len() + 1;
        }
        let scroll_x = self.input.update(cx, |input, _| {
            if input.soft_wrap {
                input.scroll_x = px(0.0);
            } else if focused && let Some(caret) = caret {
                input.scroll_x = input
                    .scroll_x
                    .min(caret.x)
                    .max(caret.x - (bounds.size.width - px(2.0)).max(px(0.0)))
                    .max(px(0.0));
            }
            input.painted = Some(painted);
            input.scroll_x
        });
        let origin = point(bounds.origin.x - scroll_x, bounds.origin.y);
        for (line, (top, start)) in lines.iter().zip(placed) {
            let line_origin = point(origin.x, origin.y + top);
            if !selection.is_empty() && !empty {
                let layout = &line.unwrapped_layout;
                for (row, range) in rows(line).into_iter().enumerate() {
                    let from = selection_bytes
                        .start
                        .saturating_sub(start)
                        .clamp(range.start, range.end);
                    let to = selection_bytes
                        .end
                        .saturating_sub(start)
                        .clamp(range.start, range.end);
                    if from < to {
                        let left = layout.x_for_index(range.start);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(
                                    line_origin.x + layout.x_for_index(from) - left,
                                    line_origin.y + line_height * row as f32,
                                ),
                                gpui::size(
                                    layout.x_for_index(to) - layout.x_for_index(from),
                                    line_height,
                                ),
                            ),
                            fg.opacity(0.2),
                        ));
                    }
                }
            }
            let _ = line.paint(
                line_origin,
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        if focused && let Some(at) = caret {
            window.paint_quad(fill(
                Bounds::new(
                    point(origin.x + at.x, origin.y + at.y),
                    gpui::size(px(1.5), line_height),
                ),
                fg,
            ));
        }
    }
}

/// Byte ranges of the rows a shaped line wraps into.
fn rows(line: &WrappedLine) -> Vec<Range<usize>> {
    let mut starts: Vec<usize> = line
        .wrap_boundaries
        .iter()
        .map(|b| line.unwrapped_layout.runs[b.run_ix].glyphs[b.glyph_ix].index)
        .collect();
    starts.insert(0, 0);
    let ends = starts.iter().skip(1).copied().chain([line.len()]);
    starts
        .iter()
        .copied()
        .zip(ends)
        .map(|(a, b)| a..b)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn clicks_land_on_soft_wrapped_rows_without_changing_the_text(cx: &mut gpui::TestAppContext) {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let (input, cx) = cx.add_window_view(|_, cx| {
            let mut input = TextInput::new("Note", gpui::white(), gpui::black(), cx);
            input.multiline = true;
            input.soft_wrap = true;
            input.set_text(text, cx);
            input
        });
        cx.simulate_resize(gpui::size(px(200.0), px(400.0)));
        cx.run_until_parked();
        let (bounds, line_height) = input.read_with(cx, |i, _| {
            let p = i.painted.as_ref().unwrap();
            (p.bounds, p.line_height)
        });
        assert!(
            bounds.size.height >= line_height * 3.0,
            "the line wraps onto several rows"
        );
        cx.simulate_click(
            point(bounds.left() + px(1.0), bounds.top() + line_height * 1.5),
            gpui::Modifiers::none(),
        );
        input.read_with(cx, |i, _| {
            assert_eq!(i.text(), text);
            assert_eq!(i.scroll_x, px(0.0));
            assert!(
                i.cursor > 0 && text[..i.cursor].ends_with(' '),
                "a click at the second row's start lands after the wrap, not at {}",
                i.cursor
            );
        });
    }

    #[gpui::test]
    fn composition_replaces_the_selected_unicode_range_and_commits_once(
        cx: &mut gpui::TestAppContext,
    ) {
        let handle =
            cx.add_window(|_, cx| TextInput::new("Note", gpui::white(), gpui::black(), cx));
        handle
            .update(cx, |input, window, cx| {
                input.multiline = true;
                input.set_text("A🧭Z", cx);
                input.replace_and_mark_text_in_range(Some(1..3), "ㅎ", Some(1..1), window, cx);
                assert_eq!(input.text(), "AㅎZ");
                assert_eq!(input.marked_text_range(window, cx), Some(1..2));
                input.replace_and_mark_text_in_range(None, "한", Some(1..1), window, cx);
                assert_eq!(
                    input.text(),
                    "A한Z",
                    "composition replaces, never duplicates, its old text"
                );
                input.replace_text_in_range(None, "한국", window, cx);
                assert_eq!(input.text(), "A한국Z");
                assert_eq!(input.marked_text_range(window, cx), None);
                assert_eq!(
                    input.selected_text_range(false, window, cx).unwrap().range,
                    3..3
                );
                input.replace_text_in_range(Some(1..3), "🧭\nNote", window, cx);
                assert_eq!(input.text(), "A🧭\nNoteZ");
            })
            .unwrap();
    }
}
