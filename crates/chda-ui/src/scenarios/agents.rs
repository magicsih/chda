//! Agent status per pane: tab dots, the ACTIVE list, the Dock badge,
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
        let active = &v.sidebar.read(cx).model.active_tabs;
        let row = active.iter().find(|t| t.tab == v.ws.tabs()[0].id).unwrap();
        assert_eq!(row.status, AgentStatus::WaitingInput, "ACTIVE list agrees");
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
            v.sidebar
                .read(cx)
                .model
                .idle_agents
                .iter()
                .any(|r| r.pane == pane && r.agent == agent)
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
    h.hook_from(
        "codex",
        Some(pane.raw()),
        &repo,
        HookKind::Stopped,
        "real-session",
    );
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
