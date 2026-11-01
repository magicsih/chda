//! Scenario tests for project sorting (Alphabetical vs Custom) and the sessions-first toggle.
use super::{harness::Harness, sidebar::draw_sidebar_frame};
use chda_config::{ChdaConfig, ProjectSortMode};
use gpui::{Modifiers, Pixels, Point, TestAppContext};
use std::path::PathBuf;

fn at(h: &mut Harness, selector: &str) -> Point<Pixels> {
    draw_sidebar_frame(h);
    h.cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center()
}

fn click(h: &mut Harness, selector: &str) {
    let p = at(h, selector);
    h.cx.simulate_click(p, Modifiers::none());
    h.cx.run_until_parked();
}

fn displayed_order(h: &mut Harness, items: &[(&str, usize)]) -> Vec<String> {
    draw_sidebar_frame(h);
    let mut positioned: Vec<(f32, String)> = items
        .iter()
        .map(|(name, index)| {
            let bounds =
                h.cx.debug_bounds(Box::leak(format!("repo-{index}").into_boxed_str()))
                    .unwrap_or_else(|| panic!("repo-{index} ({name}) is drawn"));
            (bounds.top().into(), (*name).to_string())
        })
        .collect();
    positioned.sort_by(|(y1, _), (y2, _)| y1.partial_cmp(y2).unwrap());
    positioned.into_iter().map(|(_, name)| name).collect()
}

#[gpui::test]
fn project_sorting_modes_switch_alphabetical_and_custom_and_restore_saved_order(
    cx: &mut TestAppContext,
) {
    let mut z_path = PathBuf::new();
    let mut a_path = PathBuf::new();
    let mut b_path = PathBuf::new();

    let mut h = Harness::open(cx, "project-sort-modes", |home| {
        // Create 3 repositories: Zebra, alpha, beta
        z_path = home.repo("zebra");
        a_path = home.repo("alpha");
        b_path = home.repo("beta");

        // Saved in custom order: zebra, alpha, beta
        let config = format!(
            "repos = [\"{}\", \"{}\", \"{}\"]\nproject-sort = \"custom\"\n",
            z_path.display(),
            a_path.display(),
            b_path.display(),
        );
        std::fs::write(&home.config, config).unwrap();
    });

    h.wait_prompt();
    h.wait_for("all repositories loaded", |v, cx| {
        let model = &v.sidebar.read(cx).model;
        model.repos.len() == 3 && model.repos.iter().all(|r| !r.worktrees.is_empty())
    });

    let items = [("zebra", 0), ("alpha", 1), ("beta", 2)];

    // 1. Initial order is Custom: zebra, alpha, beta
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["zebra", "alpha", "beta"]
    );
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).project_sort),
        ProjectSortMode::Custom
    );

    // 2. Click sort toggle: switches to Alphabetical (A–Z)
    click(&mut h, "project-sort-toggle");
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).project_sort),
        ProjectSortMode::Alphabetical
    );
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["alpha", "beta", "zebra"]
    );

    // Verify config was persisted
    let saved_cfg = ChdaConfig::load(&h.home.config).unwrap();
    assert_eq!(saved_cfg.project_sort, ProjectSortMode::Alphabetical);

    // 3. Click sort toggle again: switches back to Custom mode
    click(&mut h, "project-sort-toggle");
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).project_sort),
        ProjectSortMode::Custom
    );

    // The original custom order (zebra, alpha, beta) must be completely preserved!
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["zebra", "alpha", "beta"]
    );
    let saved_cfg_restored = ChdaConfig::load(&h.home.config).unwrap();
    assert_eq!(saved_cfg_restored.project_sort, ProjectSortMode::Custom);
}

