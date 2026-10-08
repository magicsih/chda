//! Resizing and collapsing the sidebar.

use gpui::{Modifiers, MouseButton, TestAppContext, point, px};

use super::harness::Harness;

fn click_row(h: &mut Harness, selector: &str) {
    h.cx.run_until_parked();
    let at =
        h.cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
            .expect(selector)
            .center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
}

#[gpui::test]
fn active_labels_and_clicks_follow_split_focus_across_branches(cx: &mut TestAppContext) {
    use super::harness::git;
    use crate::sidebar_view::SidebarEvent;
    use chda_core::agents::HookKind;

    let mut main = std::path::PathBuf::new();
    let mut feature = main.clone();
    let mut h = Harness::open(cx, "active-focus", |home| {
        main = home.repo("app");
        feature = home.home.join("feature");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                feature.to_str().unwrap(),
            ],
        );
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\"]\nactive-label = \"branch\"\n",
                main.display()
            ),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(main.clone()), window, cx);
        })
    });
    h.wait_prompt();
    let (tab, first) = h.read(|v, _| (v.ws.active_tab().unwrap().id, v.ws.focused_pane().unwrap()));
    h.hook(Some(first.raw()), &main, HookKind::PromptSubmitted);
    h.wait_for("the first pane working", |v, _| {
        v.ws.pane(first).unwrap().agent.is_some()
    });
    h.keys("cmd-d");
    h.wait_prompt();
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.run(&format!("cd '{}'", feature.display()), "test%");
    h.wait_for("the feature directory", |v, _| {
        v.ws.pane(second).unwrap().cwd.as_ref() == Some(&feature)
    });
    h.hook(Some(second.raw()), &feature, HookKind::SessionStart);
    h.wait_for("the second pane idle", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .idle_agents
            .iter()
            .any(|r| r.pane == second)
    });
    h.keys("cmd-alt-left");
    h.read(|v, cx| {
        assert_eq!(v.ws.focused_pane(), Some(first));
        let sidebar = v.sidebar.read(cx);
        let row = sidebar
            .model
            .active_tabs
            .iter()
            .find(|r| r.tab == tab)
            .unwrap();
        assert_eq!(
            row.branch.as_deref(),
            Some("main"),
            "ACTIVE follows focus even without new output or agent status"
        );
        assert_eq!(row.title, "main");
        assert_eq!(sidebar.selected.as_ref(), Some(&main));
    });
    h.keys("cmd-t");
    h.wait_prompt();
    click_row(&mut h, &format!("active-{tab:?}"));
    h.read(|v, cx| {
        assert_eq!(v.ws.focused_pane(), Some(first));
        assert_eq!(v.sidebar.read(cx).selected.as_ref(), Some(&main));
    });
    click_row(&mut h, &format!("idle-{}", second.raw()));
    h.read(|v, cx| {
        assert_eq!(v.ws.focused_pane(), Some(second));
        let sidebar = v.sidebar.read(cx);
        let row = sidebar
            .model
            .active_tabs
            .iter()
            .find(|r| r.tab == tab)
            .unwrap();
        assert_eq!(row.branch.as_deref(), Some("feature"));
        assert_eq!(sidebar.selected.as_ref(), Some(&feature));
    });
}

