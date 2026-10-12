//! Agent status per pane: tab dots, the Sessions list, the Dock badge,
//! notifications, "go to waiting agent" and notification clicks.

use chda_core::AgentStatus;
use chda_core::agents::HookKind;
use gpui::TestAppContext;

use super::harness::Harness;

#[gpui::test]
fn hook_events_drive_status_badge_jump_and_notification_click(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "agents", |_| {});
    h.wait_prompt();
    h.keys("cmd-t");
    h.wait_prompt();
    let (first, second) = h.read(|v, _| {
        let tabs = v.ws.tabs();
        (tabs[0].panes()[0], tabs[1].panes()[0])
    });
    let home = h.home.home.clone();

    h.hook(Some(first.raw()), &home, HookKind::PromptSubmitted);
    h.hook(Some(first.raw()), &home, HookKind::WaitingInput);
    h.wait_for("the first tab to wait for input", move |v, _| {
        v.ws.tab_agent(&v.ws.tabs()[0])
            .is_some_and(|a| a.status == AgentStatus::WaitingInput)
    });
    h.read(|v, cx| {
        let sessions = &v.sidebar.read(cx).model.sessions;
        let row = sessions
            .iter()
            .find(|r| r.key == chda_core::SessionKey::Pane(first))
            .unwrap();
        assert_eq!(
            row.status,
            Some(AgentStatus::WaitingInput),
            "Sessions agrees"
        );
        assert_eq!(row.status_label(), "Waiting for input");
    });
    assert_eq!(h.system.0.borrow().badge, 1, "one unseen waiting agent");
    {
        let recorded = h.system.0.borrow();
        let (title, _, target) = recorded.notifications.last().unwrap();
        assert!(title.contains("is waiting"), "{title}");
        assert_eq!(target.pane, Some(first.raw()));
    }

    // Going to the waiting agent focuses its pane and clears the badge.
    h.keys("cmd-shift-a");
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(first));
    assert_eq!(h.system.0.borrow().badge, 0);

    // A finished turn in the other tab is reviewed by clicking the
    // notification for it.
    h.hook(Some(second.raw()), &home, HookKind::Stopped);
    h.wait_for("the second tab to finish", move |v, _| {
        v.ws.pane(second)
            .and_then(|p| p.agent.as_ref())
            .is_some_and(|a| a.status == AgentStatus::Review)
    });
    let target = h.system.0.borrow().notifications.last().unwrap().2.clone();
    let click = h.system.0.borrow_mut().click.take().unwrap();
    click(target);
    h.wait_for("the notification to focus the second pane", move |v, _| {
        v.ws.focused_pane() == Some(second)
            && v.ws
                .pane(second)
                .unwrap()
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Idle)
    });
}

#[gpui::test]
fn gemini_copilot_and_opencode_report_like_claude_code(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "moreagents", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.tabs()[0].panes()[0]);
    let home = h.home.home.clone();
    for (agent, name) in [
        ("gemini", "Gemini CLI"),
        ("copilot", "Copilot CLI"),
        ("opencode", "OpenCode"),
    ] {
        h.hook_from(
            agent,
            Some(pane.raw()),
            &home,
            HookKind::PromptSubmitted,
            "s",
        );
        h.wait_for("the agent to work", move |v, _| {
            v.ws.pane(pane)
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.agent == agent && a.status == AgentStatus::Working)
        });
        h.hook_from(agent, Some(pane.raw()), &home, HookKind::WaitingInput, "s");
        h.wait_for("the agent to wait", move |v, _| {
            v.ws.pane(pane)
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.status == AgentStatus::WaitingInput)
        });
        let title = h.system.0.borrow().notifications.last().unwrap().0.clone();
        assert_eq!(title, format!("{name} is waiting"));
        h.hook_from(agent, Some(pane.raw()), &home, HookKind::Stopped, "s");
        h.wait_for("the completed agent awaits review", move |v, _| {
            v.ws.pane(pane)
                .unwrap()
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Review)
        });
        h.cx.update(|window, cx| {
            h.view.update(cx, |v, cx| {
                v.on_sidebar_event(
                    crate::sidebar_view::SidebarEvent::FocusPane(pane),
                    window,
                    cx,
                );
            })
        });
        h.wait_for("the live agent becomes idle after review", move |v, cx| {
            v.sidebar.read(cx).model.sessions.iter().any(|r| {
                r.key == chda_core::SessionKey::Pane(pane)
                    && r.live_idle()
                    && r.agent.as_deref() == Some(agent)
            })
        });
        h.hook_from(agent, Some(pane.raw()), &home, HookKind::SessionEnd, "s");
        h.wait_for("the session to end", move |v, _| {
            v.ws.pane(pane).is_some_and(|p| p.agent.is_none())
        });
    }
}

