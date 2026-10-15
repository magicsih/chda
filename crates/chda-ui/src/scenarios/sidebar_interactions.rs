//! Stable PROJECT rows, one-click activation and visible split numbers.
use super::harness::{Harness, git};
use chda_core::agents::HookKind;
use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, px, size};
use std::path::{Path, PathBuf};

fn settle(h: &mut Harness) {
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|w, cx| w.simulate_next_frame(cx));
    }
}
fn click(h: &mut Harness, selector: &str, button: MouseButton, count: usize) {
    settle(h);
    let at =
        h.cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
            .expect(selector)
            .center();
    h.cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: Modifiers::none(),
        button,
        click_count: count,
        first_mouse: false,
    });
    h.cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers: Modifiers::none(),
        button,
        click_count: count,
    });
    settle(h);
}
fn row(path: &Path) -> String {
    format!("wt:{}", path.display())
}
fn fixture(
    cx: &mut TestAppContext,
    name: &str,
    agent: bool,
) -> (Harness, PathBuf, PathBuf, PathBuf) {
    let (mut repo, mut a, mut z) = (PathBuf::new(), PathBuf::new(), PathBuf::new());
    let mut h = Harness::open(cx, name, |home| {
        repo = home.repo("project");
        a = home.home.join("feature-a");
        z = home.home.join("feature-z");
        for (branch, path) in [("feature/a", &a), ("feature/z", &z)] {
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    branch,
                    path.to_str().unwrap(),
                ],
            );
        }
        home.fake_claude();
        let mut config = chda_config::ChdaConfig::default();
        config.repos.push(repo.clone());
        if agent {
            config.default_action = chda_config::DefaultAction::Claude;
        }
        config.starred.push(chda_config::StarredBranch {
            repo: repo.clone(),
            branch: "feature/a".into(),
        });
        config.save(&home.config).unwrap();
    });
    h.wait_prompt();
    h.wait_for("worktrees", |v, cx| {
        v.sidebar.read(cx).model.repos[0].worktrees.len() == 3
    });
    settle(&mut h);
    (h, repo, a, z)
}

#[gpui::test]
fn project_click_opens_once_and_reuses_the_pane(cx: &mut TestAppContext) {
    let (mut h, _, a, _) = fixture(cx, "project-click", false);
    let pane = h.read(|v, _| v.ws.focused_pane());
    let tabs = h.read(|v, _| v.ws.tabs().len());
    click(&mut h, &row(&a), MouseButton::Left, 1);
    h.wait_prompt();
    let opened = h.read(|v, _| v.ws.focused_pane());
    h.read(|v, cx| {
        assert_eq!(v.ws.tabs().len(), tabs + 1, "a click opens the branch");
        assert_ne!(opened, pane, "a click moves terminal focus to it");
        assert_eq!(v.ws.pane(opened.unwrap()).unwrap().cwd.as_ref(), Some(&a));
        assert_eq!(v.sidebar.read(cx).selected.as_ref(), Some(&a));
    });
    h.hook(None, &a, HookKind::PromptSubmitted);
    h.wait_for("background selection sync", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .worktree_for_path(&a)
            .unwrap()
            .1
            .last_activity
            > 0
    });
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).selected.clone()),
        Some(a.clone()),
        "background sync discarded selection"
    );
    // The second and third clicks of a double or triple click open nothing.
    click(&mut h, &row(&a), MouseButton::Left, 2);
    click(&mut h, &row(&a), MouseButton::Left, 3);
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        tabs + 1,
        "a double or triple click duplicated the open"
    );
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), opened);
    let original_tab = h.read(|v, _| v.ws.tabs()[0].id);
    click(
        &mut h,
        &format!("session-tab-{original_tab:?}"),
        MouseButton::Left,
        1,
    );
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), pane);
    click(&mut h, &row(&a), MouseButton::Left, 1);
    assert_eq!(
        h.read(|v, _| v.ws.focused_pane()),
        opened,
        "existing pane is reused"
    );
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs + 1);
}

#[gpui::test]
fn starred_click_opens_and_right_click_opens_only_the_captured_menu(cx: &mut TestAppContext) {
    let (mut h, _, a, z) = fixture(cx, "starred-menu", false);
    let focused = h.read(|v, _| v.ws.focused_pane());
    let tabs = h.read(|v, _| v.ws.tabs().len());
    for target in ["starred-0".to_owned(), row(&a)] {
        click(&mut h, &target, MouseButton::Right, 1);
        assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs);
        assert!(h.read(|v, _| v.context_menu.is_some()));
        h.hook(None, &z, HookKind::PromptSubmitted);
        h.wait_for("background report", |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&z)
                .unwrap()
                .1
                .last_activity
                > 0
        });
        h.read(|v, _| {
            let menu = v.context_menu.as_ref().unwrap();
            assert!(menu.items.iter().any(|(_, action)| matches!(action, crate::workspace_view::MenuAction::OpenTerminal(p) if p == &a)), "menu target changed");
        });
        h.keys("escape");
        assert!(h.read(|v, _| v.context_menu.is_none()));
    }
    click(&mut h, "starred-0", MouseButton::Right, 1);
    let selector = h.read(|v, _| {
        let index = v
            .context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|(label, _)| label == "Open terminal")
            .unwrap();
        format!("menu-item-{index}")
    });
    click(&mut h, &selector, MouseButton::Left, 1);
    h.wait_prompt();
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs + 1);
    let opened = h.read(|v, _| v.ws.focused_pane());
    assert_eq!(
        h.read(|v, _| v.ws.pane(opened.unwrap()).unwrap().cwd.clone()),
        Some(a.clone())
    );
    let original_tab = h.read(|v, _| v.ws.tabs()[0].id);
    click(
        &mut h,
        &format!("session-tab-{original_tab:?}"),
        MouseButton::Left,
        1,
    );
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), focused);
    click(&mut h, "starred-0", MouseButton::Left, 1);
    click(&mut h, "starred-0", MouseButton::Left, 2);
    assert_eq!(
        h.read(|v, _| v.ws.focused_pane()),
        opened,
        "a click reuses the terminal"
    );
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).selected.clone()),
        Some(a.clone())
    );
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs + 1);
    click(&mut h, "unstar-0", MouseButton::Left, 1);
    assert!(h.read(|v, cx| v.sidebar.read(cx).model.starred.is_empty()));
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs + 1);
}

