//! Retired Starred branches: existing configs load safely without Starred UI
//! or context menu actions, and project navigation remains intact (#312).

use super::harness::{Harness, git};
use crate::sidebar_view::SidebarEvent;
use gpui::TestAppContext;
use std::path::{Path, PathBuf};

fn menu_items(h: &mut Harness, worktree: &Path) -> Vec<String> {
    let event = SidebarEvent::WorktreeMenu(worktree.to_path_buf(), gpui::Point::default());
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.on_sidebar_event(event, window, cx))
    });
    let items = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .map(|m| m.items.iter().map(|(label, _)| label.clone()).collect())
            .unwrap_or_default()
    });
    h.cx.update(|_, cx| h.view.update(cx, |v, _| v.context_menu = None));
    items
}

fn settle(h: &mut Harness) {
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|w, cx| w.simulate_next_frame(cx));
    }
}

fn focused_cwd(h: &Harness) -> Option<PathBuf> {
    h.read(|v, _| v.ws.pane(v.ws.focused_pane()?)?.cwd.clone())
}

#[gpui::test]
fn existing_starred_configuration_loads_without_starred_ui_and_preserves_projects(
    cx: &mut TestAppContext,
) {
    let (mut a, mut b, mut x) = Default::default();
    let mut h = Harness::open(cx, "starred-retired", |home| {
        a = home.repo("a");
        b = home.repo("b");
        x = a.parent().unwrap().join("a-x");
        git(
            &a,
            &["worktree", "add", "-q", "-b", "feat/x", x.to_str().unwrap()],
        );
        std::fs::write(
            &home.config,
            format!(
                r#"repos = ["{}", "{}"]
sidebar-width = 280

[[starred]]
repo = "{}"
branch = "main"

[[starred]]
repo = "{}"
branch = "feat/x"
"#,
                a.display(),
                b.display(),
                a.display(),
                a.display()
            ),
        )
        .unwrap();
    });
    let (a, b, x): (PathBuf, PathBuf, PathBuf) = (a, b, x);
    h.wait_prompt();
    h.wait_for("both repositories", |v, cx| {
        let s = v.sidebar.read(cx);
        s.model.repos.len() == 2 && s.model.repos.iter().all(|r| !r.worktrees.is_empty())
    });
    settle(&mut h);

    // Starred UI rows and controls are completely absent.
    assert!(h.cx.debug_bounds("starred-0").is_none());
    assert!(h.cx.debug_bounds("starred-1").is_none());
    assert!(h.cx.debug_bounds("unstar-0").is_none());

    // Context menu does not offer Add to Starred or Remove from Starred.
    let items_a = menu_items(&mut h, &a);
    assert!(!items_a.iter().any(|label| label.contains("Starred")));
    let items_x = menu_items(&mut h, &x);
    assert!(!items_x.iter().any(|label| label.contains("Starred")));

    // Normal project navigation works as expected.
    let tabs = h.read(|v, _| v.ws.tabs().len());
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(b.clone()), window, cx);
        });
    });
    h.wait_for("worktree b to open", |v, _| {
        v.ws.tabs().len() == tabs + 1
            && v.ws
                .focused_pane()
                .and_then(|p| v.ws.pane(p))
                .and_then(|p| p.cwd.as_deref())
                == Some(b.as_path())
    });
    assert_eq!(focused_cwd(&h).as_deref(), Some(b.as_path()));

    // Verify layout at narrow sidebar width (180px).
    h.view.update(&mut h.cx, |v, cx| {
        v.config.sidebar_width = 180;
        cx.notify();
    });
    settle(&mut h);
    assert!(h.cx.debug_bounds("starred-0").is_none());
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(x.clone()), window, cx);
        });
    });
    h.wait_for("worktree x to open", |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .and_then(|p| p.cwd.as_deref())
            == Some(x.as_path())
    });
    assert_eq!(focused_cwd(&h).as_deref(), Some(x.as_path()));

    // Reopen window: existing starred config loads safely and drops obsolete entries on save.
    h.read(|v, _| {
        v.config.save(&h.home.config).unwrap();
    });
    let text = std::fs::read_to_string(&h.home.config).unwrap();
    assert!(
        !text.contains("[[starred]]") && !text.contains("starred ="),
        "retired starred entries are omitted upon save: {text}"
    );
    let (mut cx2, view) = h.reopen();
    cx2.run_until_parked();
    assert_eq!(
        cx2.update(|_, cx| view.read(cx).sidebar.read(cx).model.repos.len()),
        2
    );
}
