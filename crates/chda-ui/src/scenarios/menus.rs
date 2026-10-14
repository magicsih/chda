//! Actions the menu bar adds.

use gpui::TestAppContext;

use super::harness::Harness;
use crate::workspace_view::{MenuAction, OpenConfig, ResumeSession, RunClaude};

#[gpui::test]
fn settings_writes_a_missing_config_file(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "settings", |_| {});
    h.wait_prompt();
    assert!(!h.home.config.exists());
    h.cx.dispatch_action(OpenConfig);
    h.cx.run_until_parked();
    let text = std::fs::read_to_string(&h.home.config).unwrap();
    assert!(text.contains("restore-session"), "{text}");
    assert_eq!(
        h.system.0.borrow().opened_files,
        vec![h.home.config.clone()]
    );
}

#[gpui::test]
fn agent_items_explain_why_nothing_ran(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "agentmenu", |_| {});
    h.wait_prompt();
    h.cx.dispatch_action(ResumeSession);
    h.cx.run_until_parked();
    assert!(h.read(|v, _| v.palette.is_none()));
    assert!(
        h.read(|v, _| v.notifications.latest().map(str::to_owned))
            .is_some_and(|s| s.contains("No agent sessions"))
    );

    // The test adapter reports Claude Code as not installed.
    h.cx.dispatch_action(RunClaude);
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v.notifications.latest().map(str::to_owned))
            .is_some_and(|s| s.contains("not on PATH"))
    );
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
}

#[gpui::test]
fn agent_presets_start_from_the_palette_and_the_sidebar_menu(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "presets", |home| {
        repo = home.repo("app");
        home.fake_claude();
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\"]\n\n[[agent-presets]]\nname = \"Opus\"\nagent = \"claude\"\nargs = [\"--model\", \"opus\"]\n",
                repo.display()
            ),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for("the repository's worktree", {
        let repo = repo.clone();
        move |v, cx| v.sidebar.read(cx).model.worktree_for_path(&repo).is_some()
    });

    h.keys("cmd-shift-p");
    h.type_text("Run Opus in app");
    h.keys("enter");
    h.confirm_agent_launch();
    h.wait_for("the preset in a new tab", |v, cx| {
        v.ws.tabs().len() == 2 && v.focused_text(cx).contains("--model opus")
    });
    let text = h.read(|v, cx| v.focused_text(cx));
    assert!(text.contains("fake-claude --settings"), "{text}");

    let action = MenuAction::RunPreset(repo.clone(), "Opus".into());
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.confirm_agent_launch();
    h.wait_for("a second preset tab", |v, cx| {
        v.ws.tabs().len() == 3 && v.focused_text(cx).contains("--model opus")
    });
}

#[gpui::test]
fn codex_legacy_sessions_are_listed_and_resume_with_a_slow_login_shell_path(
    cx: &mut TestAppContext,
) {
    use crate::sidebar_view::{SessionPick, SidebarEvent};
    use std::os::unix::fs::PermissionsExt;
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "codex-path", |home| {
        repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
        home.codex_session(&repo, "saved-session");
        let transcript = home
            .home
            .join(".codex/sessions/rollout-2026-10-01-saved-session.jsonl");
        let metadata = std::fs::read_to_string(&transcript).unwrap();
        std::fs::write(&transcript, format!("{metadata}{}\n{}\n",
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"resume this exact conversation"}]}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"resume this exact conversation"}}),
        )).unwrap();
        let bin = home.bin.join("codex");
        std::fs::create_dir_all(&home.bin).unwrap();
        std::fs::write(
            &bin,
            "#!/usr/bin/env sh\nprintf 'fake-codex %s\\n' \"$*\"\nexec /bin/cat >/dev/null\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(home.home.join("gui-path"), "/usr/bin:/bin:/usr/sbin:/sbin").unwrap();
        // Slower than the launch wait (#243): the PATH arrives afterwards.
        std::fs::write(
            home.home.join(".zprofile"),
            format!("sleep 3\nexport PATH='{}':$PATH\n", home.bin.display()),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for(
        "the default Codex conversation in the worktree resume list",
        {
            let repo = repo.clone();
            move |v, cx| {
                v.sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&repo)
                    .is_some_and(|(_, w)| {
                        w.sessions.len() == 1
                            && w.sessions[0].id == "saved-session"
                            && w.sessions[0].snippet == "resume this exact conversation"
                            && w.sessions[0].message_count == 1
                    })
            }
        },
    );
    let cwd = repo;
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                SidebarEvent::ResumeSessions(vec![SessionPick {
                    worktree: cwd,
                    agent: "codex".into(),
                    session: "saved-session".into(),
                }]),
                window,
                cx,
            );
        })
    });
    h.confirm_agent_launch();
    h.wait_for("Codex to resume through the imported PATH", |v, cx| {
        v.ws.tabs().len() == 2 && v.focused_text(cx).contains("resume saved-session")
    });
    assert!(h.read(|v, _| {
        v.notifications
            .latest()
            .is_none_or(|s| !s.contains("not on PATH"))
    }));
    assert!(h.read(|v, cx| v.focused_text(cx)).contains("fake-codex"));
}

