//! Resizing and collapsing the sidebar.

use gpui::{Modifiers, MouseButton, TestAppContext, point, px};

use super::harness::Harness;

fn width(h: &Harness) -> u32 {
    h.read(|v, _| v.config.sidebar_width)
}

fn saved(h: &Harness) -> String {
    std::fs::read_to_string(&h.home.config).unwrap_or_default()
}

fn drag_grip_to(h: &mut Harness, x: f32) {
    h.cx.run_until_parked();
    let grip =
        h.cx.debug_bounds("sidebar-grip")
            .expect("the grip is drawn");
    let start = grip.center();
    h.cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    for step in 1..=4 {
        let x = f32::from(start.x) + (x - f32::from(start.x)) * step as f32 / 4.0;
        h.cx.simulate_mouse_move(point(px(x), start.y), MouseButton::Left, Modifiers::none());
    }
    h.cx.simulate_mouse_up(point(px(x), start.y), MouseButton::Left, Modifiers::none());
    h.cx.run_until_parked();
}

#[gpui::test]
fn dragging_the_border_resizes_the_sidebar_within_bounds(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sidebar-drag", |_| {});
    h.wait_prompt();
    assert_eq!(width(&h), 280);

    drag_grip_to(&mut h, 400.0);
    assert_eq!(width(&h), 400);
    assert!(saved(&h).contains("sidebar-width = 400"), "{}", saved(&h));
    // The panes follow the border.
    let grip = h.cx.debug_bounds("sidebar-grip").unwrap();
    assert!((f32::from(grip.right()) - 400.0).abs() <= 2.0, "{grip:?}");

    // Past either end it stops at the bounds.
    drag_grip_to(&mut h, 20.0);
    assert_eq!(width(&h), 180);
    drag_grip_to(&mut h, 5000.0);
    let window =
        h.cx.update(|window, _| f32::from(window.viewport_size().width));
    assert_eq!(width(&h) as f32, 600f32.min(window - 320.0).max(180.0));

    // Moving the pointer without a drag changes nothing.
    let before = width(&h);
    h.cx.simulate_mouse_move(point(px(250.), px(300.)), None, Modifiers::none());
    assert_eq!(width(&h), before);

    // Double-click resets the default.
    let grip = h.cx.debug_bounds("sidebar-grip").unwrap().center();
    h.cx.simulate_event(gpui::MouseDownEvent {
        position: grip,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    h.cx.simulate_event(gpui::MouseUpEvent {
        position: grip,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
    });
    h.cx.run_until_parked();
    assert_eq!(width(&h), 280);
}

#[gpui::test]
fn the_title_bar_button_collapses_and_expands_the_sidebar(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sidebar-toggle", |_| {});
    h.wait_prompt();
    drag_grip_to(&mut h, 350.0);
    let click = |h: &mut Harness| {
        h.cx.run_until_parked();
        let at =
            h.cx.debug_bounds("sidebar-toggle")
                .expect("the button is drawn")
                .center();
        h.cx.simulate_click(at, Modifiers::none());
        h.cx.run_until_parked();
    };

    click(&mut h);
    assert!(h.read(|v, _| !v.sidebar_visible));
    assert!(h.cx.debug_bounds("sidebar-grip").is_none());
    assert!(
        saved(&h).contains("sidebar-visible = false"),
        "{}",
        saved(&h)
    );

    click(&mut h);
    assert!(h.read(|v, _| v.sidebar_visible));
    assert_eq!(width(&h), 350, "the width survives collapsing");

    // cmd-b and the button stay in step.
    h.keys("cmd-b");
    h.cx.run_until_parked();
    assert!(h.read(|v, _| !v.sidebar_visible));
}

#[gpui::test]
fn active_tabs_stay_in_place_and_labels_toggle_persistently(cx: &mut TestAppContext) {
    use super::harness::{git, wait_until};
    use crate::sidebar_view::SidebarEvent;
    use crate::terminal_view::TerminalEvent;
    use chda_config::ActiveLabel;

    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "active", |home| {
        repo = home.repo("app");
        git(
            &repo,
            &[
                "config",
                "branch.main.description",
                "Fix login\nMore detail",
            ],
        );
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    let r = repo.clone();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(r), window, cx);
        })
    });
    h.wait_prompt();
    h.wait_for("the ACTIVE branch alias", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .active_tabs
            .iter()
            .any(|t| t.title == "Fix login")
    });
    h.keys("cmd-d");
    h.wait_prompt();
    h.keys("cmd-t");
    h.wait_prompt();
    let order = h.read(|v, _| v.ws.tabs().iter().map(|t| t.id).collect::<Vec<_>>());
    let panes = h.read(|v, _| {
        v.ws.tabs()
            .iter()
            .flat_map(|t| t.panes())
            .collect::<Vec<_>>()
    });
    // Feed alternating output through the real terminal subscription, including
    // a pane that is not focused in its split tab.
    let base = crate::terminal_view::now_ms() + 10_000;
    for (index, pane) in panes.iter().copied().enumerate().rev() {
        let at = base + index as u64;
        h.cx.update(|_, cx| {
            let terminal = h.view.read(cx).panes[&pane].0.clone();
            terminal.update(cx, |_, cx| cx.emit(TerminalEvent::Activity(at)));
        });
        h.read(|v, cx| {
            let rows = &v.sidebar.read(cx).model.active_tabs;
            assert_eq!(rows.iter().map(|r| r.tab).collect::<Vec<_>>(), order);
            let tab =
                v.ws.tabs()
                    .iter()
                    .find(|t| t.panes().contains(&pane))
                    .unwrap();
            let expected = tab
                .panes()
                .iter()
                .map(|p| v.ws.pane(*p).unwrap().last_activity)
                .max()
                .unwrap();
            assert_eq!(
                rows.iter().find(|r| r.tab == tab.id).unwrap().last_activity,
                expected
            );
        });
    }
    // Use the actual header button, rather than invoking the handler directly.
    h.cx.run_until_parked();
    let at = h.cx.debug_bounds("active-label-toggle").unwrap().center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
    assert_eq!(h.read(|v, _| v.config.active_label), ActiveLabel::Branch);
    assert!(saved(&h).contains("active-label = \"branch\""));
    h.read(|v, cx| {
        let rows = &v.sidebar.read(cx).model.active_tabs;
        assert_eq!(rows.iter().map(|r| r.tab).collect::<Vec<_>>(), order);
        assert!(
            rows.iter()
                .filter(|r| r.branch.is_some())
                .all(|r| r.title == "main")
        );
    });
    let (mut cx2, view2) = h.reopen();
    wait_until(
        &mut cx2,
        &view2,
        "restored ACTIVE branch labels",
        |v, cx| {
            v.config.active_label == ActiveLabel::Branch
                && v.sidebar
                    .read(cx)
                    .model
                    .active_tabs
                    .iter()
                    .any(|r| r.title == "main")
        },
    );
    // External config edits apply live and leave ordinary tab titles alone.
    let mut config = chda_config::ChdaConfig::load(&h.home.config).unwrap();
    config.active_label = ActiveLabel::Alias;
    config.tab_title = chda_config::TabTitle::Path;
    config.save(&h.home.config).unwrap();
    h.wait_for("live alias labels", |v, cx| {
        v.config.active_label == ActiveLabel::Alias
            && v.ws.title_mode == chda_core::TitleMode::Path
            && v.sidebar
                .read(cx)
                .model
                .active_tabs
                .iter()
                .any(|r| r.title == "Fix login")
    });
    // Removing the note outside chda falls back to the actual branch name.
    git(&repo, &["config", "--unset", "branch.main.description"]);
    h.wait_for("alias fallback after the note is removed", |v, cx| {
        let rows = &v.sidebar.read(cx).model.active_tabs;
        rows.iter().any(|r| r.branch.as_deref() == Some("main"))
            && rows
                .iter()
                .filter(|r| r.branch.is_some())
                .all(|r| r.title == "main")
    });
}

