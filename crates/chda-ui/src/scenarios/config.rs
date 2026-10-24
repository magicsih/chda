//! Live config reload.

use gpui::{TestAppContext, px};

use super::harness::Harness;

#[gpui::test]
fn ghostty_and_chda_config_changes_apply_live(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "config", |home| {
        std::fs::write(&home.ghostty, "font-size = 13\n").unwrap();
    });
    h.wait_prompt();
    std::fs::write(&h.home.ghostty, "font-size = 18\nbackground = #102030\n").unwrap();
    h.wait_for("the new font size in the pane", |v, cx| {
        v.panes
            .values()
            .all(|(p, _)| p.read(cx).settings.font_size == px(18.0))
    });

    // A value Ghostty would reject keeps the previous settings.
    std::fs::write(&h.home.ghostty, "font-size = huge\n").unwrap();
    h.wait_for("a message about the broken value", |v, _| {
        v.notifications
            .latest()
            .is_some_and(|s| s.contains("font-size = huge"))
    });
    assert_eq!(h.read(|v, _| v.settings.font_size), px(18.0));

    std::fs::write(
        &h.home.config,
        "sidebar-width = 200\ntab-title = \"path\"\n",
    )
    .unwrap();
    h.wait_for("config.toml applied", |v, _| v.config.sidebar_width == 200);
    assert_eq!(h.read(|v, _| v.ws.title_mode), chda_core::TitleMode::Path);
}

#[gpui::test]
fn config_ghostty_wins_over_the_legacy_name_live(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "cfgname", |home| {
        std::fs::write(&home.ghostty, "font-size = 13\n").unwrap();
    });
    h.wait_prompt();
    let named = h.home.ghostty.with_file_name("config.ghostty");
    std::fs::write(&named, "font-size = 16\n").unwrap();
    h.wait_for("config.ghostty applied", |v, cx| {
        v.settings.font_size == px(16.0)
            && v.panes
                .values()
                .all(|(p, _)| p.read(cx).settings.font_size == px(16.0))
    });
    h.cx.dispatch_action(crate::workspace_view::OpenGhosttyConfig);
    h.cx.run_until_parked();
    assert_eq!(h.system.0.borrow().opened_files, vec![named]);
}

#[gpui::test]
fn minimum_contrast_changes_apply_without_restarting_the_pane(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "contrast", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane());
    assert_eq!(h.read(|v, _| v.settings.minimum_contrast), 3.0);
    h.type_text("still editing");
    std::fs::write(&h.home.ghostty, "minimum-contrast = 1\n").unwrap();
    h.wait_for("contrast disabled in the existing pane", |v, cx| {
        v.settings.minimum_contrast == 1.0
            && v.panes
                .values()
                .all(|(p, _)| p.read(cx).settings.minimum_contrast == 1.0)
    });
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), pane);
    assert!(h.read(|v, cx| v.focused_text(cx).contains("still editing")));
    std::fs::write(&h.home.ghostty, "minimum-contrast = 4.5\n").unwrap();
    h.wait_for("contrast raised in the existing pane", |v, cx| {
        v.settings.minimum_contrast == 4.5
            && v.panes
                .values()
                .all(|(p, _)| p.read(cx).settings.minimum_contrast == 4.5)
    });
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), pane);
    assert!(h.read(|v, cx| v.focused_text(cx).contains("still editing")));
}