#[gpui::test]
fn navigation_reveals_only_clipped_rows_and_keeps_repository_context(cx: &mut TestAppContext) {
    use super::harness::git;
    use crate::sidebar_view::SidebarEvent;

    let mut repo = std::path::PathBuf::new();
    let mut paths = Vec::new();
    let mut h = Harness::open(cx, "reveal-top", |home| {
        repo = home.repo("app");
        for index in 0..24 {
            let branch = format!("task-{index:02}");
            let path = home.home.join(&branch);
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &branch,
                    path.to_str().unwrap(),
                ],
            );
            paths.push(path);
        }
        let other = home.repo("other-app");
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\", \"{}\"]\n",
                repo.display(),
                other.display()
            ),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.cx.simulate_resize(gpui::size(px(960.0), px(480.0)));
    h.wait_for("all worktrees", |v, cx| {
        v.sidebar.read(cx).model.repos[0].worktrees.len() == 25
    });
    h.view.update(&mut h.cx, |v, cx| {
        v.sidebar.update(cx, |s, cx| {
            s.model.set_sort(chda_core::SortOrder::Name);
            cx.notify();
        })
    });
    // New tabs must not change ACTIVE's height while measuring reveal deltas.
    click_row(&mut h, "active-section-toggle");
    for (path, position) in [
        (&paths[6], 0),
        (&paths[23], 0),
        (&paths[0], 1),
        (&paths[6], 2),
        (&paths[6], 3),
    ] {
        h.cx.run_until_parked();
        if position != 0 {
            let scroll = h.read(|v, cx| v.sidebar.read(cx).scroll.clone());
            if position == 1 {
                scroll.set_offset(point(px(0.0), -scroll.max_offset().y));
            } else {
                let row =
                    h.cx.debug_bounds(Box::leak(format!("wt:{}", path.display()).into_boxed_str()))
                        .unwrap();
                let viewport = scroll.bounds();
                let delta = if position == 2 {
                    viewport.top() + px(16.0) - row.top()
                } else {
                    viewport.bottom() + px(8.0) - row.bottom()
                };
                scroll.set_offset(point(
                    px(0.0),
                    (scroll.offset().y + delta).clamp(-scroll.max_offset().y, px(0.0)),
                ));
            }
            h.cx.run_until_parked();
            h.cx.update(|window, cx| window.simulate_next_frame(cx));
            h.cx.run_until_parked();
        }
        let selector = Box::leak(format!("wt:{}", path.display()).into_boxed_str());
        let before = h.cx.debug_bounds(selector).unwrap();
        let (offset, viewport, max) = h.read(|v, cx| {
            let s = v.sidebar.read(cx);
            (
                s.scroll.offset().y,
                s.scroll.bounds(),
                s.scroll.max_offset().y,
            )
        });
        let delta = if before.top() < viewport.top() + px(32.0) {
            viewport.top() + px(32.0) - before.top()
        } else if before.bottom() > viewport.bottom() {
            viewport.bottom() - before.bottom()
        } else {
            px(0.0)
        };
        let expected = (offset + delta).clamp(-max, px(0.0));
        h.cx.update(|window, cx| {
            h.view.update(cx, |v, cx| {
                v.on_sidebar_event(SidebarEvent::OpenWorktree(path.clone()), window, cx)
            })
        });
        h.wait_prompt();
        h.cx.run_until_parked();
        // The test platform has no display frame loop. Deliver the frame
        // scheduled after the target's layout, as the native platform does.
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
        h.cx.run_until_parked();
        let after = h.cx.debug_bounds(selector).unwrap();
        let viewport = h.read(|v, cx| v.sidebar.read(cx).scroll.bounds());
        h.read(|v, cx| {
            let sidebar = v.sidebar.read(cx);
            let offset = sidebar.scroll.offset().y;
            let max = sidebar.scroll.max_offset().y;
            assert!(max > px(0.0), "fixture has a scrollable list");
            assert_eq!(sidebar.selected.as_ref(), Some(path));
            assert!((offset - expected).abs() < px(2.0),
                "visible rows retain their offset; clipped rows move only to their edge: actual {:?}, expected {expected:?}", sidebar.scroll.offset());
        });
        assert!(after.top() >= viewport.top() && after.bottom() <= viewport.bottom());
        if h.cx.debug_bounds("repo-0").unwrap().top() < viewport.top() {
            assert!(h.cx.debug_bounds("sticky-repo-context").is_some());
            assert!(after.top() >= viewport.top() + px(32.0));
        }
        // A background refresh must not move the list back to this target.
        let scroll = h.read(|v, cx| v.sidebar.read(cx).scroll.clone());
        scroll.set_offset(point(px(0.0), px(0.0)));
        h.run("echo still-here", "still-here");
        assert_eq!(scroll.offset().y, px(0.0));
    }
}

