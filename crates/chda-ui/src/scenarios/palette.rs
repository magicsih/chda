//! Command palette: moving the selection, and picking a theme.

use chda_term::Rgb;
use gpui::TestAppContext;

use super::harness::Harness;
use crate::workspace_view::SelectTheme;

const DEFAULT_BACKGROUND: Rgb = Rgb {
    r: 0x28,
    g: 0x2c,
    b: 0x34,
};

fn background(h: &Harness) -> Option<Rgb> {
    h.read(|v, _| v.settings.colors.background)
}

#[gpui::test]
fn arrow_keys_choose_a_later_entry(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "palette", |_| {});
    h.wait_prompt();
    // New tab, Close pane, Split right: two steps down splits the pane.
    h.keys("cmd-shift-p");
    h.keys("down down enter");
    h.read(|v, _| {
        assert!(v.palette.is_none());
        assert_eq!(v.ws.tabs().len(), 1);
        assert_eq!(v.ws.active_tab().unwrap().panes().len(), 2);
    });
}

#[gpui::test]
fn theme_picker_previews_restores_and_saves(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "theme", |_| {});
    h.wait_prompt();
    assert_eq!(background(&h), Some(DEFAULT_BACKGROUND));

    // Reached from the palette; the selection starts on the Ghostty config.
    h.keys("cmd-shift-p");
    h.type_text("Select theme");
    h.keys("enter");
    assert!(h.read(|v, _| v.palette.is_some()));
    h.keys("down");
    let previewed = background(&h);
    assert_ne!(previewed, Some(DEFAULT_BACKGROUND));
    h.read(|v, cx| {
        assert!(
            v.panes
                .values()
                .all(|(p, _)| p.read(cx).settings.colors.background == previewed)
        );
    });
    h.keys("escape");
    assert_eq!(background(&h), Some(DEFAULT_BACKGROUND));
    assert_eq!(h.read(|v, _| v.config.theme.clone()), None);

    h.cx.dispatch_action(SelectTheme);
    h.type_text("Catppuccin Mocha");
    h.keys("enter");
    let mocha = Some(Rgb {
        r: 0x1e,
        g: 0x1e,
        b: 0x2e,
    });
    assert_eq!(background(&h), mocha);
    assert_eq!(
        h.read(|v, _| v.config.theme.clone()).as_deref(),
        Some("Catppuccin Mocha")
    );
    let saved = std::fs::read_to_string(&h.home.config).unwrap();
    assert!(saved.contains("theme = \"Catppuccin Mocha\""), "{saved}");

    // The saved theme survives the reload its own write triggers.
    h.wait_for("config.toml reloaded", |v, _| {
        v.settings.colors.background == mocha && v.config.theme.is_some()
    });
}
