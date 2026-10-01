//! Command palette: fuzzy-filtered actions, worktrees and sessions.

use std::path::PathBuf;

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, Render,
    Subscription, Window, div, prelude::*, px,
};

use crate::text_input::{TextInput, TextInputEvent};

/// What a palette entry does when chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteCommand {
    /// A keyboard action by name, dispatched by the workspace.
    Action(&'static str),
    GoToWorktree(PathBuf),
    RunAgent(PathBuf, String),
    ResumeSession {
        worktree: PathBuf,
        agent: String,
        session: String,
    },
    NewWorktree(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaletteItem {
    pub label: String,
    pub detail: String,
    pub command: PaletteCommand,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaletteEvent {
    Chosen(PaletteCommand),
    Dismissed,
}

pub struct Palette {
    items: Vec<PaletteItem>,
    input: Entity<TextInput>,
    _sub: Subscription,
    selected: usize,
    fg: Hsla,
    bg: Hsla,
}

impl EventEmitter<PaletteEvent> for Palette {}

/// Subsequence match with a score: lower is better; `None` means no match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0u32;
    let mut pos = 0usize;
    let mut last = None;
    for q in query.to_lowercase().chars() {
        let found = text[pos..].iter().position(|c| *c == q)? + pos;
        if let Some(l) = last {
            score += (found - l - 1) as u32;
        } else {
            score += found as u32;
        }
        last = Some(found);
        pos = found + 1;
    }
    Some(score)
}

impl Palette {
    pub fn new(
        items: Vec<PaletteItem>,
        fg: Hsla,
        bg: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new("Type a command, worktree or session", fg, bg, cx));
        let sub = cx.subscribe(&input, |this, _, event, cx| match event {
            TextInputEvent::Submit(_) => this.choose(cx),
            TextInputEvent::Cancel => cx.emit(PaletteEvent::Dismissed),
        });
        cx.observe(&input, |this, _, cx| {
            this.selected = 0;
            cx.notify();
        })
        .detach();
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        Self {
            items,
            input,
            _sub: sub,
            selected: 0,
            fg,
            bg,
        }
    }

    fn matches(&self, cx: &App) -> Vec<(u32, &PaletteItem)> {
        let query = self.input.read(cx).text().to_owned();
        let mut out: Vec<(u32, &PaletteItem)> = self
            .items
            .iter()
            .filter_map(|i| {
                fuzzy_score(&query, &format!("{} {}", i.label, i.detail)).map(|s| (s, i))
            })
            .collect();
        out.sort_by_key(|(s, _)| *s);
        out.truncate(12);
        out
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        let matches = self.matches(cx);
        if let Some((_, item)) = matches.get(self.selected) {
            let command = item.command.clone();
            cx.emit(PaletteEvent::Chosen(command));
        }
    }

    pub fn move_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        let n = self.matches(cx).len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).rem_euclid(n as i32) as usize;
        cx.notify();
    }

    pub fn input_focus(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input_focus(cx)
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = self.fg;
        let rows: Vec<AnyElement> = self
            .matches(cx)
            .into_iter()
            .enumerate()
            .map(|(i, (_, item))| {
                let selected = i == self.selected;
                div()
                    .id(("palette-item", i))
                    .flex()
                    .flex_row()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(fg.opacity(0.15)))
                    .hover(|s| s.bg(fg.opacity(0.1)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = i;
                        this.choose(cx);
                    }))
                    .child(div().flex_shrink_0().child(item.label.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(fg.opacity(0.5))
                            .child(item.detail.clone()),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .w(px(560.0))
            .p_2()
            .rounded_md()
            .bg(self.bg)
            .border_1()
            .border_color(fg.opacity(0.2))
            .shadow_lg()
            .text_sm()
            .text_color(fg)
            .flex()
            .flex_col()
            .gap_1()
            .key_context("Palette")
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "down" => this.move_selection(1, cx),
                    "up" => this.move_selection(-1, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(self.input.clone())
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_tight_matches() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert_eq!(fuzzy_score("nt", "New Tab"), Some(3));
        assert!(
            fuzzy_score("spl", "Split right").unwrap()
                < fuzzy_score("spl", "Some pale line").unwrap()
        );
        assert_eq!(fuzzy_score("zz", "New Tab"), None);
    }
}
