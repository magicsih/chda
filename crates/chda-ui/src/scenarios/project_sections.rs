//! Input-driven project grouping, including plain folders and old configs.
use super::{harness::Harness, sidebar::draw_sidebar_frame};
use chda_config::ChdaConfig;
use gpui::{
    ExternalPaths, FileDropEvent, Modifiers, MouseButton, Pixels, Point, TestAppContext, point, px,
};

fn saved(h: &Harness) -> ChdaConfig {
    ChdaConfig::load(&h.home.config).unwrap()
}
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
fn menu(h: &mut Harness, selector: &str, label: &str) {
    let p = at(h, selector);
    h.cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(p, MouseButton::Right, Modifiers::none());
    h.cx.run_until_parked();
    menu_item(h, label);
}
fn menu_item(h: &mut Harness, label: &str) {
    let i = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .expect("menu")
            .items
            .iter()
            .position(|(s, _)| s == label)
            .unwrap_or_else(|| panic!("menu item {label}"))
    });
    click(h, &format!("menu-item-{i}"));
}
fn group(id: &str) -> String {
    format!("project-group-{id}")
}
fn create(h: &mut Harness, name: &str) -> String {
    click(h, "add-section");
    h.type_text(name);
    h.keys("enter");
    h.cx.run_until_parked();
    saved(h).project_sections.last().unwrap().id.clone()
}
fn drag(h: &mut Harness, from: &str, to: &str) {
    let (a, b) = (at(h, from), at(h, to));
    h.cx.simulate_mouse_down(a, MouseButton::Left, Modifiers::none());
    for step in 1..=6 {
        let t = step as f32 / 6.;
        h.cx.simulate_mouse_move(
            point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t + px(1.)),
            MouseButton::Left,
            Modifiers::none(),
        );
    }
    h.cx.simulate_mouse_move(b, MouseButton::Left, Modifiers::none());
    h.cx.simulate_mouse_up(b, MouseButton::Left, Modifiers::none());
    h.cx.run_until_parked();
}
fn drop_folder(h: &mut Harness, folder: &std::path::Path, selector: &str) {
    let p = at(h, selector);
    h.cx.simulate_event(FileDropEvent::Entered {
        position: p,
        paths: ExternalPaths([folder.to_path_buf()].into_iter().collect()),
    });
    h.cx.simulate_event(FileDropEvent::Pending { position: p });
    h.cx.simulate_event(FileDropEvent::Submit { position: p });
    h.cx.run_until_parked();
}

#[gpui::test]
fn sections_create_rename_move_collapse_delete_and_restore(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sections-edit", |home| {
        let c = ChdaConfig {
            repos: [home.repo("a"), home.repo("b")].into(),
            ..Default::default()
        };
        c.save(&home.config).unwrap();
    });
    h.wait_prompt();
    h.wait_for("repos", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .all(|r| !r.worktrees.is_empty())
    });
    assert!(
        h.cx.debug_bounds("project-group-uncategorized").is_none(),
        "legacy flat list"
    );
    let work = create(&mut h, "Work");
    let empty = create(&mut h, "Empty");
    let repos = saved(&h).repos;
    // Real repository drag into an empty section, no row click or terminal change.
    let focus = h.read(|v, _| v.ws.focused_pane());
    drag(&mut h, "repo-0", &group(&work));
    assert_eq!(saved(&h).repo_section(&repos[0]), Some(work.as_str()));
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), focus);
    // A project context menu moves the other one to the same section.
    menu(&mut h, "repo-0", "Move to section…");
    menu_item(&mut h, "Work");
    assert_eq!(saved(&h).repo_section(&repos[1]), Some(work.as_str()));
    // Rename never changes identity or membership.
    menu(&mut h, &group(&work), "Rename section…");
    h.keys("cmd-a");
    h.type_text("Renamed");
    h.keys("enter");
    assert_eq!(saved(&h).project_sections[0].id, work);
    assert_eq!(saved(&h).project_sections[0].name, "Renamed");
    click(&mut h, &group(&work));
    assert!(saved(&h).project_sections[0].collapsed);
    draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("repo-0").is_none());
    // Bulk repository disclosure preserves section state.
    click(&mut h, "repos-bulk-toggle");
    assert!(saved(&h).project_sections[0].collapsed);
    // Sections themselves reorder, including an empty one.
    drag(&mut h, &group(&empty), &group(&work));
    assert_eq!(saved(&h).project_sections[0].id, empty);
    let (mut reopened, view) = h.reopen();
    reopened.run_until_parked();
    assert_eq!(
        reopened.update(|_, cx| view.read(cx).sidebar.read(cx).project_sections.clone()),
        saved(&h).project_sections
    );
    menu(
        &mut h,
        &group(&work),
        "Delete section (move projects to Uncategorized)",
    );
    let c = saved(&h);
    assert!(repos.iter().all(|r| c.repo_section(r).is_none()));
    assert!(c.repo_sections.is_empty());
    assert_eq!(c.repos.len(), 2);
    menu(
        &mut h,
        &group(&empty),
        "Delete section (move projects to Uncategorized)",
    );
    draw_sidebar_frame(&mut h);
    assert!(h.cx.debug_bounds("project-group-uncategorized").is_none());
}

