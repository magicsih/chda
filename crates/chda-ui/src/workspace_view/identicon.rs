//! The enlarged session identicon (#272): a small popover over the window,
//! opened by a click on a Sessions row's picture or from the palette.

use super::*;
use chda_core::SessionIdentity;

/// Side of one cell in the preview: five of them are 160px.
const PREVIEW_CELL: f32 = 32.0;

/// A conversation's identicon shown large, with its provider and full id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdenticonPreview {
    pub identity: SessionIdentity,
    /// Where the popover's corner goes, e.g. where the icon was clicked.
    pub position: Point<Pixels>,
}

impl WorkspaceView {
    /// The conversation of the Sessions row holding the focus, once its
    /// provider has reported the session id.
    pub(super) fn focused_identity(&self, cx: &App) -> Option<SessionIdentity> {
        let sidebar = self.sidebar.read(cx);
        let key = sidebar.focused_session()?;
        sidebar
            .model
            .sessions
            .iter()
            .find(|row| row.key == key)?
            .identity
            .clone()
    }

    pub(super) fn open_identicon(
        &mut self,
        identity: SessionIdentity,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::tooltip::dismiss(window, cx);
        if self.identicon_preview.is_none() {
            self.identicon_origin = self.input_origin();
        }
        self.identicon_preview = Some(IdenticonPreview { identity, position });
        // Escape reaches the window, wherever the click left the focus.
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(super) fn close_identicon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.identicon_preview.take().is_none() {
            return;
        }
        let origin = self.identicon_origin.take();
        // Keep a pane or tab the user chose while the popover was open.
        if self.focus_handle.is_focused(window) {
            match origin {
                Some(focus) => self.restore_close_focus(focus, window, cx),
                None => self.refocus_terminal(window, cx),
            }
        }
        cx.notify();
    }

    /// The palette's "Show session icon", which has no pointer to anchor at:
    /// the popover opens near the middle of the window.
    pub(super) fn show_focused_session_icon(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_focus(window, cx);
        let Some(identity) = self.focused_identity(cx) else {
            return;
        };
        let size = window.viewport_size();
        let position = point(size.width * 0.5 - px(160.0), size.height * 0.5 - px(140.0));
        self.open_identicon(identity, position, window, cx);
    }

    pub(super) fn render_identicon_preview(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let preview = self.identicon_preview.as_ref()?;
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let icon = preview.identity.identicon();
        let color = crate::sidebar_view::identicon_color(&icon, fg, bg);
        let provider = self
            .sidebar
            .read(cx)
            .agents
            .iter()
            .find(|agent| agent.id == preview.identity.provider)
            .map_or_else(|| preview.identity.provider.clone(), |a| a.name.clone());
        let panel = div()
            .id("identicon-preview")
            .debug_selector(|| "identicon-preview".into())
            .w(px(320.0))
            .p_4()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .bg(bg)
            .text_color(fg)
            .rounded_md()
            .border_1()
            .border_color(fg.opacity(0.2))
            .shadow_lg()
            // A click inside the popover is not a click outside it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .debug_selector(|| "identicon-preview-icon".into())
                    .child(crate::sidebar_view::identicon_element(
                        &icon,
                        px(PREVIEW_CELL),
                        color,
                        fg.opacity(0.08),
                    )),
            )
            .child(
                div()
                    .debug_selector(|| "identicon-preview-provider".into())
                    .text_sm()
                    .child(provider),
            )
            .child(
                div()
                    .debug_selector(|| "identicon-preview-id".into())
                    .text_xs()
                    .text_color(fg.opacity(0.8))
                    .child(preview.identity.id.clone()),
            )
            .child(
                div()
                    .id("identicon-preview-close")
                    .debug_selector(|| "identicon-preview-close".into())
                    .cursor_pointer()
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(fg.opacity(0.3))
                    .hover(|s| s.bg(fg.opacity(0.12)))
                    .on_click(cx.listener(|this, _, window, cx| this.close_identicon(window, cx)))
                    .child("Close"),
            );
        Some(
            deferred(
                // Blocks what is under the popover, so hovered rows show no
                // tooltip over it and a click outside only dismisses it.
                div()
                    .absolute()
                    .size_full()
                    .top_0()
                    .left_0()
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_identicon(window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, window, cx| {
                            this.close_identicon(window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        anchored()
                            .position(preview.position)
                            .snap_to_window()
                            .child(panel),
                    ),
            )
            .into_any_element(),
        )
    }
}