#[gpui::test]
fn project_sessions_first_toggle_promotes_open_sessions_alphabetically(cx: &mut TestAppContext) {
    let mut z_path = PathBuf::new();
    let mut a_path = PathBuf::new();
    let mut b_path = PathBuf::new();

    let mut h = Harness::open(cx, "project-sessions-first", |home| {
        z_path = home.repo("zebra");
        a_path = home.repo("alpha");
        b_path = home.repo("beta");

        // Saved in custom order: zebra, alpha, beta
        let config = format!(
            "repos = [\"{}\", \"{}\", \"{}\"]\nproject-sort = \"custom\"\nproject-sessions-first = false\n",
            z_path.display(),
            a_path.display(),
            b_path.display(),
        );
        std::fs::write(&home.config, config).unwrap();
    });

    h.wait_prompt();
    h.wait_for("all repositories loaded", |v, cx| {
        let model = &v.sidebar.read(cx).model;
        model.repos.len() == 3 && model.repos.iter().all(|r| !r.worktrees.is_empty())
    });

    let items = [("zebra", 0), ("alpha", 1), ("beta", 2)];

    // Currently, only the initial terminal is open (in zebra because it's first or default).
    // Let's open a terminal in "beta" as well.
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::OpenTerminal(b_path.clone()),
                window,
                cx,
            );
        });
    });
    h.wait_for("beta terminal opened", |v, cx| {
        v.sidebar.read(cx).open_session_repos.contains(&b_path)
    });

    // Now beta and zebra have sessions; alpha does not.
    // 1. Initially sessions-first is OFF, base is Custom (zebra, alpha, beta)
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["zebra", "alpha", "beta"]
    );

    // 2. Toggle sessions-first ON (with only beta having a session)
    click(&mut h, "project-sessions-first-toggle");
    assert!(h.read(|v, cx| v.sidebar.read(cx).project_sessions_first));

    // Promoted: beta (with session)
    // Remaining without sessions in Custom order: zebra, alpha
    // Result: beta, zebra, alpha
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["beta", "zebra", "alpha"]
    );

    // Check config saved
    let saved_cfg = ChdaConfig::load(&h.home.config).unwrap();
    assert!(saved_cfg.project_sessions_first);

    // 3. Switch base sort to Alphabetical while sessions-first is ON
    click(&mut h, "project-sort-toggle");
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).project_sort),
        ProjectSortMode::Alphabetical
    );
    // Promoted: beta (with session)
    // Remaining without sessions in Alphabetical order: alpha, zebra
    // Result: beta, alpha, zebra
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["beta", "alpha", "zebra"]
    );

    // Now open a terminal in zebra as well -> both beta and zebra have sessions
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::OpenTerminal(z_path.clone()),
                window,
                cx,
            );
        });
    });
    h.wait_for("zebra terminal opened", |v, cx| {
        v.sidebar.read(cx).open_session_repos.contains(&z_path)
    });

    // Promoted: beta, zebra (always alphabetical: beta < zebra)
    // Remaining without sessions: alpha
    // Result: beta, zebra, alpha
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["beta", "zebra", "alpha"]
    );

    // 4. Toggle sessions-first OFF
    click(&mut h, "project-sessions-first-toggle");
    assert!(!h.read(|v, cx| v.sidebar.read(cx).project_sessions_first));

    // Base Alphabetical restored for all: alpha, beta, zebra
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["alpha", "beta", "zebra"]
    );
}

#[gpui::test]
fn project_sorting_controls_render_at_narrow_and_default_widths(cx: &mut TestAppContext) {
    let mut r_path = PathBuf::new();
    let mut h = Harness::open(cx, "project-sorting-widths", |home| {
        r_path = home.repo("app");
        std::fs::write(
            &home.config,
            format!("repos = [\"{}\"]\n", r_path.display()),
        )
        .unwrap();
    });

    h.wait_prompt();
    h.wait_for("repository loaded", |v, cx| {
        !v.sidebar.read(cx).model.repos.is_empty()
    });

    // Default width (280px): controls exist and clickable
    assert!(h.cx.debug_bounds("project-sort-toggle").is_some());
    assert!(h.cx.debug_bounds("project-sessions-first-toggle").is_some());

    // Narrow width (180px)
    h.cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            v.config.sidebar_width = 180;
            cx.notify();
        });
    });
    draw_sidebar_frame(&mut h);

    assert!(h.cx.debug_bounds("project-sort-toggle").is_some());
    assert!(h.cx.debug_bounds("project-sessions-first-toggle").is_some());

    // Click works at 180px
    click(&mut h, "project-sort-toggle");
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).project_sort),
        ProjectSortMode::Alphabetical
    );
}

