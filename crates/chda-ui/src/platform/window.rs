//! Linux client decorations for compositors that do not draw a title bar.

use gpui::{AnyElement, App, Hsla, Window};

pub(crate) fn has_application_menu() -> bool {
    cfg!(target_os = "linux")
}

pub(crate) fn controls(
    window: &Window,
    close: impl Fn(&mut Window, &mut App) + 'static,
) -> Option<AnyElement> {
    #[cfg(target_os = "linux")]
    {
        use gpui::{Decorations, MouseButton, div, prelude::*, px};
        if window.is_fullscreen()
            || !matches!(window.window_decorations(), Decorations::Client { .. })
        {
            return None;
        }
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .w(px(32.0))
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .rounded_sm()
                .hover(|style| style.bg(gpui::rgba(0xffffff18)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .tooltip(crate::tooltip::text(label))
        };
        Some(
            div()
                .flex()
                .flex_row()
                .flex_shrink_0()
                .child(
                    button("window-minimize", "Minimize")
                        .child("−")
                        .on_click(|_, window, _| window.minimize_window()),
                )
                .child(
                    button("window-maximize", "Maximize or restore")
                        .child(if window.is_maximized() { "❐" } else { "□" })
                        .on_click(|_, window, _| window.zoom_window()),
                )
                .child(
                    button("window-close", "Close window")
                        .hover(|style| style.bg(gpui::rgb(0xc42b1c)))
                        .child("×")
                        .on_click(move |_, window, cx| close(window, cx)),
                )
                .into_any_element(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, close);
        None
    }
}

pub(crate) fn frame(
    content: AnyElement,
    window: &mut Window,
    background: Hsla,
    foreground: Hsla,
) -> AnyElement {
    #[cfg(target_os = "linux")]
    {
        use gpui::{CursorStyle, Decorations, MouseButton, ResizeEdge, div, prelude::*, px};
        let resize = matches!(window.window_decorations(), Decorations::Client { tiling } if !tiling.is_tiled())
            && !window.is_fullscreen()
            && !window.is_maximized();
        let inset = if resize { px(5.0) } else { px(0.0) };
        window.set_client_inset(inset);
        if !resize {
            return content;
        }
        let mut frame = div()
            .id("client-window-frame")
            .relative()
            .size_full()
            .p(inset)
            .bg(background)
            .border_1()
            .border_color(foreground.opacity(0.2))
            .child(content);
        for (id, edge, cursor) in [
            ("resize-top", ResizeEdge::Top, CursorStyle::ResizeUpDown),
            (
                "resize-bottom",
                ResizeEdge::Bottom,
                CursorStyle::ResizeUpDown,
            ),
            (
                "resize-left",
                ResizeEdge::Left,
                CursorStyle::ResizeLeftRight,
            ),
            (
                "resize-right",
                ResizeEdge::Right,
                CursorStyle::ResizeLeftRight,
            ),
            (
                "resize-top-left",
                ResizeEdge::TopLeft,
                CursorStyle::ResizeUpLeftDownRight,
            ),
            (
                "resize-top-right",
                ResizeEdge::TopRight,
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                "resize-bottom-left",
                ResizeEdge::BottomLeft,
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                "resize-bottom-right",
                ResizeEdge::BottomRight,
                CursorStyle::ResizeUpLeftDownRight,
            ),
        ] {
            let area = div()
                .id(id)
                .debug_selector(move || id.into())
                .absolute()
                .cursor(cursor)
                .map(|area| match edge {
                    ResizeEdge::Top => area.top_0().left(inset).right(inset).h(inset),
                    ResizeEdge::Bottom => area.bottom_0().left(inset).right(inset).h(inset),
                    ResizeEdge::Left => area.left_0().top(inset).bottom(inset).w(inset),
                    ResizeEdge::Right => area.right_0().top(inset).bottom(inset).w(inset),
                    ResizeEdge::TopLeft => area.left_0().top_0().size(inset),
                    ResizeEdge::TopRight => area.right_0().top_0().size(inset),
                    ResizeEdge::BottomLeft => area.left_0().bottom_0().size(inset),
                    ResizeEdge::BottomRight => area.right_0().bottom_0().size(inset),
                })
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.start_window_resize(edge);
                    cx.stop_propagation();
                });
            frame = frame.child(area);
        }
        frame.into_any_element()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, background, foreground);
        content
    }
}