#[gpui::test]
fn activity_ages_refresh_without_output_or_session_writes(cx: &mut TestAppContext) {
    use futures::{FutureExt, StreamExt};
    use std::time::Duration;

    let mut h = Harness::open(cx, "active-age", |_| {});
    h.wait_prompt();
    h.wait_for("the initial session index", |v, cx| {
        v.sidebar.read(cx).sessions_loaded
    });
    h.cx.run_until_parked();
    let sidebar = h.read(|v, _| v.sidebar.clone());
    let before = h.read(|v, cx| v.sidebar.read(cx).model.active_tabs.clone());
    let session = std::fs::read(h.home.data.join("session.json")).unwrap();
    let mut notices = h.cx.cx.notifications(&sidebar);
    h.cx.executor().advance_clock(Duration::from_secs(1));
    h.cx.run_until_parked();
    assert!(
        notices.next().now_or_never().is_some(),
        "idle ages repaint each second"
    );
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).model.active_tabs.clone()),
        before,
        "a clock tick does not create activity or reorder rows"
    );
    assert_eq!(
        std::fs::read(h.home.data.join("session.json")).unwrap(),
        session,
        "a clock tick does not write a session"
    );
    h.keys("cmd-b");
    while notices.next().now_or_never().is_some() {}
    h.cx.executor().advance_clock(Duration::from_secs(1));
    h.cx.run_until_parked();
    assert!(
        notices.next().now_or_never().is_none(),
        "hidden sidebar is not repainted"
    );
}