#[gpui::test]
fn project_sorting_interaction_safety_defers_reorder_during_menu(cx: &mut TestAppContext) {
    let mut z_path = PathBuf::new();
    let mut a_path = PathBuf::new();

    let mut h = Harness::open(cx, "project-sorting-safety", |home| {
        z_path = home.repo("zebra");
        a_path = home.repo("alpha");
        let config = format!(
            "repos = [\"{}\", \"{}\"]\nproject-sort = \"alphabetical\"\nproject-sessions-first = true\n",
            z_path.display(),
            a_path.display(),
        );
        std::fs::write(&home.config, config).unwrap();
    });

    h.wait_prompt();
    h.wait_for("repositories loaded", |v, cx| {
        let model = &v.sidebar.read(cx).model;
        model.repos.len() == 2 && model.repos.iter().all(|r| !r.worktrees.is_empty())
    });

    let items = [("zebra", 0), ("alpha", 1)];

    // Initially neither zebra nor alpha has a session.
    // Base is Alphabetical, so alpha comes before zebra.
    assert_eq!(displayed_order(&mut h, &items), vec!["alpha", "zebra"]);

    // Open a context menu on alpha
    let alpha_pos = at(&mut h, "repo-1");
    h.cx.simulate_mouse_down(alpha_pos, gpui::MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(alpha_pos, gpui::MouseButton::Right, Modifiers::none());
    h.cx.run_until_parked();

    assert!(h.read(|v, _| v.context_menu.is_some()), "menu is open");

    // While the menu is open, simulate a pane navigating to zebra
    let existing_pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            if let Some(info) = v.ws.pane_mut(existing_pane) {
                info.cwd = Some(z_path.clone());
            }
            v.sync_panes(cx);
        });
    });

    // The order must NOT change yet because menu interaction freezes the order!
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["alpha", "zebra"],
        "order deferred during open menu"
    );

    // Dismiss menu by pressing Escape
    h.keys("escape");
    h.cx.run_until_parked();
    assert!(h.read(|v, _| v.context_menu.is_none()));

    // Now re-rendering without menu reflects the promoted zebra session: zebra comes first!
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["zebra", "alpha"],
        "order updated after menu dismissed"
    );
}

#[gpui::test]
fn project_sections_preserve_sections_with_sessions_first(cx: &mut TestAppContext) {
    let mut z_path = PathBuf::new();
    let mut a_path = PathBuf::new();
    let mut b_path = PathBuf::new();

    let mut h = Harness::open(cx, "project-sections-sessions-first", |home| {
        z_path = home.repo("zebra");
        a_path = home.repo("alpha");
        b_path = home.repo("beta");

        let config = format!(
            "repos = [\"{}\", \"{}\", \"{}\"]\n\
             project-sort = \"alphabetical\"\n\
             project-sessions-first = true\n\
             [[project-sections]]\n\
             id = \"sec-1\"\n\
             name = \"First Section\"\n\
             [repo-sections]\n\
             \"{}\" = \"sec-1\"\n\
             \"{}\" = \"sec-1\"\n",
            z_path.display(),
            a_path.display(),
            b_path.display(),
            z_path.display(),
            a_path.display(),
        );
        std::fs::write(&home.config, config).unwrap();
    });

    h.wait_prompt();
    h.wait_for("repositories loaded", |v, cx| {
        let model = &v.sidebar.read(cx).model;
        model.repos.len() == 3 && model.repos.iter().all(|r| !r.worktrees.is_empty())
    });

    let items = [("zebra", 0), ("alpha", 1), ("beta", 2)];

    // zebra and alpha are in "First Section"; beta is in Uncategorized.
    // Open session in zebra.
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::OpenTerminal(z_path.clone()),
                window,
                cx,
            );
        });
    });
    h.wait_for("zebra session registered", |v, cx| {
        v.sidebar.read(cx).open_session_repos.contains(&z_path)
    });

    // In "First Section": zebra (session) comes before alpha.
    // "beta" remains in Uncategorized below "First Section".
    assert_eq!(
        displayed_order(&mut h, &items),
        vec!["zebra", "alpha", "beta"]
    );
}
