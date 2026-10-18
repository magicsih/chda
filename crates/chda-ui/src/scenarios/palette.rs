//! Command palette: moving the selection, and picking a theme.

use chda_term::Rgb;
use gpui::TestAppContext;

use super::harness::Harness;
use crate::workspace_view::{SelectDarkTheme, SelectTheme};

const DEFAULT_BACKGROUND: Rgb = Rgb {
    r: 0x28,
    g: 0x2c,
    b: 0x34,
};

#[gpui::test]
fn a_narrow_palette_keeps_its_panel_and_keyboard_selection_inside_the_window(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "narrow-palette", |_| {});
    h.wait_prompt();
    h.cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(420.0)));
    h.keys("cmd-shift-p");
    let panel = h.cx.debug_bounds("command-palette").unwrap();
    assert!(panel.origin.x >= gpui::px(0.0));
    assert!(panel.right() <= gpui::px(640.0));
    assert!(panel.bottom() <= gpui::px(420.0), "{panel:?}");
    // Up wraps to the final action, which must be reachable in the same viewport.
    h.keys("up");
    let selected = h.cx.debug_bounds("palette-selected").unwrap();
    assert!(selected.origin.y >= panel.origin.y, "{selected:?}");
    assert!(selected.bottom() <= panel.bottom(), "{selected:?}");
    h.cx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(640.0)));
    h.keys("down up");
    h.cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(420.0)));
    h.cx.run_until_parked();
    let panel = h.cx.debug_bounds("command-palette").unwrap();
    let selected = h.cx.debug_bounds("palette-selected").unwrap();
    assert!(panel.bottom() <= gpui::px(420.0), "{panel:?}");
    assert!(selected.origin.y >= panel.origin.y, "{selected:?}");
    assert!(selected.bottom() <= panel.bottom(), "{selected:?}");
    h.keys("escape");
    h.run("printf 'palette-%s\\n' restored", "palette-restored");
}

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

/// `theme = light:X,dark:Y` follows the window's appearance, and the light
/// and dark pickers each change one half (#247).
#[gpui::test]
fn a_theme_pair_follows_the_appearance_and_each_picker_keeps_the_other_half(
    cx: &mut TestAppContext,
) {
    use chda_config::Appearance;
    let mut h = Harness::open(cx, "theme-pair", |home| {
        std::fs::write(
            &home.config,
            "theme = \"light:Catppuccin Latte,dark:Catppuccin Mocha\"\n",
        )
        .unwrap();
    });
    h.wait_prompt();
    let latte = Some(Rgb {
        r: 0xef,
        g: 0xf1,
        b: 0xf5,
    });
    let mocha = Some(Rgb {
        r: 0x1e,
        g: 0x1e,
        b: 0x2e,
    });
    // Test windows report a light appearance.
    assert_eq!(background(&h), latte);
    let appear = |h: &mut Harness, appearance| {
        h.cx.update(|window, cx| {
            h.view
                .update(cx, |v, cx| v.set_appearance(appearance, window, cx))
        });
        h.cx.run_until_parked();
    };
    appear(&mut h, Appearance::Dark);
    assert_eq!(background(&h), mocha);
    h.read(|v, cx| {
        assert!(
            v.panes
                .values()
                .all(|(p, _)| p.read(cx).settings.colors.background == mocha),
            "every pane switches without a restart"
        );
    });
    h.run("printf 'alive-%s\\n' after-switch", "alive-after-switch");

    // The dark picker previews its theme and keeps the light half.
    h.cx.dispatch_action(SelectDarkTheme);
    h.type_text("Dracula");
    let dracula = Some(Rgb {
        r: 0x28,
        g: 0x2a,
        b: 0x36,
    });
    // The palette filters and previews the highlighted entry asynchronously.
    h.wait_for("the Dracula preview", move |v, _| {
        v.settings.colors.background == dracula
    });
    h.keys("enter");
    assert_eq!(
        h.read(|v, _| v.config.theme.clone()).as_deref(),
        Some("light:Catppuccin Latte,dark:Dracula")
    );
    assert_eq!(background(&h), dracula);
    appear(&mut h, Appearance::Light);
    assert_eq!(background(&h), latte);
}
