//! Activity, exact restore directory, child protection and process retention.
use super::harness::{Harness, wait_until};
use chda_core::agents::{ChildEvent, ChildState, HookEvent, HookKind, ipc};
use gpui::TestAppContext;
use std::time::Duration;

#[gpui::test]
fn quiet_panes_keep_activity_when_the_theme_and_size_change(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "quiet-activity", |_| {});
    h.wait_prompt();
    h.run(
        "stty -echo; printf '\nquiet-ready-one\n'; exec cat",
        "\nquiet-ready-one\n",
    );
    h.keys("cmd-t");
    h.wait_prompt();
    h.run(
        "stty -echo; printf '\nquiet-ready-two\n'; exec cat",
        "\nquiet-ready-two\n",
    );
    std::thread::sleep(Duration::from_millis(1100));
    h.cx.run_until_parked();
    let before = h.read(|v, _| {
        v.ws.tabs()
            .iter()
            .flat_map(|t| t.panes())
            .map(|p| {
                let info = v.ws.pane(p).unwrap();
                (p, info.last_activity, info.previous_activity)
            })
            .collect::<Vec<_>>()
    });
    std::fs::write(&h.home.ghostty, "font-size = 18\nbackground = #102030\n").unwrap();
    h.wait_for("new terminal frames in every quiet pane", |v, cx| {
        v.panes.values().all(|(p, _)| {
            p.read(cx).frame().background
                == chda_term::Rgb {
                    r: 16,
                    g: 32,
                    b: 48,
                }
        })
    });
    h.read(|v, _| {
        for (pane, activity, previous) in before {
            let info = v.ws.pane(pane).unwrap();
            assert_eq!(
                info.last_activity, activity,
                "theme/resize changed quiet activity"
            );
            assert_eq!(info.previous_activity, previous);
        }
    });
}

#[gpui::test]
fn exact_hook_directory_wins_over_a_stale_shell_directory_on_restore(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "restore-agent-cwd", |home| {
        repo = home.repo("agent-project");
        home.fake_claude();
        home.claude_session(&repo, "cwd-session", "directory fixture");
        std::fs::write(
            &home.claude_bin,
            "#!/bin/sh\nprintf 'resumed-cwd:%s\\n' \"$PWD\"\nexec cat >/dev/null\n",
        )
        .unwrap();
    });
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &repo,
        HookKind::SessionStart,
        "cwd-session",
    );
    h.wait_for("exact conversation", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "cwd-session")
    });
    assert_ne!(
        h.read(|v, _| v.ws.pane(pane).unwrap().cwd.clone()),
        Some(repo.clone()),
        "fixture requires the stale shell directory"
    );
    let (mut cx2, view2) = h.reopen();
    wait_until(&mut cx2, &view2, "CLI resume directory", |v, cx| {
        v.focused_text(cx).contains("resumed-cwd:")
    });
    let text = view2.read_with(&cx2, |v, cx| v.focused_text(cx));
    assert!(
        text.contains(&format!("resumed-cwd:{}", repo.display())),
        "{text}"
    );
}

#[gpui::test]
fn idle_parent_with_a_busy_child_requires_cancel_by_default(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "close-busy-child", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "parent",
    );
    h.wait_for("idle parent", |v, _| v.ws.pane(pane).unwrap().agent_live);
    ipc::send(
        &ipc::socket_path(&h.home.data),
        &HookEvent {
            agent: "claude".into(),
            session_id: "parent".into(),
            cwd: h.home.home.clone(),
            kind: HookKind::Stopped,
            timestamp: crate::terminal_view::now_ms(),
            pane: Some(pane.raw()),
            child: Some(ChildEvent {
                id: "child".into(),
                label: None,
                state: ChildState::Working,
                started: true,
            }),
        },
    )
    .unwrap();
    h.wait_for("child update", |v, cx| {
        !v.sidebar.read(cx).child_groups.is_empty()
    });
    h.keys("cmd-w");
    assert!(
        h.read(|v, _| v.confirm.is_some()),
        "busy child was closed without confirmation"
    );
    h.keys("enter");
    assert!(
        h.read(|v, _| v.confirm.is_none()),
        "Enter must activate default Cancel"
    );
    assert!(h.read(|v, _| v.ws.pane(pane).is_some() && v.panes.contains_key(&pane)));
}

#[gpui::test]
fn ordinary_process_exit_retains_background_layout_and_output(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "ordinary-exit", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let release = h.home.home.join("exit-release");
    h.type_text(&format!(
        "while [ ! -f '{}' ]; do sleep 0.02; done; printf 'retained-exit-output\\n'; exit 7",
        release.display()
    ));
    h.keys("enter");
    h.keys("cmd-t");
    h.wait_prompt();
    let focused = h.read(|v, _| v.ws.focused_pane());
    std::fs::write(&release, "").unwrap();
    h.wait_for("ordinary top-level exit", |v, _| {
        !v.panes.contains_key(&pane)
    });
    h.read(|v, _| {
        assert!(v.ws.pane(pane).is_some(), "ordinary exit removed its pane");
        assert_eq!(v.ws.tabs().len(), 2);
        assert_eq!(v.ws.focused_pane(), focused);
        assert!(
            v.ws.pane(pane)
                .unwrap()
                .pane_run
                .as_ref()
                .is_some_and(|r| r.stopped())
        );
        let file =
            v.ws.pane(pane)
                .unwrap()
                .previous_run
                .as_ref()
                .expect("final output reference");
        let text = std::fs::read_to_string(h.home.data.join("agent-output").join(file)).unwrap();
        assert!(text.contains("retained-exit-output"), "{text}");
    });
}