fn hover(h: &mut Harness, at: gpui::Point<gpui::Pixels>) {
    h.cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    h.cx.run_until_parked();
    h.cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    h.cx.run_until_parked();
}
fn tip(h: &mut Harness, text: &'static str) -> bool {
    h.cx.debug_bounds(text).is_some()
}
fn click(h: &mut Harness, selector: &'static str) {
    let at =
        h.cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("Missing {selector}"))
            .center();
    h.cx.simulate_click(at, gpui::Modifiers::none());
    h.cx.run_until_parked();
}

#[gpui::test]
fn context_menu_removes_an_existing_tooltip_and_hover_recovers(cx: &mut TestAppContext) {
    use crate::sidebar_view::SidebarEvent;
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "menu-tooltip", |home| {
        repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("repository", |v, cx| {
        v.sidebar.read(cx).model.repos.len() == 1
    });
    let header = h.cx.debug_bounds("project-section-toggle").unwrap();
    let at = gpui::point(header.left() + gpui::px(24.), header.center().y);
    hover(&mut h, at);
    assert!(tip(&mut h, "tooltip: Collapse PROJECT"));
    // A menu opened by any source must also remove already rendered hints.
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::RepoMenu(repo.clone(), at), window, cx)
        })
    });
    h.cx.run_until_parked();
    assert!(h.cx.debug_bounds("menu-item-0").is_some());
    assert!(
        !tip(&mut h, "tooltip: Collapse PROJECT"),
        "a stale tooltip covers the menu"
    );
    hover(&mut h, at);
    assert!(
        !tip(&mut h, "tooltip: Collapse PROJECT"),
        "hover during the menu is suppressed"
    );
    h.keys("escape");
    hover(
        &mut h,
        gpui::point(
            header.right() + gpui::px(200.),
            header.bottom() + gpui::px(100.),
        ),
    );
    hover(&mut h, at);
    assert!(
        tip(&mut h, "tooltip: Collapse PROJECT"),
        "normal hover recovers after dismissal"
    );
}

#[gpui::test]
fn section_toggle_dismisses_old_caption_before_the_next_hover(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "toggle-tip", |_| {});
    h.wait_prompt();
    for (selector, old, new) in [
        (
            "sessions-section-toggle",
            "tooltip: Collapse Sessions",
            "tooltip: Expand Sessions",
        ),
        (
            "project-section-toggle",
            "tooltip: Collapse PROJECT",
            "tooltip: Expand PROJECT",
        ),
    ] {
        let b = h.cx.debug_bounds(selector).unwrap();
        let at = gpui::point(b.left() + gpui::px(24.), b.center().y);
        hover(&mut h, at);
        assert!(tip(&mut h, old));
        h.cx.update(|window, cx| {
            h.view.update(cx, |v, cx| {
                let event = if selector == "sessions-section-toggle" {
                    crate::sidebar_view::SidebarEvent::ToggleSessions
                } else {
                    crate::sidebar_view::SidebarEvent::ToggleProject
                };
                v.on_sidebar_event(event, window, cx);
            })
        });
        h.cx.run_until_parked();
        assert!(
            !tip(&mut h, old),
            "the caption stayed after its section changed"
        );
        hover(
            &mut h,
            gpui::point(b.right() + gpui::px(200.), b.bottom() + gpui::px(100.)),
        );
        hover(&mut h, at);
        assert!(
            tip(&mut h, new),
            "next hover must describe the current section state"
        );
    }
}

#[gpui::test]
fn usage_escape_and_close_restore_the_original_terminal_input(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "usage-input", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane());
    click(&mut h, "usage-provider-claude");
    assert!(h.read(|v, _| v.status_bar.details_open));
    h.keys("escape");
    assert!(
        !h.read(|v, _| v.status_bar.details_open),
        "Escape must close usage details"
    );
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), pane);
    h.run("printf 'usage-escape-input\\n'", "usage-escape-input\n");
    click(&mut h, "usage-provider-codex");
    click(&mut h, "usage-close");
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), pane);
    h.run("printf 'usage-close-input\\n'", "usage-close-input\n");
}

#[gpui::test]
fn launch_permissions_and_buttons_are_operable_without_the_mouse(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "launch-keys", |home| home.fake_claude());
    h.wait_prompt();
    h.cx.dispatch_action(RunClaude);
    h.cx.run_until_parked();
    h.keys("down tab enter");
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        1,
        "Tab reaches Cancel before Start"
    );
    assert!(h.cx.debug_bounds("launch-start").is_none());
    h.run("printf 'launch-cancel-input\\n'", "launch-cancel-input\n");
    h.cx.dispatch_action(RunClaude);
    h.cx.run_until_parked();
    h.keys("down tab tab space");
    h.wait_for("keyboard launch with chosen bypass", |v, cx| {
        v.focused_text(cx)
            .replace('\n', "")
            .contains("--dangerously-skip-permissions")
    });
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
    assert!(h.cx.debug_bounds("launch-start").is_none());
}