/// Type `codex` at a real shell prompt, without a menu launch or injected
/// hook events. A CLI stand-in emits Codex 0.160's actual OSC title format.
#[gpui::test]
fn directly_typed_codex_tracks_each_turn_and_exits(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::open(cx, "codexdirect", |home| {
        let repo = home.repo("app");
        std::fs::write(
            &home.config,
            format!("repos = [{}]\n", serde_json::to_string(&repo).unwrap()),
        )
        .unwrap();
        std::fs::create_dir_all(&home.bin).unwrap();
        let script = home.bin.join("codex");
        std::fs::write(
            &script,
            r#"#!/bin/sh
[ "$1" = '-c' ] && [ "$2" = "$CHDA_CODEX_TITLE_CONFIG" ] || exit 42
shift 2
[ "$1" = 'resume' ] && [ "$2" = 'argument with spaces' ] || exit 43
control="$HOME/codex-step-$CHDA_PANE_ID"
printf '\033]0;codex | Ready\007'
echo fake-codex-ready
while :; do
    if [ ! -f "$control" ]; then sleep 0.02; continue; fi
    step=$(cat "$control")
    rm "$control"
    case "$step" in
        work) title="⠋ codex | Thinking" ;;
        spin) title="⠙ codex | Working" ;;
        background) title="codex | Waiting" ;;
        wait) title="[ ! ] Action Required | codex" ;;
        blink) title="[ . ] Action Required | codex" ;;
        ready) title="codex | Ready" ;;
        clear) title="" ;;
        exit) exit 7 ;;
        *) exit 44 ;;
    esac
    printf '\033]0;%s\007' "$title"
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
    });
    h.wait_prompt();
    let repo = h.home.home.join("src/app");
    h.run(
        &format!(
            "export PATH={}:$PATH; cd {}",
            chda_term::shell_quote(&h.home.bin.to_string_lossy()),
            chda_term::shell_quote(&repo.to_string_lossy())
        ),
        "test%",
    );
    let repo_ready = repo.clone();
    h.wait_for("the branch terminal's working directory", move |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .and_then(|p| p.cwd.as_ref())
            == Some(&repo_ready)
    });
    h.run("codex resume 'argument with spaces'", "fake-codex-ready");
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    assert!(h.read(|v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == AgentStatus::Idle)
    }));
    h.keys("cmd-t");
    h.wait_prompt();
    let notify_created_before_title = crate::terminal_view::now_ms();
    for (step, status) in [
        ("work", AgentStatus::Working),
        ("spin", AgentStatus::Working),
        ("background", AgentStatus::Working),
        ("wait", AgentStatus::WaitingInput),
        ("blink", AgentStatus::WaitingInput),
        ("ready", AgentStatus::Review),
        ("work", AgentStatus::Working),
        ("ready", AgentStatus::Review),
    ] {
        std::fs::write(h.home.home.join(format!("codex-step-{}", pane.raw())), step).unwrap();
        let step_path = h.home.home.join(format!("codex-step-{}", pane.raw()));
        h.wait_for(step, move |v, cx| {
            !step_path.exists()
                && v.ws
                    .pane(pane)
                    .and_then(|p| p.agent.as_ref())
                    .is_some_and(|a| a.status == status)
                && v.sidebar.read(cx).model.repos[0].worktrees[0].status() == status
        });
    }
    assert_eq!(
        h.system
            .0
            .borrow()
            .notifications
            .iter()
            .filter(|(title, _, _)| title == "Codex is waiting")
            .count(),
        1,
        "blinking titles must not send duplicate notifications"
    );
    // A menu-launch completion notify can arrive after the title. Keep its
    // real session ID for restore without announcing the turn twice.
    let notifications = h.system.0.borrow().notifications.len();
    chda_core::agents::ipc::send(
        &chda_core::agents::ipc::socket_path(&h.home.data),
        &chda_core::agents::HookEvent {
            agent: "codex".into(),
            session_id: "real-session".into(),
            cwd: repo.clone(),
            kind: HookKind::Stopped,
            timestamp: notify_created_before_title,
            pane: Some(pane.raw()),
            child: None,
        },
    )
    .unwrap();
    h.wait_for("notify records the conversation", move |v, _| {
        v.ws.pane(pane)
            .and_then(|p| p.agent_session.as_ref())
            .is_some_and(|s| s.session == "real-session")
    });
    assert_eq!(h.system.0.borrow().notifications.len(), notifications);

    // Start another directly typed Codex in the same branch. Finishing or
    // exiting the first one must leave the branch working for the second.
    h.run(
        &format!(
            "export PATH={}:$PATH; codex resume 'argument with spaces'",
            chda_term::shell_quote(&h.home.bin.to_string_lossy())
        ),
        "fake-codex-ready",
    );
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    std::fs::write(
        h.home.home.join(format!("codex-step-{}", second.raw())),
        "work",
    )
    .unwrap();
    h.wait_for("the second Codex works", move |v, _| {
        v.ws.pane(second)
            .and_then(|p| p.agent.as_ref())
            .is_some_and(|a| a.status == AgentStatus::Working)
    });
    let notify_before_exit = crate::terminal_view::now_ms();
    std::fs::write(
        h.home.home.join(format!("codex-step-{}", pane.raw())),
        "clear",
    )
    .unwrap();
    h.wait_for("a cleared title ends tracking", move |v, cx| {
        v.ws.pane(pane)
            .is_some_and(|p| p.title.is_empty() && p.agent.is_none() && p.agent_session.is_none())
            && v.sidebar.read(cx).model.repos[0].worktrees[0].status() == AgentStatus::Working
    });
    chda_core::agents::ipc::send(
        &chda_core::agents::ipc::socket_path(&h.home.data),
        &chda_core::agents::HookEvent {
            agent: "codex".into(),
            session_id: "real-session".into(),
            cwd: repo.clone(),
            kind: HookKind::Stopped,
            timestamp: notify_before_exit,
            pane: Some(pane.raw()),
            child: None,
        },
    )
    .unwrap();
    // The following message acknowledges that the earlier IPC reply was drained.
    h.hook_from(
        "codex",
        Some(second.raw()),
        &repo,
        HookKind::Stopped,
        "second-real-session",
    );
    h.wait_for(
        "second conversation after the late ended notify",
        move |v, _| {
            v.ws.pane(second)
                .unwrap()
                .agent_session
                .as_ref()
                .is_some_and(|s| s.session == "second-real-session")
        },
    );
    assert!(
        h.read(|v, _| v
            .ws
            .pane(pane)
            .is_some_and(|p| p.agent.is_none() && p.agent_session.is_none())),
        "late notify revived an ended Codex conversation"
    );
    std::fs::write(
        h.home.home.join(format!("codex-step-{}", pane.raw())),
        "exit",
    )
    .unwrap();
    h.wait_for("Codex exits back to the shell", move |v, cx| {
        v.ws.pane(pane)
            .is_some_and(|p| !p.title.is_empty() && p.agent.is_none() && p.agent_session.is_none())
            && v.sidebar.read(cx).model.repos[0].worktrees[0].status() == AgentStatus::Working
    });
    std::fs::write(
        h.home.home.join(format!("codex-step-{}", second.raw())),
        "exit",
    )
    .unwrap();
    h.wait_for("the final Codex exits", move |v, cx| {
        v.ws.pane(second).is_some_and(|p| p.agent.is_none())
            && v.sidebar.read(cx).model.repos[0].worktrees[0].status() == AgentStatus::Idle
    });
}

