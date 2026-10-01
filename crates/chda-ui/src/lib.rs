//! GPUI views, elements and theme. The only crate that may depend on GPUI.

mod platform;
mod settings;
mod terminal_element;
mod terminal_view;

use gpui::{
    App, AppContext, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};

pub use settings::Settings;
use terminal_view::{Copy, Paste, Quit, TerminalView};

/// Start the application and open the main window.
pub fn run(settings: Settings) {
    gpui_platform::application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
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
                |window, cx| cx.new(|cx| TerminalView::new(settings, window, cx)),
            )
            .expect("failed to open main window");
        window
            .update(cx, |view, window, cx| view.focus(window, cx))
            .expect("main window vanished");
        cx.activate(true);
    });
}