fn click_menu(h: &mut Harness, label: &str) {
    let index = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|(text, _)| text == label)
            .unwrap()
    });
    let selector = Box::leak(format!("menu-item-{index}").into_boxed_str());
    click(h, selector);
}
fn worktree_menu(h: &mut Harness, path: &std::path::Path) {
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    let selector = Box::leak(format!("wt:{}", path.display()).into_boxed_str());
    let at = h.cx.debug_bounds(selector).unwrap().center();
    h.cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
    h.cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
    h.cx.run_until_parked();
}
#[gpui::test]
fn nonnavigation_menus_return_input_and_new_terminal_keeps_its_destination(
    cx: &mut TestAppContext,
) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "menu-input", |home| {
        repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("project", |v, cx| {
        v.sidebar.read(cx).model.worktree_for_path(&repo).is_some()
    });
    let origin = h.read(|v, _| v.ws.focused_pane());
    worktree_menu(&mut h, &repo);
    click_menu(&mut h, "Add to Starred");
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), origin);
    h.run("printf 'star-menu-input\\n'", "star-menu-input\n");
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                crate::sidebar_view::SidebarEvent::RepoMenu(
                    repo.clone(),
                    gpui::point(gpui::px(100.), gpui::px(100.)),
                ),
                window,
                cx,
            )
        })
    });
    h.cx.run_until_parked();
    click_menu(&mut h, "Refresh");
    h.run("printf 'refresh-menu-input\\n'", "refresh-menu-input\n");
    // Terminal link actions use the same menu/focus result as mouse hit testing.
    for (label, marker) in [
        ("Copy link", "copy-link-menu-input"),
        ("Open link", "open-link-menu-input"),
    ] {
        let term = h.focused_terminal();
        h.cx.update(|_, cx| {
            term.update(cx, |_, cx| {
                cx.emit(crate::terminal_view::TerminalEvent::LinkMenu {
                    link: crate::terminal_view::LinkOpen::Url("https://example.com/fixture".into()),
                    position: gpui::point(gpui::px(400.), gpui::px(200.)),
                    at_prompt: true,
                })
            })
        });
        h.cx.run_until_parked();
        click_menu(&mut h, label);
        assert_eq!(h.read(|v, _| v.ws.focused_pane()), origin);
        h.run(&format!("printf '{marker}\\n'"), &format!("{marker}\n"));
    }
    let term = h.focused_terminal();
    h.cx.update(|_, cx| {
        term.update(cx, |_, cx| {
            cx.emit(crate::terminal_view::TerminalEvent::LinkMenu {
                link: crate::terminal_view::LinkOpen::Path {
                    path: repo.clone(),
                    line: None,
                    column: None,
                    is_dir: true,
                },
                position: gpui::point(gpui::px(400.), gpui::px(200.)),
                at_prompt: true,
            })
        })
    });
    h.cx.run_until_parked();
    click_menu(
        &mut h,
        &format!("Reveal in {}", crate::platform::file_manager_name()),
    );
    h.run("printf 'reveal-menu-input\\n'", "reveal-menu-input\n");
    worktree_menu(&mut h, &repo);
    click_menu(&mut h, "Open terminal");
    h.wait_prompt();
    assert_ne!(h.read(|v, _| v.ws.focused_pane()), origin);
    h.run(
        "printf 'destination-menu-input\\n'",
        "destination-menu-input\n",
    );
    h.wait_for("destination directory", |v, _| {
        v.ws.pane(v.ws.focused_pane().unwrap())
            .unwrap()
            .cwd
            .as_ref()
            == Some(&repo)
    });
}

#[gpui::test]
fn failed_navigation_menu_keeps_input_in_its_original_pane(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "failed-menu-nav", |home| {
        repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    let origin = h.read(|v, _| v.ws.focused_pane());
    h.wait_for("worktree", |v, cx| {
        v.sidebar.read(cx).model.worktree_for_path(&repo).is_some()
    });
    worktree_menu(&mut h, &repo);
    // The row had a diff when rendered, but it disappeared before activation.
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::ViewDiff(repo.clone()),
                window,
                cx,
            )
        })
    });
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), origin);
    h.run(
        "printf 'failed-navigation-input\\n'",
        "failed-navigation-input\n",
    );
}

#[gpui::test]
fn usage_close_keeps_a_pane_selected_while_the_popup_was_open(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "usage-new-focus", |_| {});
    h.wait_prompt();
    let first = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.keys("cmd-t");
    h.wait_prompt();
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.keys("cmd-1");
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.toggle_usage(window, cx)));
    h.keys("cmd-2");
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(second));
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.close_usage(window, cx)));
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(second));
    assert_ne!(Some(first), Some(second));
    h.run("printf 'usage-new-pane-input\\n'", "usage-new-pane-input\n");
}
