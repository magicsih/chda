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
fn split_agent_rows_keep_their_own_labels_and_follow_pane_focus(cx: &mut TestAppContext) {
    use super::harness::git;
    use crate::sidebar_view::SidebarEvent;
    use chda_core::SessionKey;
    use chda_core::agents::HookKind;

    let mut main = std::path::PathBuf::new();
    let mut feature = main.clone();
    let mut h = Harness::open(cx, "split-labels", |home| {
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
    let first = h.read(|v, _| v.ws.focused_pane().unwrap());
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
            .sessions
            .iter()
            .any(|r| r.key == SessionKey::Pane(second) && r.live_idle())
    });
    let labels = |h: &Harness| {
        h.read(|v, cx| {
            v.sidebar
                .read(cx)
                .model
                .sessions
                .iter()
                .filter(|r| matches!(r.key, SessionKey::Pane(_)))
                .map(|r| (r.key, r.title.clone()))
                .collect::<Vec<_>>()
        })
    };
    let expected = vec![
        (SessionKey::Pane(first), "main".to_owned()),
        (SessionKey::Pane(second), "feature".to_owned()),
    ];
    assert_eq!(
        labels(&h),
        expected,
        "each agent row names its own pane's branch"
    );
    // Moving the split focus without output moves only the highlights.
    h.keys("cmd-alt-left");
    h.read(|v, cx| {
        assert_eq!(v.ws.focused_pane(), Some(first));
        let sidebar = v.sidebar.read(cx);
        assert_eq!(sidebar.focused_session(), Some(SessionKey::Pane(first)));
        assert_eq!(sidebar.selected.as_ref(), Some(&main));
    });
    assert_eq!(labels(&h), expected);
    h.keys("cmd-t");
    h.wait_prompt();
    click_row(&mut h, &format!("session-pane-{}", first.raw()));
    h.read(|v, cx| {
        assert_eq!(v.ws.focused_pane(), Some(first));
        assert_eq!(v.sidebar.read(cx).selected.as_ref(), Some(&main));
    });
    click_row(&mut h, &format!("session-pane-{}", second.raw()));
    h.read(|v, cx| {
        assert_eq!(v.ws.focused_pane(), Some(second));
        let sidebar = v.sidebar.read(cx);
        assert_eq!(sidebar.focused_session(), Some(SessionKey::Pane(second)));
        assert_eq!(sidebar.selected.as_ref(), Some(&feature));
    });
    assert_eq!(labels(&h), expected);
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
        v.sidebar.update(cx, |_, cx| {
            cx.notify();
        })
    });
    // New tabs must not change Sessions' height while measuring reveal deltas.
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    click_row(&mut h, "sessions-section-toggle");
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    assert!(
        h.read(|v, cx| v.config.sessions_collapsed && v.sidebar.read(cx).sessions_collapsed),
        "Sessions must be collapsed before measuring reveal geometry"
    );
    for (path, position) in [
        (&paths[6], 0),
        (&paths[23], 0),
        (&paths[0], 1),
        (&paths[6], 2),
        (&paths[6], 3),
    ] {
        h.cx.run_until_parked();
        if position != 0 {
            let scroll = h.read(|v, cx| v.sidebar.read(cx).project_scroll.clone());
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
        // Settle the layout before deriving the expected scroll delta too.
        // The test platform does not deliver native display frames itself.
        for _ in 0..3 {
            h.cx.run_until_parked();
            h.cx.update(|window, cx| window.simulate_next_frame(cx));
        }
        let selector = Box::leak(format!("wt:{}", path.display()).into_boxed_str());
        let before = h.cx.debug_bounds(selector).unwrap();
        let (offset, viewport, max) = h.read(|v, cx| {
            let s = v.sidebar.read(cx);
            (
                s.project_scroll.offset().y,
                s.project_scroll.bounds(),
                s.project_scroll.max_offset().y,
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
        let viewport = h.read(|v, cx| v.sidebar.read(cx).project_scroll.bounds());
        h.read(|v, cx| {
            let sidebar = v.sidebar.read(cx);
            let offset = sidebar.project_scroll.offset().y;
            let max = sidebar.project_scroll.max_offset().y;
            assert!(max > px(0.0), "fixture has a scrollable list");
            assert_eq!(sidebar.selected.as_ref(), Some(path));
            assert!((offset - expected).abs() < px(2.0),
                "visible rows retain their offset; clipped rows move only to their edge: actual {:?}, expected {expected:?}", sidebar.project_scroll.offset());
        });
        assert!(after.top() >= viewport.top() && after.bottom() <= viewport.bottom());
        if h.cx.debug_bounds("repo-0").unwrap().top() < viewport.top() {
            assert!(h.cx.debug_bounds("sticky-repo-context").is_some());
            assert!(after.top() >= viewport.top() + px(32.0));
        }
        // A background refresh must not move the list back to this target.
        let scroll = h.read(|v, cx| v.sidebar.read(cx).project_scroll.clone());
        scroll.set_offset(point(px(0.0), px(0.0)));
        h.run("echo still-here", "still-here");
        assert_eq!(scroll.offset().y, px(0.0));
    }
}

/// The repository name pinned at PROJECT's top acts on its repository like
/// the header it stands in for, never on the worktree row scrolled under it,
/// and the wheel still scrolls PROJECT over it (#149).
#[gpui::test]
fn sticky_repository_header_acts_on_its_repository(cx: &mut TestAppContext) {
    use super::harness::git;

    let mut repo = std::path::PathBuf::new();
    let mut paths = Vec::new();
    let mut h = Harness::open(cx, "sticky-header", |home| {
        repo = home.repo("app");
        let other = home.repo("other-app");
        for (owner, prefix) in [(&repo, "task"), (&other, "other")] {
            for index in 0..16 {
                let branch = format!("{prefix}-{index:02}");
                let path = home.home.join(&branch);
                git(
                    owner,
                    &[
                        "worktree",
                        "add",
                        "-q",
                        "-b",
                        &branch,
                        path.to_str().unwrap(),
                    ],
                );
                // A branch note gives the whole worktree row a tooltip.
                git(
                    owner,
                    &[
                        "config",
                        &format!("branch.{branch}.description"),
                        &format!("Note {branch}"),
                    ],
                );
                if owner == &repo {
                    paths.push(path);
                }
            }
        }
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
    h.wait_for("all worktrees and notes", |v, cx| {
        let repos = &v.sidebar.read(cx).model.repos;
        repos.len() == 2
            && repos
                .iter()
                .all(|r| r.worktrees.len() == 17 && r.worktrees.iter().any(|w| w.note.is_some()))
    });
    h.view.update(&mut h.cx, |v, cx| {
        v.sidebar.update(cx, |_, cx| {
            cx.notify();
        })
    });
    click_row(&mut h, "sessions-section-toggle");
    let settle = |h: &mut Harness| {
        for _ in 0..3 {
            h.cx.run_until_parked();
            h.cx.update(|window, cx| window.simulate_next_frame(cx));
        }
        h.cx.run_until_parked();
    };
    let scroll = h.read(|v, cx| v.sidebar.read(cx).project_scroll.clone());
    let covered = paths[6].clone();
    let selector: &'static str = Box::leak(format!("wt:{}", covered.display()).into_boxed_str());
    // Scroll PROJECT so the sticky name covers the middle of `covered`.
    let scroll_under_sticky = |h: &mut Harness| {
        settle(h);
        let row = h.cx.debug_bounds(selector).unwrap();
        let target = scroll.bounds().top() + px(16.0) - row.size.height / 2.0;
        scroll.set_offset(point(
            px(0.0),
            (scroll.offset().y + target - row.top()).clamp(-scroll.max_offset().y, px(0.0)),
        ));
        h.view
            .update(&mut h.cx, |v, cx| v.sidebar.update(cx, |_, cx| cx.notify()));
        settle(h);
        let sticky =
            h.cx.debug_bounds("sticky-repo-context")
                .expect("the repository name is pinned over its rows");
        let row = h.cx.debug_bounds(selector).unwrap();
        assert!(
            row.top() < sticky.center().y && row.bottom() > sticky.center().y,
            "{row:?} lies under {sticky:?}"
        );
        sticky.center()
    };
    let at = scroll_under_sticky(&mut h);
    let tabs = h.read(|v, _| v.ws.tabs().len());
    let selected = h.read(|v, cx| v.sidebar.read(cx).selected.clone());

    // Hover shows the repository, not the covered worktree's note.
    let (repo_tip, note_tip) = h.read(|v, cx| {
        let model = &v.sidebar.read(cx).model;
        let (repo, worktree) = model.worktree_for_path(&covered).unwrap();
        (
            repo.path.to_string_lossy().into_owned(),
            format!(
                "{}\n\n{}",
                worktree.note.as_deref().unwrap(),
                worktree.branch.as_deref().unwrap()
            ),
        )
    });
    hover(&mut h, at);
    assert!(tooltip(&mut h, &repo_tip), "{repo_tip}");
    assert!(!tooltip(&mut h, &note_tip), "{note_tip}");

    // The wheel over the name scrolls PROJECT.
    let before = scroll.offset().y;
    h.cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-40.0))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    settle(&mut h);
    assert!(
        scroll.offset().y < before,
        "the wheel over the sticky name scrolls PROJECT"
    );
    let at = scroll_under_sticky(&mut h);

    // Right-click: the repository's menu, not the covered worktree's.
    h.cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    h.cx.run_until_parked();
    let items: Vec<String> = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .expect("a menu opens")
            .items
            .iter()
            .map(|(label, _)| label.clone())
            .collect()
    });
    assert!(
        items.iter().any(|l| l == "New worktree...")
            && items.iter().any(|l| l == "Remove from sidebar"),
        "{items:?}"
    );
    assert!(
        !items
            .iter()
            .any(|l| l.starts_with("Delete worktree") || l == "Open terminal"),
        "{items:?}"
    );
    h.keys("escape");
    assert!(h.read(|v, _| v.context_menu.is_none()));

    // Left-click collapses the repository and leaves its header in view;
    // the covered worktree does not open.
    h.cx.simulate_click(at, Modifiers::none());
    settle(&mut h);
    h.read(|v, cx| {
        let sidebar = v.sidebar.read(cx);
        assert!(sidebar.model.repos[0].collapsed, "the repository collapses");
        assert!(!sidebar.model.repos[1].collapsed);
        assert_eq!(v.ws.tabs().len(), tabs, "no terminal opens");
        assert_eq!(sidebar.selected, selected, "the highlight stays");
        assert!(sidebar.project_scroll.max_offset().y > px(0.0));
    });
    assert!(h.cx.debug_bounds(selector).is_none());
    assert!(h.cx.debug_bounds("sticky-repo-context").is_none());
    let header = h.cx.debug_bounds("repo-0").unwrap();
    let viewport = scroll.bounds();
    assert!(
        (header.top() - viewport.top()).abs() < px(1.0),
        "the collapsed header {header:?} sits at PROJECT's top {viewport:?}"
    );
    // The real header expands it again.
    click_row(&mut h, "repo-0");
    assert!(h.read(|v, cx| !v.sidebar.read(cx).model.repos[0].collapsed));
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs);
}