#[gpui::test]
fn missing_confirmed_agent_directory_keeps_a_stopped_pane_without_a_resume(
    cx: &mut TestAppContext,
) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "missing-agent-cwd", |home| {
        repo = home.repo("missing-project");
        home.fake_claude();
        home.claude_session(&repo, "missing-cwd", "fixture");
    });
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &repo,
        HookKind::SessionStart,
        "missing-cwd",
    );
    h.wait_for("confirmed cwd saved", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.cwd == Some(repo.clone()))
    });
    std::fs::remove_dir_all(&repo).unwrap();
    let (cx2, view2) = h.reopen();
    cx2.run_until_parked();
    view2.read_with(&cx2, |v, _| {
        let pane = v.ws.focused_pane().unwrap();
        assert!(
            !v.panes.contains_key(&pane),
            "resume or fallback shell started in another directory"
        );
        let info = v.ws.pane(pane).unwrap();
        assert_eq!(info.cwd.as_ref(), Some(&repo));
        assert!(
            info.pane_run
                .as_ref()
                .unwrap()
                .label()
                .contains("no conversation was resumed")
        );
        assert!(
            info.agent_session.is_some(),
            "exact identity remains available for diagnosis"
        );
    });
}

#[gpui::test]
fn committed_input_without_echo_counts_as_activity(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "input-activity", |_| {});
    h.wait_prompt();
    h.run(
        "stty -echo; printf '\nno-echo-ready\n'; exec cat",
        "\nno-echo-ready\n",
    );
    h.cx.run_until_parked();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let before = h.read(|v, _| v.ws.pane(pane).unwrap().last_activity);
    let visible = h.read(|v, cx| v.focused_text(cx));
    h.type_text("input-without-echo");
    h.wait_for("successful input activity", |v, _| {
        v.ws.pane(pane).unwrap().last_activity > before
    });
    assert_eq!(
        h.read(|v, cx| v.focused_text(cx)),
        visible,
        "the fixture must produce no PTY output"
    );
}

fn click(h: &mut Harness, selector: String) {
    h.cx.run_until_parked();
    let bounds =
        h.cx.debug_bounds(Box::leak(selector.into_boxed_str()))
            .expect("control bounds");
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.cx.run_until_parked();
}

#[gpui::test]
fn every_close_path_reads_latest_child_state_and_completed_children_do_not_block(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "child-close-paths", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "parent-close",
    );
    h.wait_for("parent identity", |v, _| {
        v.ws.pane(pane).unwrap().agent_live
    });
    h.keys("cmd-t");
    h.wait_prompt();
    let focus = h.read(|v, _| v.ws.focused_pane());
    let mut at = crate::terminal_view::now_ms();
    for state in [
        ChildState::Working,
        ChildState::WaitingInput,
        ChildState::Unknown,
    ] {
        at += 1;
        // Keep the already rendered close control. Its handler must read this
        // newly arrived board state rather than rely on the old row snapshot.
        h.view.update(&mut h.cx, |v, _| {
            v.env
                .windows
                .borrow_mut()
                .children
                .apply(chda_core::agents::ChildActivity {
                    agent: "claude".into(),
                    parent: "parent-close".into(),
                    observed_at: at,
                    child: ChildEvent {
                        id: "late-child".into(),
                        label: None,
                        state,
                        started: state == ChildState::Working,
                    },
                });
        });
        click(&mut h, format!("session-close-pane-{}", pane.raw()));
        assert!(
            h.read(|v, _| v.confirm.is_some()),
            "Sessions close ignored {state:?}"
        );
        h.keys("enter");
        assert_eq!(h.read(|v, _| v.ws.focused_pane()), focus);
        assert!(h.read(|v, _| v.ws.pane(pane).is_some()));
        click(&mut h, "tab-close-0".into());
        assert!(
            h.read(|v, _| v.confirm.is_some()),
            "tab close ignored {state:?}"
        );
        h.keys("escape");
        h.keys("cmd-1 cmd-w");
        assert!(
            h.read(|v, _| v.confirm.is_some()),
            "shortcut ignored {state:?}"
        );
        h.keys("tab shift-tab space");
        assert!(h.read(|v, _| v.confirm.is_none() && v.ws.pane(pane).is_some()));
        h.keys("cmd-2");
    }
    h.view.update(&mut h.cx, |v, _| {
        v.env
            .windows
            .borrow_mut()
            .children
            .apply(chda_core::agents::ChildActivity {
                agent: "claude".into(),
                parent: "parent-close".into(),
                observed_at: at + 1,
                child: ChildEvent {
                    id: "late-child".into(),
                    label: None,
                    state: ChildState::TurnComplete,
                    started: false,
                },
            });
    });
    click(&mut h, format!("session-close-pane-{}", pane.raw()));
    assert!(h.read(|v, _| v.confirm.is_none() && v.ws.pane(pane).is_none()));
}