#[gpui::test]
fn working_spinner_turns_only_while_an_agent_works(cx: &mut TestAppContext) {
    use crate::status_icon::SPIN_STEP;
    use futures::{FutureExt, StreamExt};

    let mut h = Harness::open(cx, "spinner", |_| {});
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.tabs()[0].panes()[0]);
    let home = h.home.home.clone();
    let sidebar = h.read(|v, _| v.sidebar.clone());
    let spin = |h: &Harness| h.read(|v, cx| v.sidebar.read(cx).spin);

    h.hook(Some(pane.raw()), &home, HookKind::PromptSubmitted);
    h.wait_for("the agent to work", move |v, _| {
        v.ws.pane(pane)
            .and_then(|p| p.agent.as_ref())
            .is_some_and(|a| a.status == AgentStatus::Working)
    });
    let start = spin(&h);
    let mut notices = h.cx.cx.notifications(&sidebar);
    for _ in 0..3 {
        h.cx.executor().advance_clock(SPIN_STEP);
        h.cx.run_until_parked();
    }
    assert_eq!(spin(&h), start + 3, "the sidebar spinner turns each step");
    assert!(notices.next().now_or_never().is_some());

    // Waiting for input is a still icon: the timer stops with the work.
    h.hook(Some(pane.raw()), &home, HookKind::WaitingInput);
    h.wait_for("the agent to wait", move |v, _| {
        v.ws.pane(pane)
            .and_then(|p| p.agent.as_ref())
            .is_some_and(|a| a.status == AgentStatus::WaitingInput)
    });
    h.cx.executor().advance_clock(SPIN_STEP);
    h.cx.run_until_parked();
    assert!(
        !h.read(|v, _| v.spinning),
        "no timer without a working agent"
    );
    // Only the once-a-second Sessions age tick repaints the sidebar now.
    while notices.next().now_or_never().is_some() {}
    let mut repaints = 0;
    for _ in 0..16 {
        h.cx.executor().advance_clock(SPIN_STEP);
        h.cx.run_until_parked();
        while notices.next().now_or_never().is_some() {
            repaints += 1;
        }
    }
    assert!(
        repaints <= 3,
        "a waiting agent repainted the sidebar {repaints} times in two seconds"
    );
}