#[gpui::test]
fn agent_sections_collapse_independently_and_persist(cx: &mut TestAppContext) {
    use super::harness::wait_until;
    use crate::terminal_view::TerminalEvent;
    use chda_core::agents::HookKind;

    let mut h = Harness::open(cx, "section-collapse", |_| {});
    h.wait_prompt();
    let (tab, pane) = h.read(|v, _| (v.ws.active_tab().unwrap().id, v.ws.focused_pane().unwrap()));
    h.hook(Some(pane.raw()), &h.home.home, HookKind::SessionStart);
    h.wait_for("the live idle row", |v, cx| {
        v.sidebar.read(cx).model.idle_agents.len() == 1
    });
    let active_selector = Box::leak(format!("active-{tab:?}").into_boxed_str());
    let idle_selector = Box::leak(format!("idle-{}", pane.raw()).into_boxed_str());
    click_row(&mut h, "active-section-toggle");
    assert!(h.cx.debug_bounds(active_selector).is_none());
    assert!(h.cx.debug_bounds(idle_selector).is_some());
    let label = h.read(|v, _| v.config.active_label);
    click_row(&mut h, "active-label-toggle");
    assert_ne!(h.read(|v, _| v.config.active_label), label);
    assert!(
        h.read(|v, _| v.config.active_collapsed),
        "label toggle never expands its parent"
    );
    click_row(&mut h, "idle-section-toggle");
    assert!(h.cx.debug_bounds(idle_selector).is_none());
    assert!(h.cx.debug_bounds("active-section-toggle").is_some());
    assert!(h.cx.debug_bounds("idle-section-toggle").is_some());
    let config = chda_config::ChdaConfig::load(&h.home.config).unwrap();
    assert!(config.active_collapsed && config.idle_agents_collapsed);
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(pane));
    let terminal = h.focused_terminal();
    h.cx.update(|_, cx| {
        terminal.update(cx, |_, cx| {
            cx.emit(TerminalEvent::Activity(crate::terminal_view::now_ms()))
        })
    });
    h.cx.run_until_parked();
    assert!(h.cx.debug_bounds(active_selector).is_none());
    assert!(h.cx.debug_bounds(idle_selector).is_none());
    click_row(&mut h, "active-section-toggle");
    assert!(h.cx.debug_bounds(active_selector).is_some());
    assert!(h.cx.debug_bounds(idle_selector).is_none());
    click_row(&mut h, "active-section-toggle");
    // Restore does not restore agent life, but it retains both preferences.
    let (mut cx2, view2) = h.reopen();
    wait_until(&mut cx2, &view2, "saved section preferences", |v, cx| {
        let sidebar = v.sidebar.read(cx);
        sidebar.active_collapsed
            && sidebar.idle_agents_collapsed
            && !sidebar.model.active_tabs.is_empty()
    });
    cx2.run_until_parked();
    assert!(cx2.debug_bounds("active-section-toggle").is_some());
    let restored_tab = view2.read_with(&cx2, |v, _| v.ws.active_tab().unwrap().id);
    assert!(
        cx2.debug_bounds(Box::leak(
            format!("active-{restored_tab:?}").into_boxed_str()
        ))
        .is_none()
    );
    // Live configuration changes also apply independently.
    let mut config = chda_config::ChdaConfig::load(&h.home.config).unwrap();
    config.active_collapsed = false;
    config.save(&h.home.config).unwrap();
    wait_until(&mut cx2, &view2, "live section preference", |v, cx| {
        let sidebar = v.sidebar.read(cx);
        !sidebar.active_collapsed && sidebar.idle_agents_collapsed
    });
}