#[gpui::test]
fn last_shell_exit_survives_cold_restore_and_only_explicit_start_creates_a_shell(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "last-shell-exit", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let pid = h.focused_terminal().read_with(&h.cx, |t, _| t.child_pid());
    h.type_text("printf 'last-shell-output\\n'; exit 5");
    h.keys("enter");
    h.wait_for("last shell retained", |v, _| {
        v.ws.pane(pane)
            .is_some_and(|p| p.pane_run.as_ref().is_some_and(chda_core::PaneRun::stopped))
    });
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
    assert!(h.read(|v, _| v.panes.is_empty()));
    let (cx2, view2) = h.reopen();
    cx2.run_until_parked();
    view2.read_with(&cx2, |v, _| {
        assert_eq!(v.ws.tabs().len(), 1);
        assert!(
            v.panes.is_empty(),
            "cold restore restarted the ended process"
        );
        assert!(
            v.ws.pane(v.ws.focused_pane().unwrap())
                .unwrap()
                .pane_run
                .as_ref()
                .unwrap()
                .label()
                .contains("code 5")
        );
    });
    h.type_text("must-not-reach-a-process");
    assert!(h.read(|v, _| v.panes.is_empty()));
    click(&mut h, "start-shell".into());
    h.wait_prompt();
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(pane));
    assert_ne!(
        h.focused_terminal().read_with(&h.cx, |t, _| t.child_pid()),
        pid
    );
    assert!(h.read(|v, _| v.ws.pane(pane).unwrap().previous_run.is_some()));
    h.run("printf 'new-shell-ready\\n'", "new-shell-ready");
}

#[gpui::test]
fn managed_restart_uses_the_same_conversations_latest_confirmed_directory(cx: &mut TestAppContext) {
    let mut report_dir = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "managed-cwd", |home| {
        report_dir = home.repo("reported");
        home.fake_claude();
        home.claude_session(&home.home, "restart-cwd", "fixture");
        std::fs::write(
            &home.claude_bin,
            r#"#!/bin/sh
case " $* " in
  *' --resume restart-cwd '*) printf '%s' "$PWD" > "$HOME/resumed-directory"; exec cat >/dev/null ;;
esac
cd "$HOME/src/reported" || exit 8
printf 'managed-directory-ready\n'
while [ ! -f "$HOME/managed-release" ]; do sleep 0.02; done
exit 6
"#,
        )
        .unwrap();
    });
    h.wait_prompt();
    h.cx.dispatch_action(crate::workspace_view::RunClaude);
    h.confirm_agent_launch();
    h.wait_for("managed agent changed directory", |v, cx| {
        v.focused_text(cx).contains("managed-directory-ready")
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &report_dir,
        HookKind::SessionStart,
        "restart-cwd",
    );
    h.wait_for("same conversation confirmed directory", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.cwd == Some(report_dir.clone()))
    });
    std::fs::write(h.home.home.join("managed-release"), "").unwrap();
    h.wait_for("managed stopped", |v, _| !v.panes.contains_key(&pane));
    click(&mut h, "agent-restart".into());
    let resumed = h.home.home.join("resumed-directory");
    h.wait_for("exact resumed directory", |_, _| resumed.exists());
    assert_eq!(
        std::fs::read_to_string(resumed).unwrap(),
        report_dir.to_string_lossy()
    );
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(pane));
}

#[gpui::test]
fn stopped_parent_with_retained_child_reports_closes_without_hidden_confirmation(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "stopped-child", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &h.home.home,
        HookKind::SessionStart,
        "ended-parent",
    );
    h.wait_for("parent identity", |v, _| {
        v.ws.pane(pane).unwrap().agent_session.is_some()
    });
    h.view.update(&mut h.cx, |v, _| {
        v.env
            .windows
            .borrow_mut()
            .children
            .apply(chda_core::agents::ChildActivity {
                agent: "claude".into(),
                parent: "ended-parent".into(),
                observed_at: crate::terminal_view::now_ms(),
                child: ChildEvent {
                    id: "retained-child".into(),
                    label: None,
                    state: ChildState::Unknown,
                    started: false,
                },
            });
    });
    h.type_text("exit 0");
    h.keys("enter");
    h.wait_for("stopped parent", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .pane_run
            .as_ref()
            .is_some_and(chda_core::PaneRun::stopped)
    });
    h.keys("cmd-t");
    h.wait_prompt();
    click(&mut h, "tab-close-0".into());
    assert!(
        h.read(|v, _| v.confirm.is_none()),
        "stopped parent asked to stop a hidden child"
    );
    assert!(h.read(|v, _| v.ws.pane(pane).is_none()));
}