#[gpui::test]
fn section_folder_drops_preserve_the_plain_folder_choice_and_default_to_uncategorized(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "section-drops", |_| {});
    h.wait_prompt();
    let work = create(&mut h, "Work");
    let repo = h.home.repo("dropped");
    drop_folder(&mut h, &repo, &group(&work));
    h.wait_for("dropped repository", |v, cx| {
        v.sidebar.read(cx).model.repos.len() == 1
    });
    assert_eq!(saved(&h).repo_section(&repo), Some(work.as_str()));
    let folder = h.home.home.join("notes");
    std::fs::create_dir(&folder).unwrap();
    drop_folder(&mut h, &folder, &group(&work));
    h.cx.simulate_prompt_answer("Cancel");
    h.cx.run_until_parked();
    assert!(!saved(&h).repos.contains(&folder));
    drop_folder(&mut h, &folder, &group(&work));
    h.cx.simulate_prompt_answer("Add as folder");
    h.wait_for("plain folder", |v, cx| {
        v.sidebar.read(cx).model.is_folder(&folder)
    });
    assert_eq!(saved(&h).repo_section(&folder), Some(work.as_str()));
    assert!(!folder.join(".git").exists());
    let other = h.home.repo("default");
    drop_folder(&mut h, &other, "project-section-toggle");
    h.wait_for("default project", |v, cx| {
        v.sidebar.read(cx).model.repos.len() == 3
    });
    assert_eq!(saved(&h).repo_section(&other), None);
}

#[gpui::test]
fn navigation_reveals_its_group_and_background_refresh_leaves_groups_and_scroll_alone(
    cx: &mut TestAppContext,
) {
    use crate::sidebar_view::SidebarEvent;
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "section-reveal", |home| {
        repo = home.repo("project");
        let mut c = ChdaConfig {
            repos: vec![repo.clone()],
            ..Default::default()
        };
        let id = c.add_project_section("Work").unwrap();
        c.assign_repo_section(&repo, Some(&id));
        c.project_sections[0].collapsed = true;
        c.collapsed_repos.push(repo.clone());
        c.save(&home.config).unwrap();
    });
    h.wait_prompt();
    h.wait_for("worktree listed", |v, cx| {
        !v.sidebar.read(cx).model.repos[0].worktrees.is_empty()
    });
    let work = saved(&h).project_sections[0].id.clone();
    assert!(saved(&h).project_sections[0].collapsed);
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(repo.clone()), window, cx)
        })
    });
    h.wait_prompt();
    h.cx.run_until_parked();
    assert!(!saved(&h).project_sections[0].collapsed);
    assert!(!h.read(|v, cx| v.sidebar.read(cx).model.repos[0].collapsed));
    click(&mut h, &group(&work));
    let before = h.read(|v, cx| {
        (
            v.sidebar.read(cx).project_scroll.offset(),
            v.sidebar.read(cx).sessions_scroll.offset(),
        )
    });
    h.view
        .update(&mut h.cx, |v, cx| v.refresh_repo(repo.clone(), cx));
    h.cx.run_until_parked();
    assert!(saved(&h).project_sections[0].collapsed);
    assert_eq!(
        h.read(|v, cx| (
            v.sidebar.read(cx).project_scroll.offset(),
            v.sidebar.read(cx).sessions_scroll.offset()
        )),
        before
    );
    click(&mut h, "project-section-toggle");
    h.keys("cmd-t");
    h.wait_prompt();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(repo.clone()), window, cx)
        })
    });
    assert!(h.read(|v, cx| v.sidebar.read(cx).project_collapsed));
}

