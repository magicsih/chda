//! Files and folders dropped from other applications.
//!
//! GPUI's `on_drop` only reaches the element under the mouse, and a window
//! counts nothing as under the mouse while its last input was the keyboard.
//! A drag from Finder does not change that (only real mouse moves do), so
//! after typing, which is the usual state of a terminal, every drop would be
//! lost. Instead, a target remembers the dragged paths from
//! `on_drag_move`, which reaches every listener, and [`catcher`] takes the
//! drop when the mouse is released inside the target's bounds.

use std::path::PathBuf;

use gpui::{
    Context, DispatchPhase, DragMoveEvent, Entity, ExternalPaths, IntoElement, MouseUpEvent,
    Styled, Window, canvas,
};

/// Paths currently dragged over the window, kept by a drop target.
#[derive(Default)]
pub(crate) struct Dragged(Option<Vec<PathBuf>>);

impl Dragged {
    /// Call from `on_drag_move::<ExternalPaths>`.
    pub(crate) fn track(&mut self, event: &DragMoveEvent<ExternalPaths>, cx: &gpui::App) {
        self.0 = Some(event.drag(cx).paths().to_vec());
    }
}

/// An invisible layer filling its (relatively positioned) parent that hands
/// a drop inside it to `on_drop`.
pub(crate) fn catcher<V: 'static>(
    view: Entity<V>,
    dragged: fn(&mut V) -> &mut Dragged,
    on_drop: fn(&mut V, Vec<PathBuf>, &mut Window, &mut Context<V>),
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let view = view.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                // Only external file drags exist in chda, so an active drag
                // at mouse up is the one `dragged` holds.
                if phase != DispatchPhase::Bubble
                    || !cx.has_active_drag()
                    || !bounds.contains(&event.position)
                {
                    return;
                }
                view.update(cx, |v, cx| {
                    if let Some(paths) = dragged(v).0.take()
                        && !paths.is_empty()
                    {
                        on_drop(v, paths, window, cx);
                    }
                });
                cx.stop_propagation();
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
