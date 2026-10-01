//! GPUI views, elements and theme. The only crate that may depend on GPUI.

mod platform;
mod settings;
mod sidebar_view;
mod terminal_element;
mod terminal_view;
mod text_input;
mod workspace_view;

use gpui::{
    App, AppContext, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};

pub use settings::Settings;
use terminal_view::{Copy, JumpToNextPrompt, JumpToPrevPrompt, Paste};
use workspace_view::*;

/// Keybindings, following Ghostty's macOS defaults.
fn key_bindings() -> Vec<KeyBinding> {
    let t = Some("Terminal");
    vec![
        KeyBinding::new("cmd-c", Copy, t),
        KeyBinding::new("cmd-v", Paste, t),
        KeyBinding::new("cmd-up", JumpToPrevPrompt, t),
        KeyBinding::new("cmd-down", JumpToNextPrompt, t),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-t", NewTab, None),
        KeyBinding::new("cmd-w", CloseSurface, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("cmd-shift-[", PrevTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("cmd-1", GotoTab1, None),
        KeyBinding::new("cmd-2", GotoTab2, None),
        KeyBinding::new("cmd-3", GotoTab3, None),
        KeyBinding::new("cmd-4", GotoTab4, None),
        KeyBinding::new("cmd-5", GotoTab5, None),
        KeyBinding::new("cmd-6", GotoTab6, None),
        KeyBinding::new("cmd-7", GotoTab7, None),
        KeyBinding::new("cmd-8", GotoTab8, None),
        KeyBinding::new("cmd-9", LastTab, None),
        KeyBinding::new("cmd-d", SplitRight, None),
        KeyBinding::new("cmd-shift-d", SplitDown, None),
        KeyBinding::new("cmd-alt-left", FocusLeft, None),
        KeyBinding::new("cmd-alt-right", FocusRight, None),
        KeyBinding::new("cmd-alt-up", FocusUp, None),
        KeyBinding::new("cmd-alt-down", FocusDown, None),
        KeyBinding::new("cmd-]", NextSplit, None),
        KeyBinding::new("cmd-[", PrevSplit, None),
        KeyBinding::new("cmd-ctrl-left", ResizeLeft, None),
        KeyBinding::new("cmd-ctrl-right", ResizeRight, None),
        KeyBinding::new("cmd-ctrl-up", ResizeUp, None),
        KeyBinding::new("cmd-ctrl-down", ResizeDown, None),
        KeyBinding::new("cmd-ctrl-=", EqualizeSplits, None),
        KeyBinding::new("cmd-shift-enter", ToggleZoom, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-o", AddRepo, None),
        KeyBinding::new("cmd-n", NewWorktree, None),
        KeyBinding::new("escape", Dismiss, None),
    ]
}

/// Start the application and open the main window.
pub fn run(settings: Settings) {
    gpui_platform::application().run(|cx: &mut App| {
        cx.bind_keys(key_bindings());
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("chda".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| WorkspaceView::new(settings, window, cx)),
            )
            .expect("failed to open main window");
        window
            .update(cx, |view, window, cx| view.focus(window, cx))
            .expect("main window vanished");
        cx.activate(true);
    });
}