#[gpui::test]
fn project_is_a_peer_section_and_collapses_without_hiding_active(cx: &mut TestAppContext) {
    use super::harness::wait_until;
    let mut h = Harness::open(cx, "project-section", |home| {
        let repo = home.repo("app");
        let folder = home.home.join("plain-folder");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\", \"{}\"]\n",
                repo.display(),
                folder.display()
            ),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for("both registered folders", |v, cx| {
        v.sidebar.read(cx).model.repos.len() == 2
    });
    h.cx.run_until_parked();
    let active = h.cx.debug_bounds("active-section-toggle").unwrap();
    let idle =
        h.cx.debug_bounds("idle-section-toggle")
            .expect("empty IDLE still has a header");
    let project =
        h.cx.debug_bounds("project-section-toggle")
            .expect("PROJECT is the third peer section");
    assert!(active.top() < idle.top() && idle.top() < project.top());
    assert!(h.cx.debug_bounds("repo-0").unwrap().top() >= project.bottom());
    assert!(h.cx.debug_bounds("repo-1").is_some());
    assert!(
        h.cx.debug_bounds("expand-all").is_none() && h.cx.debug_bounds("collapse-all").is_none()
    );
    click_row(&mut h, "project-section-toggle");
    assert!(h.cx.debug_bounds("repo-0").is_none());
    assert!(h.cx.debug_bounds("active-section-toggle").is_some());
    assert!(h.cx.debug_bounds("add-repo").is_some());
    assert!(saved(&h).contains("project-collapsed = true"));
    let (mut cx2, view2) = h.reopen();
    wait_until(
        &mut cx2,
        &view2,
        "collapsed PROJECT restoration",
        |v, cx| v.sidebar.read(cx).model.repos.len() == 2,
    );
    cx2.run_until_parked();
    assert!(cx2.debug_bounds("repo-0").is_none());
    assert!(cx2.debug_bounds("project-section-toggle").is_some());
    click_row(&mut h, "project-section-toggle");
    assert!(h.cx.debug_bounds("repo-0").is_some());
}

#[gpui::test]
fn plain_terminal_rows_follow_agents_and_close_the_exact_background_tab(cx: &mut TestAppContext) {
    use chda_core::agents::HookKind;
    let mut h = Harness::open(cx, "plain-active", |_| {});
    h.wait_prompt();
    let shell = h.read(|v, _| v.ws.active_tab().unwrap().id);
    h.keys("cmd-t");
    h.wait_prompt();
    let (agent, pane) =
        h.read(|v, _| (v.ws.active_tab().unwrap().id, v.ws.focused_pane().unwrap()));
    h.hook(Some(pane.raw()), &h.home.home, HookKind::PromptSubmitted);
    h.wait_for("live agent", |v, _| v.ws.pane(pane).unwrap().agent_live);
    h.keys("cmd-d");
    h.wait_prompt();
    h.cx.run_until_parked();
    let bounds = |h: &mut Harness, id| {
        h.cx.debug_bounds(Box::leak(format!("active-{id:?}").into_boxed_str()))
            .unwrap()
    };
    let agent_top = bounds(&mut h, agent).top();
    let shell_top = bounds(&mut h, shell).top();
    assert!(
        agent_top < shell_top,
        "agent split comes before ordinary shells"
    );
    assert!(
        h.cx.debug_bounds(Box::leak(
            format!("active-close-{agent:?}").into_boxed_str()
        ))
        .is_none()
    );
    click_row(&mut h, &format!("active-close-{shell:?}"));
    assert_eq!(
        h.read(|v, _| v.ws.active_tab().unwrap().id),
        agent,
        "closing background shell does not activate it"
    );
    assert!(h.read(|v, _| !v.ws.tabs().iter().any(|t| t.id == shell)));
    assert!(h.read(|v, _| v.confirm.is_none()));
}

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
