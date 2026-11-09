//! Scenario tests for project archiving and unarchiving.
use super::{harness::Harness, sidebar::draw_sidebar_frame};
use chda_config::ChdaConfig;
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

#[gpui::test]
fn project_archiving_and_unarchiving_flow(cx: &mut TestAppContext) {
    let mut repo_a = PathBuf::new();
    let mut repo_b = PathBuf::new();

    let mut h = Harness::open(cx, "project-archiving", |home| {
        repo_a = home.repo("alpha");
        repo_b = home.repo("beta");

        let config = format!(
            "repos = [\"{}\", \"{}\"]\n",
            repo_a.display(),
            repo_b.display(),
        );
        std::fs::write(&home.config, config).unwrap();
    });

    h.wait_prompt();
    h.wait_for("both repositories loaded", |v, cx| {
        let model = &v.sidebar.read(cx).model;
        model.repos.len() == 2 && model.repos.iter().all(|r| !r.worktrees.is_empty())
    });

    // Verify initial state: neither is archived
    assert!(!h.read(|v, cx| v.sidebar.read(cx).archived_repos.contains(&repo_b)));

    // 1. Trigger Archive for repo_b via context menu action
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::ArchiveRepo(repo_b.clone()),
                window,
                cx,
            );
        });
    });
    h.cx.run_until_parked();

    // Verify repo_b is archived in config and sidebar state
    let saved_cfg = ChdaConfig::load(&h.home.config).unwrap();
    assert!(saved_cfg.is_archived(&repo_b));
    assert!(h.read(|v, cx| v.sidebar.read(cx).archived_repos.contains(&repo_b)));

    // 2. Expand the Archived section by clicking the header
    click(&mut h, "archived-section-toggle");
    assert!(!h.read(|v, cx| v.sidebar.read(cx).archived_collapsed));

    // 3. Click the Unarchive button for repo_b
    let unarchive_selector = format!("unarchive:{}", repo_b.display());
    click(&mut h, &unarchive_selector);

    // Verify repo_b is unarchived
    let saved_cfg = ChdaConfig::load(&h.home.config).unwrap();
    assert!(!saved_cfg.is_archived(&repo_b));
    assert!(!h.read(|v, cx| v.sidebar.read(cx).archived_repos.contains(&repo_b)));
}