#[gpui::test]
fn project_is_a_peer_section_and_collapses_without_hiding_sessions(cx: &mut TestAppContext) {
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
    let sessions = h.cx.debug_bounds("sessions-section-toggle").unwrap();
    let project =
        h.cx.debug_bounds("project-section-toggle")
            .expect("PROJECT is the second peer section");
    assert!(sessions.top() < project.top());
    assert!(
        h.cx.debug_bounds("active-section-toggle").is_none()
            && h.cx.debug_bounds("idle-section-toggle").is_none(),
        "Sessions replaces ACTIVE and IDLE"
    );
    assert!(h.cx.debug_bounds("repo-0").unwrap().top() >= project.bottom());
    assert!(h.cx.debug_bounds("repo-1").is_some());
    assert!(
        h.cx.debug_bounds("expand-all").is_none() && h.cx.debug_bounds("collapse-all").is_none()
    );
    click_row(&mut h, "project-section-toggle");
    assert!(h.cx.debug_bounds("repo-0").is_none());
    assert!(h.cx.debug_bounds("sessions-section-toggle").is_some());
    assert!(h.cx.debug_bounds("sessions-list").is_some());
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
    let bounds =
        |h: &mut Harness, selector: String| h.cx.debug_bounds(Box::leak(selector.into_boxed_str()));
    let agent_top = bounds(&mut h, format!("session-pane-{}", pane.raw()))
        .unwrap()
        .top();
    let shell_top = bounds(&mut h, format!("session-tab-{shell:?}"))
        .unwrap()
        .top();
    assert!(
        agent_top < shell_top,
        "agent split comes before ordinary shells"
    );
    assert!(
        bounds(&mut h, format!("session-tab-{agent:?}")).is_none(),
        "a tab with an agent pane is listed by its agent"
    );
    assert!(bounds(&mut h, format!("session-close-tab-{agent:?}")).is_none());
    assert!(
        bounds(&mut h, format!("session-close-pane-{}", pane.raw())).is_some(),
        "a working agent keeps its close control and confirms on click"
    );
    click_row(&mut h, &format!("session-close-tab-{shell:?}"));
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

fn draw_drag_frame(h: &mut Harness) {
    h.cx.run_until_parked();
    h.cx.update(|window, cx| {
        // simulate_next_frame drains callbacks; it does not draw the window.
        // Mouse events need the current layout and hitboxes, even when the
        // test platform's scheduled frame has not been delivered yet.
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
    h.cx.run_until_parked();
}

fn drag_grip_to(h: &mut Harness, x: f32) {
    draw_drag_frame(h);
    let grip =
        h.cx.debug_bounds("sidebar-grip")
            .expect("the grip is drawn");
    assert!(
        (f32::from(grip.right()) - width(h) as f32).abs() <= 2.0,
        "the displayed grip follows the configured width: {grip:?}"
    );
    let start = grip.center();
    // A pointer reaches the grip before pressing it. MouseDown alone does
    // not model the intervening hover/layout work after a previous drag.
    h.cx.simulate_mouse_move(start, None, Modifiers::none());
    draw_drag_frame(h);
    h.cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    draw_drag_frame(h);
    // A real drag crosses the window before moving beyond it. Four steps to
    // 5000px would jump outside the window on the very first move.
    let steps = ((x - f32::from(start.x)).abs() / 100.0).ceil().max(4.0) as usize;
    for step in 1..=steps {
        let x = f32::from(start.x) + (x - f32::from(start.x)) * step as f32 / steps as f32;
        h.cx.simulate_mouse_move(point(px(x), start.y), MouseButton::Left, Modifiers::none());
        draw_drag_frame(h);
    }
    h.cx.simulate_mouse_up(point(px(x), start.y), MouseButton::Left, Modifiers::none());
    draw_drag_frame(h);
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
fn config_reload_keeps_the_current_drag_width_and_applies_other_settings(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sidebar-drag-reload", |_| {});
    h.wait_prompt();
    let mut on_disk = h.read(|v, _| v.config.clone());
    on_disk.save(&h.home.config).unwrap();
    draw_drag_frame(&mut h);
    let start = h.cx.debug_bounds("sidebar-grip").unwrap().center();
    h.cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    draw_drag_frame(&mut h);
    h.cx.simulate_mouse_move(
        point(px(400.), start.y),
        MouseButton::Left,
        Modifiers::none(),
    );
    draw_drag_frame(&mut h);
    assert_eq!(width(&h), 400);

    // A queued notification for the previous saved width can arrive after
    // the pointer has moved. Other edited settings still reload normally.
    on_disk.tab_title = chda_config::TabTitle::Path;
    on_disk.save(&h.home.config).unwrap();
    h.cx.dispatch_action(crate::workspace_view::ReloadConfig);
    draw_drag_frame(&mut h);
    assert_eq!(width(&h), 400, "reload must not rewind an in-progress drag");
    assert_eq!(h.read(|v, _| v.ws.title_mode), chda_core::TitleMode::Path);
    h.cx.simulate_mouse_up(
        point(px(400.), start.y),
        MouseButton::Left,
        Modifiers::none(),
    );
    draw_drag_frame(&mut h);
    assert!(saved(&h).contains("sidebar-width = 400"));

    // Once the drag ends, a changed width in the file applies again.
    on_disk.sidebar_width = 240;
    on_disk.save(&h.home.config).unwrap();
    h.cx.dispatch_action(crate::workspace_view::ReloadConfig);
    draw_drag_frame(&mut h);
    assert_eq!(width(&h), 240);
    let grip = h.cx.debug_bounds("sidebar-grip").unwrap();
    assert!((f32::from(grip.right()) - 240.).abs() <= 2.0, "{grip:?}");
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
fn session_rows_stay_in_place_and_labels_toggle_persistently(cx: &mut TestAppContext) {
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
    h.wait_for("the Sessions branch alias", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
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
            let rows = &v.sidebar.read(cx).model.sessions;
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
    let at = h.cx.debug_bounds("sessions-label-toggle").unwrap().center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
    assert_eq!(h.read(|v, _| v.config.active_label), ActiveLabel::Branch);
    assert!(saved(&h).contains("active-label = \"branch\""));
    h.read(|v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
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
        "restored Sessions branch labels",
        |v, cx| {
            v.config.active_label == ActiveLabel::Branch
                && v.sidebar
                    .read(cx)
                    .model
                    .sessions
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
                .sessions
                .iter()
                .any(|r| r.title == "Fix login")
    });
    // Removing the note outside chda falls back to the actual branch name.
    git(&repo, &["config", "--unset", "branch.main.description"]);
    h.wait_for("alias fallback after the note is removed", |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
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
    let before = h.read(|v, cx| v.sidebar.read(cx).model.sessions.clone());
    let session = std::fs::read(h.home.data.join("session.json")).unwrap();
    let mut notices = h.cx.cx.notifications(&sidebar);
    h.cx.executor().advance_clock(Duration::from_secs(1));
    h.cx.run_until_parked();
    assert!(
        notices.next().now_or_never().is_some(),
        "idle ages repaint each second"
    );
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).model.sessions.clone()),
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

/// Hover `at` and wait past the tooltip delay.
fn hover(h: &mut Harness, at: gpui::Point<gpui::Pixels>) {
    h.cx.simulate_mouse_move(at, None, Modifiers::none());
    h.cx.run_until_parked();
    h.cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    h.cx.run_until_parked();
}

fn tooltip(h: &mut Harness, text: &str) -> bool {
    h.cx.debug_bounds(Box::leak(format!("tooltip: {text}").into_boxed_str()))
        .is_some()
}

/// The buttons in the Sessions and PROJECT headers show their own tooltips,
/// never the section toggle's, also when the pointer arrives from the toggle
/// with its tooltip showing.
#[gpui::test]
fn header_buttons_show_their_own_tooltips(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "header-tooltips", |home| {
        let repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("the repository", |v, cx| {
        v.sidebar.read(cx).model.repos.len() == 1
    });
    h.cx.run_until_parked();
    for (toggle, toggle_tip, button, button_tip) in [
        (
            "project-section-toggle",
            "Collapse PROJECT",
            "add-repo",
            "Add a repository (cmd-shift-o)",
        ),
        (
            "sessions-section-toggle",
            "Collapse Sessions",
            "sessions-label-toggle",
            "Switch Sessions labels between branch aliases and branch names",
        ),
    ] {
        let header = h.cx.debug_bounds(toggle).expect(toggle);
        // A point in the terminal, right of the sidebar.
        let away = point(header.right() + px(200.0), header.bottom() + px(100.0));
        hover(&mut h, point(header.left() + px(24.0), header.center().y));
        assert!(tooltip(&mut h, toggle_tip), "{toggle_tip} on the toggle");
        let at = h.cx.debug_bounds(button).expect(button).center();
        h.cx.simulate_mouse_move(at, None, Modifiers::none());
        h.cx.run_until_parked();
        assert!(
            !tooltip(&mut h, toggle_tip),
            "{toggle_tip} stays on {button}"
        );
        hover(&mut h, at);
        assert!(tooltip(&mut h, button_tip), "{button_tip} on {button}");
        assert!(!tooltip(&mut h, toggle_tip), "{toggle_tip} on {button}");
        // Straight onto the button from the terminal.
        hover(&mut h, away);
        assert!(!tooltip(&mut h, button_tip));
        hover(&mut h, at);
        assert!(tooltip(&mut h, button_tip), "{button_tip} on {button}");
        assert!(!tooltip(&mut h, toggle_tip), "{toggle_tip} on {button}");
        hover(&mut h, away);
    }
}
