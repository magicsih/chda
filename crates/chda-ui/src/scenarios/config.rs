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
        v.status_line
            .as_deref()
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
