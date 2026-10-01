//! A view that owns one terminal session and forwards input to it.

use std::sync::Arc;
use std::sync::mpsc;

use chda_term::{
    Event, Frame, KeyAction, KeyCode, KeyInput, Modifiers, Session, SessionOptions, Size,
};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui::{
    App, ClipboardItem, Context, FocusHandle, Focusable, Font, FontFeatures, FontStyle, FontWeight,
    KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, Pixels, Render, ScrollWheelEvent,
    TouchPhase, Window, actions, div, prelude::*, px,
};

use crate::terminal_element::TerminalElement;

actions!(terminal, [Paste, Quit]);

/// Font used for the cell grid. Read from the Ghostty config later (M1).
#[derive(Clone, Debug)]
pub struct FontConfig {
    pub family: String,
    pub size: Pixels,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "Menlo".into(),
            size: px(13.0),
        }
    }
}

impl FontConfig {
    pub fn font(&self) -> Font {
        Font {
            family: self.family.clone().into(),
            features: FontFeatures::disable_ligatures(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }
}

pub struct TerminalView {
    session: Session,
    events: mpsc::Receiver<Event>,
    frame: Arc<Frame>,
    focus_handle: FocusHandle,
    pub font: FontConfig,
    /// Grid size last sent to the session.
    grid: Size,
    scroll_px: f32,
}

impl TerminalView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (events_tx, events) = mpsc::channel();
        let (wake_tx, wake_rx) = unbounded::<()>();
        let session = Session::spawn(SessionOptions::default(), events_tx, move || {
            let _ = wake_tx.unbounded_send(());
        })
        .expect("failed to start the shell");
        let frame = session.frame();

        Self::drive(wake_rx, window, cx);

        let focus_handle = cx.focus_handle();
        cx.on_focus(&focus_handle, window, |this, _, _| this.session.focus(true))
            .detach();
        cx.on_blur(&focus_handle, window, |this, _, _| {
            this.session.focus(false)
        })
        .detach();

        Self {
            session,
            events,
            frame,
            focus_handle,
            font: FontConfig::default(),
            grid: Size { cols: 80, rows: 24 },
            scroll_px: 0.0,
        }
    }

    /// Pump session events into the view whenever the terminal thread wakes us.
    fn drive(mut wake_rx: UnboundedReceiver<()>, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            while wake_rx.next().await.is_some() {
                if this
                    .update_in(cx, |view, window, cx| view.drain_events(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn drain_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut exited = false;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Frame => {
                    self.frame = self.session.frame();
                    cx.notify();
                }
                Event::TitleChanged(title) => {
                    let title = if title.is_empty() { "chda" } else { &title };
                    window.set_window_title(title);
                }
                Event::PwdChanged(_) | Event::Bell => {}
                Event::ClipboardWrite(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text))
                }
                Event::Exited(_) => exited = true,
            }
        }
        if exited {
            cx.quit();
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    pub fn frame(&self) -> Arc<Frame> {
        Arc::clone(&self.frame)
    }

    /// Called by the element when the grid that fits the bounds changes.
    pub fn set_grid(&mut self, grid: Size, cell_width: Pixels, line_height: Pixels) {
        if grid == self.grid {
            return;
        }
        self.grid = grid;
        self.session.resize(
            grid,
            f32::from(cell_width).round() as u32,
            f32::from(line_height).round() as u32,
        );
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        // Command shortcuts belong to the app, never to the shell.
        if ks.modifiers.platform {
            return;
        }
        let Some(key) = key_code(ks) else {
            return;
        };
        let text = ks
            .key_char
            .as_deref()
            .filter(|t| {
                t.chars()
                    .all(|c| !c.is_control() && !('\u{E000}'..='\u{F8FF}').contains(&c))
            })
            .map(str::to_owned);
        self.session.key(KeyInput {
            action: if event.is_held {
                KeyAction::Repeat
            } else {
                KeyAction::Press
            },
            key,
            mods: Modifiers {
                shift: ks.modifiers.shift,
                alt: ks.modifiers.alt,
                ctrl: ks.modifiers.control,
                meta: false,
            },
            text,
        });
        cx.stop_propagation();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.session.paste(text);
        }
    }

    fn mouse_down(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window, cx);
    }

    fn scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        let line_height = window.line_height();
        match event.touch_phase {
            TouchPhase::Started => self.scroll_px = 0.0,
            TouchPhase::Ended | TouchPhase::Cancelled => return,
            TouchPhase::Moved => {}
        }
        let before = (self.scroll_px / f32::from(line_height)) as i32;
        self.scroll_px += f32::from(event.delta.pixel_delta(line_height).y);
        let after = (self.scroll_px / f32::from(line_height)) as i32;
        let lines = after - before;
        if lines != 0 {
            // Wheel up (positive delta) moves the viewport towards history.
            self.session.scroll(-(lines as isize));
        }
    }
}

/// Map a GPUI keystroke to the terminal's logical key.
fn key_code(ks: &Keystroke) -> Option<KeyCode> {
    let key = ks.key.as_str();
    Some(match key {
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Escape,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Space,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::ArrowUp,
        "down" => KeyCode::ArrowDown,
        "left" => KeyCode::ArrowLeft,
        "right" => KeyCode::ArrowRight,
        _ => {
            if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
                && key.len() > 1
            {
                return Some(KeyCode::F(n));
            }
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return None,
            }
        }
    })
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(TerminalElement::new(cx.entity(), self.focus_handle.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(key: &str) -> Keystroke {
        Keystroke {
            modifiers: Default::default(),
            key: key.into(),
            key_char: None,
        }
    }

    #[test]
    fn keystroke_names_map_to_key_codes() {
        assert_eq!(key_code(&ks("a")), Some(KeyCode::Char('a')));
        assert_eq!(key_code(&ks("/")), Some(KeyCode::Char('/')));
        assert_eq!(key_code(&ks("enter")), Some(KeyCode::Enter));
        assert_eq!(key_code(&ks("pageup")), Some(KeyCode::PageUp));
        assert_eq!(key_code(&ks("f12")), Some(KeyCode::F(12)));
        assert_eq!(key_code(&ks("f")), Some(KeyCode::Char('f')));
        assert_eq!(key_code(&ks("capslock")), None);
    }
}