#[gpui::test]
fn background_hooks_keep_project_rows_under_the_pointer(cx: &mut TestAppContext) {
    let (mut h, _, a, z) = fixture(cx, "stable-rows", false);
    let before =
        h.cx.debug_bounds(Box::leak(row(&a).into_boxed_str()))
            .unwrap();
    h.cx.simulate_mouse_move(before.center(), Some(MouseButton::Left), Modifiers::none());
    h.hook(None, &z, HookKind::PromptSubmitted);
    h.wait_for("background activity", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .worktree_for_path(&z)
            .unwrap()
            .1
            .last_activity
            > 0
    });
    settle(&mut h);
    assert_eq!(
        h.cx.debug_bounds(Box::leak(row(&a).into_boxed_str()))
            .unwrap(),
        before,
        "background activity moved the pointed row"
    );
    click(&mut h, &row(&a), MouseButton::Right, 1);
    h.read(|v, _| assert!(v.context_menu.as_ref().unwrap().items.iter().any(|(_, action)| matches!(action, crate::workspace_view::MenuAction::OpenTerminal(p) if p == &a))));
}

#[gpui::test]
fn agent_default_starts_from_a_click_and_context_terminal_still_opens_a_shell(
    cx: &mut TestAppContext,
) {
    let (mut h, _, a, _) = fixture(cx, "click-agent", true);
    let tabs = h.read(|v, _| v.ws.tabs().len());
    click(&mut h, &row(&a), MouseButton::Left, 1);
    assert!(
        h.cx.debug_bounds("launch-start").is_some(),
        "a click starts the default agent's launch"
    );
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs);
    h.keys("escape");
    click(&mut h, &row(&a), MouseButton::Right, 1);
    let index = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|(label, _)| label == "Open terminal")
            .unwrap()
    });
    click(&mut h, &format!("menu-item-{index}"), MouseButton::Left, 1);
    h.wait_prompt();
    assert!(h.cx.debug_bounds("launch-start").is_none());
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs + 1);
    assert!(h.read(|v, _| {
        v.ws.pane(v.ws.focused_pane().unwrap())
            .unwrap()
            .agent_launch
            .is_none()
    }));
}

#[gpui::test]
fn split_numbers_fit_the_row_at_default_and_minimum_width(cx: &mut TestAppContext) {
    let (mut h, repo, _, _) = fixture(cx, "split-number", false);
    std::fs::write(&h.home.ghostty, "font-size = 18\n").unwrap();
    h.wait_for("18pt terminal font", |v, _| {
        f32::from(v.settings.font_size) == 18.0
    });
    h.cx.simulate_resize(size(px(640.0), px(600.0)));
    settle(&mut h);
    click(&mut h, &row(&repo), MouseButton::Left, 1);
    h.wait_prompt();
    let first = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.run(
        "stty -echo; printf '\nnumber-ready-one\n'; exec cat",
        "\nnumber-ready-one\n",
    );
    h.hook_session(Some(first.raw()), &repo, HookKind::SessionStart, "first");
    h.wait_for("first agent", |v, _| v.ws.pane(first).unwrap().agent_live);
    h.keys("cmd-d");
    h.wait_prompt();
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.run(
        "stty -echo; printf '\nnumber-ready-two\n'; exec cat",
        "\nnumber-ready-two\n",
    );
    h.hook_session(Some(second.raw()), &repo, HookKind::SessionStart, "second");
    h.wait_for("second agent", |v, _| v.ws.pane(second).unwrap().agent_live);
    assert_ne!(first, second, "the fixture has a real split");
    h.wait_for("two numbered rows", |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
        [first, second].iter().all(|pane| {
            rows.iter()
                .any(|r| r.key == chda_core::SessionKey::Pane(*pane) && r.shares_tab)
        })
    });
    for width in [280, 180] {
        h.view.update(&mut h.cx, |v, cx| {
            v.config.sidebar_width = width;
            cx.notify();
        });
        h.cx.simulate_resize(size(px(640.0), px(600.0)));
        settle(&mut h);
        for pane in [first, second] {
            let id = format!("pane-{}", pane.raw());
            let row =
                h.cx.debug_bounds(Box::leak(format!("session-{id}").into_boxed_str()))
                    .unwrap();
            let number =
                h.cx.debug_bounds(Box::leak(format!("session-number-{id}").into_boxed_str()))
                    .expect("visible independent pane number");
            let close =
                h.cx.debug_bounds(Box::leak(
                    format!("session-close-pane-{}", pane.raw()).into_boxed_str(),
                ))
                .expect("split pane close control");
            assert!(
                close.right() <= row.right() && close.left() >= row.left(),
                "close control outside row at {width}px"
            );
            assert!(number.size.width >= px(12.0));
            assert!(
                row.contains(&number.origin) && number.right() <= row.right(),
                "{width}: {number:?} outside {row:?}"
            );
        }
    }
}