#[gpui::test]
fn adding_an_existing_project_preserves_membership_and_editing_is_visible_while_scrolled(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "section-existing", |home| {
        let mut c = ChdaConfig {
            repos: (0..20).map(|i| home.repo(&format!("p{i:02}"))).collect(),
            ..Default::default()
        };
        let id = c.add_project_section("Work").unwrap();
        c.assign_repo_section(&c.repos[0].clone(), Some(&id));
        c.save(&home.config).unwrap();
    });
    h.wait_prompt();
    h.wait_for("many projects", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .all(|r| !r.worktrees.is_empty())
    });
    let c = saved(&h);
    let repo = c.repos[0].clone();
    let id = c.project_sections[0].id.clone();
    drop_folder(&mut h, &repo, "project-section-toggle");
    assert_eq!(saved(&h).repo_section(&repo), Some(id.as_str()));
    // A deliberate Uncategorized target does move an existing project.
    drop_folder(&mut h, &repo, &group("uncategorized"));
    assert_eq!(saved(&h).repo_section(&repo), None);
    let scroll = h.read(|v, cx| v.sidebar.read(cx).project_scroll.clone());
    scroll.set_offset(point(px(0.), -scroll.max_offset().y));
    h.view
        .update(&mut h.cx, |v, cx| v.sidebar.update(cx, |_, cx| cx.notify()));
    let added = h.home.repo("from-fixed-header");
    drop_folder(&mut h, &added, "project-section-toggle");
    assert_eq!(
        saved(&h).repo_section(&added),
        None,
        "clipped list rows cannot classify a header drop"
    );
    // Scroll down again after the explicit add revealed its new row.
    scroll.set_offset(point(px(0.), -scroll.max_offset().y));
    click(&mut h, "add-section");
    draw_sidebar_frame(&mut h);
    let editor = h.cx.debug_bounds("section-name-editor").expect("editor");
    let project = h.cx.debug_bounds("project-list").unwrap();
    assert!(
        editor.bottom() <= project.top(),
        "editor stays above the scrolling list"
    );
    h.keys("escape");
    assert!(saved(&h).project_sections.len() == 1);
}

#[gpui::test]
fn moving_projects_between_headers_keeps_order_after_refresh_and_restart(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "section-order", |home| {
        let mut c = ChdaConfig {
            repos: [home.repo("a"), home.repo("b"), home.repo("c")].into(),
            ..Default::default()
        };
        let work = c.add_project_section("Work").unwrap();
        let other = c.add_project_section("Other").unwrap();
        for i in [0, 1] {
            c.assign_repo_section(&c.repos[i].clone(), Some(&work));
        }
        c.assign_repo_section(&c.repos[2].clone(), Some(&other));
        c.save(&home.config).unwrap();
    });
    h.wait_prompt();
    h.wait_for("projects", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .all(|r| !r.worktrees.is_empty())
    });
    let initial = saved(&h);
    let [a, b, c] = initial.repos.clone().try_into().unwrap();
    drag(&mut h, "repo-0", "repo-1");
    assert_eq!(saved(&h).repos, [b.clone(), a.clone(), c.clone()]);
    drag(&mut h, "repo-2", "repo-0");
    let current = saved(&h);
    assert_eq!(
        current.repo_section(&c),
        Some(initial.project_sections[0].id.as_str())
    );
    assert_eq!(current.repos, [c, b, a]);
    h.view.update(&mut h.cx, |v, cx| {
        for repo in &current.repos {
            v.refresh_repo(repo.clone(), cx);
        }
    });
    h.cx.run_until_parked();
    assert_eq!(saved(&h).repos, current.repos);
    let (mut cx2, v) = h.reopen();
    cx2.run_until_parked();
    assert_eq!(
        cx2.update(|_, cx| v
            .read(cx)
            .sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .map(|r| r.path.clone())
            .collect::<Vec<_>>()),
        current.repos
    );
}
