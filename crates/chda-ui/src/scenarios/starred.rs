//! STARRED branches: context-menu starring, navigation, one-click unstarring
//! and persistence (#129).

use super::harness::{Harness, git};
use crate::sidebar_view::SidebarEvent;
use crate::workspace_view::MenuAction;
use gpui::{Modifiers, TestAppContext};
use std::path::{Path, PathBuf};

fn menu_item(h: &mut Harness, worktree: &Path, prefix: &str) -> Option<(String, MenuAction)> {
    let event = SidebarEvent::WorktreeMenu(worktree.to_path_buf(), gpui::Point::default());
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.on_sidebar_event(event, window, cx))
    });
    let item = h.read(|v, _| {
        v.context_menu
            .as_ref()?
            .items
            .iter()
            .find(|(label, _)| label.starts_with(prefix))
            .cloned()
    });
    h.cx.update(|_, cx| h.view.update(cx, |v, _| v.context_menu = None));
    item
}

fn star(h: &mut Harness, worktree: &Path) {
    let (_, action) = menu_item(h, worktree, "Add to Starred").expect("Add to Starred");
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.cx.run_until_parked();
}

fn starred(h: &Harness) -> Vec<(PathBuf, String)> {
    h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .starred
            .iter()
            .map(|s| (s.repo.clone(), s.branch.clone()))
            .collect()
    })
}

fn click(h: &mut Harness, selector: String) {
    h.cx.run_until_parked();
    let at =
        h.cx.debug_bounds(Box::leak(selector.clone().into_boxed_str()))
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
}

fn focused_cwd(h: &Harness) -> Option<PathBuf> {
    h.read(|v, _| v.ws.pane(v.ws.focused_pane()?)?.cwd.clone())
}

#[gpui::test]
fn starred_branches_navigate_unstar_in_one_click_and_persist(cx: &mut TestAppContext) {
    let (mut a, mut b, mut x) = Default::default();
    let mut h = Harness::open(cx, "starred", |home| {
        a = home.repo("a");
        b = home.repo("b");
        x = a.parent().unwrap().join("a-x");
        git(
            &a,
            &["worktree", "add", "-q", "-b", "feat/x", x.to_str().unwrap()],
        );
        std::fs::write(
            &home.config,
            format!("repos = [\"{}\", \"{}\"]\n", a.display(), b.display()),
        )
        .unwrap();
    });
    let (a, b, x): (PathBuf, PathBuf, PathBuf) = (a, b, x);
    h.wait_prompt();
    h.wait_for("both repositories", |v, cx| {
        let s = v.sidebar.read(cx);
        s.model.repos.iter().all(|r| !r.worktrees.is_empty())
            && s.model.branch_worktree(&a, "feat/x").is_some()
    });
    assert!(starred(&h).is_empty());
    assert!(h.cx.debug_bounds("starred-0").is_none(), "no empty section");

    // The same branch name in two repositories stays two entries; starring
    // again is not offered, so there are no duplicates.
    star(&mut h, &a);
    assert!(menu_item(&mut h, &a, "Add to Starred").is_none());
    assert!(menu_item(&mut h, &a, "Remove from Starred").is_some());
    star(&mut h, &b);
    star(&mut h, &x);
    assert_eq!(
        starred(&h),
        vec![
            (a.clone(), "main".to_owned()),
            (b.clone(), "main".to_owned()),
            (a.clone(), "feat/x".to_owned()),
        ]
    );
    // STARRED shows without any Sessions row for b or the feature worktree.
    let tabs = h.read(|v, _| v.ws.tabs().len());

    // Clicking a label navigates with the usual worktree navigation.
    click(&mut h, "starred-1".into());
    assert_eq!(focused_cwd(&h).as_deref(), Some(b.as_path()));
    let after_b = h.read(|v, _| v.ws.tabs().len());
    assert_eq!(after_b, tabs + 1);
    click(&mut h, "starred-2".into());
    assert_eq!(focused_cwd(&h).as_deref(), Some(x.as_path()));
    click(&mut h, "starred-1".into());
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        after_b + 1,
        "the existing tab of b is reused"
    );
    let focused = h.read(|v, _| v.ws.focused_pane());

    // The star only unstars: no navigation, no tab change.
    click(&mut h, "unstar-2".into());
    assert_eq!(starred(&h).len(), 2);
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), focused);
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), after_b + 1);
    assert!(x.is_dir(), "unstarring keeps the worktree");

    // A starred branch whose worktree disappears stays listed and does not
    // navigate anywhere else.
    star(&mut h, &x);
    git(&a, &["worktree", "remove", "--force", x.to_str().unwrap()]);
    h.cx.update(|_, cx| h.view.update(cx, |v, cx| v.refresh_repo(a.clone(), cx)));
    h.wait_for("the removed worktree", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .branch_worktree(&a, "feat/x")
            .is_none()
    });
    click(&mut h, "starred-2".into());
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), focused);
    assert!(
        h.read(|v, _| v.notifications.latest().map(str::to_owned))
            .is_some_and(|s| s.contains("No worktree has feat/x checked out"))
    );

    // Saved in the config and back after a restart.
    let text = std::fs::read_to_string(&h.home.config).unwrap();
    assert_eq!(text.matches("[[starred]]").count(), 3, "{text}");
    let (mut cx2, view) = h.reopen();
    cx2.run_until_parked();
    let restored: Vec<(PathBuf, String)> = cx2.update(|_, cx| {
        view.read(cx)
            .sidebar
            .read(cx)
            .model
            .starred
            .iter()
            .map(|s| (s.repo.clone(), s.branch.clone()))
            .collect()
    });
    assert_eq!(restored, starred(&h));
}
