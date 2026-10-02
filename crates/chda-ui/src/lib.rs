//! GPUI views, elements and theme. The only crate that may depend on GPUI.

mod diagram;
mod environment;
mod external_drop;
mod fonts;
mod link_menu;
mod menus;
mod palette;
mod platform;
#[cfg(test)]
mod scenarios;
mod settings;
mod sidebar_view;
mod terminal_element;
mod terminal_images;
mod terminal_view;
mod text_input;
mod tooltip;
mod workspace_view;

use gpui::{
    App, AppContext, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, point, px,
    size,
};

pub use settings::Settings;
use terminal_view::{
    CloseFind, Copy, Find, FindNext, FindPrevious, JumpToNextPrompt, JumpToPrevPrompt, Paste,
    ToggleFindCase, ToggleFindRegex,
};
use workspace_view::*;

/// Keybindings, following Ghostty's macOS defaults.
fn key_bindings() -> Vec<KeyBinding> {
    let t = Some("Terminal");
    vec![
        KeyBinding::new("cmd-c", Copy, t),
        KeyBinding::new("cmd-v", Paste, t),
        KeyBinding::new("cmd-up", JumpToPrevPrompt, t),
        KeyBinding::new("cmd-down", JumpToNextPrompt, t),
        KeyBinding::new("cmd-f", Find, t),
        KeyBinding::new("cmd-g", FindNext, t),
        KeyBinding::new("cmd-shift-g", FindPrevious, t),
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
        KeyBinding::new("cmd-shift-p", TogglePalette, None),
        KeyBinding::new("escape", Dismiss, None),
        KeyBinding::new("cmd-=", IncreaseFontSize, None),
        KeyBinding::new("cmd-+", IncreaseFontSize, None),
        KeyBinding::new("cmd--", DecreaseFontSize, None),
        KeyBinding::new("cmd-0", ResetFontSize, None),
        KeyBinding::new("cmd-shift-a", GoToWaitingAgent, None),
        KeyBinding::new("cmd-,", OpenConfig, None),
        KeyBinding::new("cmd-shift-,", ReloadConfig, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-h", menus::Hide, None),
        KeyBinding::new("cmd-alt-h", menus::HideOthers, None),
        // Global bindings count as matching at the focused element's depth,
        // so these name the focused field and come last: on equal depth the
        // later binding wins over the global escape (Dismiss).
        KeyBinding::new("enter", FindNext, Some("SearchBar > TextInput")),
        KeyBinding::new("shift-enter", FindPrevious, Some("SearchBar > TextInput")),
        KeyBinding::new("escape", CloseFind, Some("SearchBar > TextInput")),
        KeyBinding::new("alt-c", ToggleFindCase, Some("SearchBar > TextInput")),
        KeyBinding::new("alt-r", ToggleFindRegex, Some("SearchBar > TextInput")),
    ]
}

/// Start the application and open the main window with the user's Ghostty
/// config.
pub fn run(ghostty: chda_config::GhosttyConfig) {
    gpui_platform::application().run(|cx: &mut App| {
        fonts::register(cx);
        cx.bind_keys(key_bindings());
        cx.on_action(|_: &Quit, cx| cx.quit());
        menus::install(cx);

        let env = std::rc::Rc::new(environment::Environment::for_user());
        let config = env.load_config();
        let saved = config
            .restore_session
            .then(|| {
                env.data_dir
                    .as_deref()
                    .and_then(chda_core::SavedWindow::load)
            })
            .flatten();
        let bounds = match saved.as_ref().and_then(|s| s.bounds) {
            Some(b) => Bounds::new(point(px(b.x), px(b.y)), size(px(b.width), px(b.height))),
            None => Bounds::centered(None, size(px(960.0), px(640.0)), cx),
        };
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
                |window, cx| cx.new(|cx| WorkspaceView::new(ghostty, saved, env, window, cx)),
            )
            .expect("failed to open main window");
        window
            .update(cx, |view, window, cx| view.focus(window, cx))
            .expect("main window vanished");
        cx.activate(true);
    });
}