/// A new conversation's first turn in a managed Codex launch. Codex 0.162
/// generates the conversation title through an ephemeral thread that still
/// runs `notify` mid-turn, with nothing saved for it. The pane must keep
/// working, keep no identity but the real conversation's, announce the turn
/// once, and ask before ✕ or cmd-w stop the working agent.
#[gpui::test]
fn codex_first_turn_helper_notify_keeps_working_and_close_confirms(cx: &mut TestAppContext) {
    use crate::workspace_view::MenuAction;
    use chda_core::agents::AgentId;
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::open(cx, "codex-first-turn", |home| {
        let repo = home.repo("app");
        std::fs::write(
            &home.config,
            format!("repos = [{}]\n", serde_json::to_string(&repo).unwrap()),
        )
        .unwrap();
        std::fs::create_dir_all(&home.bin).unwrap();
        let script = home.bin.join("codex");
        // Codex 0.160's OSC title layout, as chda requests it.
        std::fs::write(
            &script,
            r#"#!/bin/sh
control="$HOME/codex-step-$CHDA_PANE_ID"
printf '\033]0;codex | Ready\007'
echo fake-codex-ready
while :; do
    if [ ! -f "$control" ]; then sleep 0.02; continue; fi
    step=$(cat "$control")
    rm "$control"
    case "$step" in
        work) title="⠋ codex | Thinking" ;;
        spin) title="⠙ codex | Working" ;;
        spin-again) title="⠹ codex | Working" ;;
        ready) title="codex | Ready" ;;
        *) exit 44 ;;
    esac
    printf '\033]0;%s\007' "$title"
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            home.home.join("gui-path"),
            format!("{}:/usr/bin:/bin", home.bin.display()),
        )
        .unwrap();
    });
    h.wait_prompt();
    let repo = h.home.home.join("src/app");
    let launch = repo.clone();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(MenuAction::RunAgent(launch, AgentId::Codex), window, cx)
        })
    });
    h.confirm_agent_launch();
    h.wait_for("the managed Codex to start idle", |v, cx| {
        v.focused_text(cx).contains("fake-codex-ready")
            && v.ws
                .focused_pane()
                .and_then(|p| v.ws.pane(p))
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.status == AgentStatus::Idle)
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let status = move |v: &crate::workspace_view::WorkspaceView| {
        v.ws.pane(pane)
            .and_then(|p| p.agent.as_ref())
            .map(|a| a.status)
    };
    let step = |h: &mut Harness, step: &str, title: &'static str| {
        let control = h.home.home.join(format!("codex-step-{}", pane.raw()));
        // Renamed into place, so the stand-in never reads a half-written step.
        let partial = control.with_extension("partial");
        std::fs::write(&partial, step).unwrap();
        std::fs::rename(&partial, &control).unwrap();
        h.wait_for(step, move |v, _| {
            !control.exists() && v.ws.pane(pane).is_some_and(|p| p.title == title)
        });
    };
    let finished = |h: &Harness| {
        h.read(|v, _| {
            v.notifications
                .newest()
                .iter()
                .filter(|n| n.message.starts_with("Codex finished"))
                .count()
        })
    };

    // The conversation starts its first turn and Codex saves it.
    let day = h.home.home.join(".codex/sessions/2026/10/11");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(
        day.join("rollout-2026-10-11T09-00-00-real-conversation.jsonl"),
        r#"{"type":"session_meta","payload":{"id":"real-conversation","source":"cli"}}"#,
    )
    .unwrap();
    step(&mut h, "work", "⠋ codex | Thinking");
    assert_eq!(h.read(|v, _| status(v)), Some(AgentStatus::Working));

    // Mid-turn, the unsaved title request notifies.
    h.codex_notify(pane, &repo, "title-request");
    h.cx.run_until_parked();
    step(&mut h, "spin", "⠙ codex | Working");
    h.read(|v, _| {
        assert_eq!(status(v), Some(AgentStatus::Working));
        let info = v.ws.pane(pane).unwrap();
        assert!(info.agent_session.is_none(), "{:?}", info.agent_session);
        assert!(
            info.agent_launch
                .as_ref()
                .is_some_and(|c| c.session.is_none()),
            "no unresumable conversation is captured"
        );
    });
    assert_eq!(finished(&h), 0, "the turn has not finished");
    assert!(
        h.cx.debug_bounds(Box::leak(
            format!("session-close-pane-{}", pane.raw()).into_boxed_str()
        ))
        .is_some(),
        "a working agent keeps its close button and checks state on click"
    );
    h.keys("cmd-w");
    assert!(
        h.read(|v, _| v.confirm.is_some() && v.ws.pane(pane).is_some()),
        "cmd-w asks before stopping a working agent"
    );
    h.keys("escape");
    assert!(h.read(|v, _| v.confirm.is_none() && v.ws.pane(pane).is_some()));

    // Whatever leaves the stored status behind, Codex's own title still
    // reports the turn: closing asks, and the next frame restores Working.
    h.cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            v.ws.set_agent_status(pane, "codex", AgentStatus::Idle, 0);
            cx.notify();
        })
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                crate::sidebar_view::SidebarEvent::ClosePane(pane),
                window,
                cx,
            )
        })
    });
    assert!(
        h.read(|v, _| v.confirm.is_some() && v.ws.pane(pane).is_some()),
        "✕ asks while the title reports work"
    );
    h.keys("escape");
    step(&mut h, "spin-again", "⠹ codex | Working");
    assert_eq!(h.read(|v, _| status(v)), Some(AgentStatus::Working));

    // The turn ends once; the conversation's own notify names it.
    step(&mut h, "ready", "codex | Ready");
    assert!(matches!(
        h.read(|v, _| status(v)),
        Some(AgentStatus::Review | AgentStatus::Idle)
    ));
    assert_eq!(finished(&h), 1);
    h.codex_notify(pane, &repo, "real-conversation");
    h.wait_for("the real conversation's identity", move |v, _| {
        v.ws.pane(pane)
            .and_then(|p| p.agent_session.as_ref())
            .is_some_and(|s| s.session == "real-conversation")
    });
    assert_eq!(finished(&h), 1, "the turn is announced once");
    assert!(h.read(|v, _| {
        v.ws.pane(pane)
            .and_then(|p| p.agent_launch.as_ref())
            .and_then(|c| c.session.as_ref())
            .is_some_and(|s| s.0 == "real-conversation")
    }));

    // A conversation's notify can also arrive before its title turns Ready;
    // the turn still ends with the title, once.
    step(&mut h, "work", "⠋ codex | Thinking");
    h.codex_notify(pane, &repo, "real-conversation");
    h.cx.run_until_parked();
    assert_eq!(h.read(|v, _| status(v)), Some(AgentStatus::Working));
    assert_eq!(finished(&h), 1);
    step(&mut h, "ready", "codex | Ready");
    assert_eq!(finished(&h), 2);
}
