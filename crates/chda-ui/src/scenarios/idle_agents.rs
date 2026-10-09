//! Live idle sessions stay distinct from shells and saved conversations (#112).
use super::harness::Harness;
use chda_core::agents::HookKind;
use chda_core::{AgentSessionRef, AgentStatus, PaneId, SessionKey, SessionRow};
use gpui::{Modifiers, TestAppContext};
use std::process::Command;

fn click(h: &mut Harness, selector: &str) {
    h.cx.run_until_parked();
    let at =
        h.cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
            .expect(selector)
            .center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
}
/// Live idle agent rows, in Sessions order.
fn idle_rows(v: &crate::workspace_view::WorkspaceView, cx: &gpui::App) -> Vec<SessionRow> {
    v.sidebar
        .read(cx)
        .model
        .sessions
        .iter()
        .filter(|r| r.live_idle())
        .cloned()
        .collect()
}
fn rows(h: &Harness) -> Vec<PaneId> {
    h.read(|v, cx| {
        idle_rows(v, cx)
            .iter()
            .filter_map(|r| match r.key {
                SessionKey::Pane(pane) => Some(pane),
                SessionKey::Tab(_) => None,
            })
            .collect()
    })
}
fn pane(h: &Harness) -> PaneId {
    h.read(|v, _| v.ws.focused_pane().unwrap())
}
fn emit(h: &mut Harness, target: PaneId, event: HookKind) {
    h.hook_session(Some(target.raw()), &h.home.home, event, "live");
}

#[gpui::test]
fn idle_rows_follow_live_panes_and_preserve_mixed_state_attention(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "idle-mixed", |_| {});
    h.wait_prompt();
    let first = pane(&h);
    h.keys("cmd-d");
    h.wait_prompt();
    let working = pane(&h);
    h.keys("cmd-shift-d");
    h.wait_prompt();
    let waiting = pane(&h);
    h.keys("cmd-t");
    h.wait_prompt();
    let plain = pane(&h);
    assert!(rows(&h).is_empty());
    emit(&mut h, first, HookKind::SessionStart);
    emit(&mut h, working, HookKind::PromptSubmitted);
    emit(&mut h, waiting, HookKind::WaitingInput);
    h.wait_for("one live idle and two active agents", |v, cx| {
        idle_rows(v, cx).len() == 1
            && v.ws
                .pane(working)
                .unwrap()
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Working)
            && v.ws.unseen_waiting() == 1
    });
    assert_eq!(rows(&h), vec![first]);
    h.view.update(&mut h.cx, |v, cx| {
        v.config.sidebar_width = 180;
        cx.notify();
    });
    h.cx.run_until_parked();
    let name =
        h.cx.debug_bounds(Box::leak(
            format!("session-label-pane-{}", first.raw()).into_boxed_str(),
        ))
        .unwrap();
    assert!(
        f32::from(name.size.width) > 40.0,
        "idle name remains readable at minimum width: {name:?}"
    );
    // The status icon and the label share one line.
    let bounds = |h: &mut Harness, kind: &str| {
        h.cx.debug_bounds(Box::leak(
            format!("session-{kind}-pane-{}", first.raw()).into_boxed_str(),
        ))
        .unwrap()
    };
    let (dot, label) = (bounds(&mut h, "status"), bounds(&mut h, "label"));
    assert!(
        dot.top() < label.bottom() && label.top() < dot.bottom() && dot.right() <= label.left(),
        "dot {dot:?} sits left of the name {label:?}"
    );
    h.read(|v, cx| {
        let r = &idle_rows(v, cx)[0];
        assert_eq!(r.agent.as_deref(), Some("claude"));
        assert_eq!(r.pane_index, 1);
        assert!(r.shares_tab, "three agents share the split tab");
        assert_eq!(
            r.since, 0,
            "startup alone does not timestamp idle readiness"
        );
        assert!(!r.tab_title.is_empty());
        assert!(!r.location().is_empty());
    });
    // SessionStart on compaction of the same conversation keeps its work.
    emit(&mut h, working, HookKind::SessionStart);
    h.hook_session(
        Some(waiting.raw()),
        &h.home.home,
        HookKind::WaitingInput,
        "barrier",
    );
    h.wait_for("compaction event consumed", |v, _| {
        v.ws.pane(waiting)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "barrier")
    });
    assert!(h.read(|v, _| {
        v.ws.pane(working)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == AgentStatus::Working)
    }));
    // Clicking the idle row must choose its pane rather than the tab's
    // higher-priority working/waiting pane, and preserve their attention.
    click(&mut h, &format!("session-pane-{}", first.raw()));
    assert_eq!(pane(&h), first);
    assert_eq!(h.system.0.borrow().badge, 1);
    emit(&mut h, first, HookKind::PromptSubmitted);
    h.wait_for("the idle pane starts work", |v, cx| {
        idle_rows(v, cx).is_empty()
    });
    h.keys("cmd-2");
    emit(&mut h, first, HookKind::Stopped);
    h.wait_for("unread completion", |v, _| {
        v.ws.pane(first)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == AgentStatus::Review)
    });
    h.hook_session(Some(first.raw()), &h.home.home, HookKind::Idle, "idle-note");
    h.wait_for("the delayed idle hook is recorded", |v, _| {
        v.ws.pane(first)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "idle-note")
    });
    assert!(
        rows(&h).is_empty(),
        "idle notification does not dismiss unread completion"
    );
    h.keys("cmd-1");
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                crate::sidebar_view::SidebarEvent::FocusPane(first),
                window,
                cx,
            )
        })
    });
    assert_eq!(rows(&h), vec![first]);
    let since = h.read(|v, cx| idle_rows(v, cx)[0].since);
    assert!(since > 0);
    emit(&mut h, working, HookKind::Stopped);
    h.wait_for("second unread completion", |v, _| {
        v.ws.pane(working)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == AgentStatus::Review)
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                crate::sidebar_view::SidebarEvent::FocusPane(working),
                window,
                cx,
            )
        })
    });
    assert_eq!(
        rows(&h),
        vec![first, working],
        "tab/pane order, not completion order"
    );
    for _ in 0..2 {
        emit(&mut h, first, HookKind::Idle);
    }
    h.wait_for("no duplicates", |v, cx| idle_rows(v, cx).len() == 2);
    assert_eq!(h.read(|v, cx| idle_rows(v, cx)[0].since), since);
    emit(&mut h, first, HookKind::SessionEnd);
    h.wait_for("ended agent removed", |v, cx| idle_rows(v, cx).len() == 1);
    assert_eq!(rows(&h), vec![working]);
    assert!(h.read(|v, _| v.ws.pane(first).unwrap().agent.is_none()));
    assert!(h.read(|v, _| v.ws.pane(plain).unwrap().agent.is_none()));
}

#[gpui::test]
fn idle_close_targets_only_its_pane_and_tab_close_still_confirms(cx: &mut TestAppContext) {
    use crate::terminal_view::TerminalEvent;
    use std::cell::Cell;
    use std::rc::Rc;

    let mut h = Harness::open(cx, "idle-close", |_| {});
    h.wait_prompt();
    let idle = pane(&h);
    let prompt_shown = Rc::new(Cell::new(false));
    let terminal = h.focused_terminal();
    let prompt_subscription = h.cx.update(|_, cx| {
        let shown = prompt_shown.clone();
        cx.subscribe(&terminal, move |_, event, _| {
            if matches!(event, TerminalEvent::Prompt) {
                shown.set(true);
            }
        })
    });
    let path = h.home.home.join("idle.pid");
    h.run(
        &format!("echo $$ > '{}'; echo pid-saved", path.display()),
        "pid-saved",
    );
    // The typed command contains the marker too. Wait for its output and
    // the new prompt event before simulating an agent in this shell;
    // otherwise that late prompt can end the simulated session.
    h.wait_for("the PID command to finish", |v, cx| {
        prompt_shown.get()
            && v.pane_text(idle, cx)
                .lines()
                .any(|l| l.trim() == "pid-saved")
    });
    drop(prompt_subscription);
    drop(terminal);
    let pid: u32 = std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    h.keys("cmd-d");
    h.wait_prompt();
    let working = pane(&h);
    emit(&mut h, idle, HookKind::SessionStart);
    emit(&mut h, working, HookKind::PromptSubmitted);
    h.wait_for("idle close row", |v, cx| {
        idle_rows(v, cx).len() == 1
            && v.ws
                .pane(working)
                .unwrap()
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::Working)
    });
    // The tab-wide control retains #108's active-agent confirmation.
    click(&mut h, "tab-close-0");
    assert!(h.read(|v, _| v.confirm.is_some()));
    h.keys("escape");
    click(&mut h, &format!("session-close-pane-{}", idle.raw()));
    assert_eq!(pane(&h), working, "close does not click the parent row");
    h.wait_for("only the idle shell is terminated", |v, cx| {
        v.ws.pane(idle).is_none()
            && v.ws.pane(working).is_some()
            && idle_rows(v, cx).is_empty()
            && !Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .unwrap()
                .status
                .success()
    });
    assert!(h.read(|v, _| v.confirm.is_none()));
    // Late events for the removed ID cannot claim a replacement shell.
    emit(&mut h, idle, HookKind::Idle);
    h.keys("cmd-t");
    h.wait_prompt();
    assert!(rows(&h).is_empty());
}

#[gpui::test]
fn saved_conversations_output_age_and_shell_prompts_do_not_create_live_idle(
    cx: &mut TestAppContext,
) {
    use crate::terminal_view::TerminalEvent;
    let mut h = Harness::open(cx, "idle-history", |home| {
        std::fs::write(&home.config, "restore-agents = false\n").unwrap();
    });
    h.wait_prompt();
    let target = pane(&h);
    assert!(rows(&h).is_empty());
    h.cx.update(|_, cx| {
        let terminal = h.view.read(cx).panes[&target].0.clone();
        terminal.update(cx, |_, cx| cx.emit(TerminalEvent::Activity(1)));
    });
    assert!(
        rows(&h).is_empty(),
        "old terminal output is not idle status"
    );
    // Worktree-only reports may show attention but cannot establish which
    // pane holds a live process.
    h.hook(None, &h.home.home, HookKind::Stopped);
    h.wait_for("inferred completion", |v, _| {
        v.ws.pane(target).unwrap().agent.is_some()
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                crate::sidebar_view::SidebarEvent::FocusPane(target),
                window,
                cx,
            )
        })
    });
    assert!(rows(&h).is_empty());
    emit(&mut h, target, HookKind::PromptSubmitted);
    emit(&mut h, target, HookKind::Stopped);
    h.wait_for("completion awaiting review", |v, _| {
        v.ws.pane(target)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == AgentStatus::Review)
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                crate::sidebar_view::SidebarEvent::FocusPane(target),
                window,
                cx,
            );
        })
    });
    assert_eq!(rows(&h), vec![target]);
    h.cx.update(|_, cx| {
        let terminal = h.view.read(cx).panes[&target].0.clone();
        terminal.update(cx, |_, cx| cx.emit(TerminalEvent::Prompt));
    });
    assert!(rows(&h).is_empty());
    assert!(h.read(|v, _| v.ws.pane(target).unwrap().agent.is_none()));
    h.view.update(&mut h.cx, |v, cx| {
        v.ws.set_agent_session(
            target,
            Some(AgentSessionRef {
                agent: "claude".into(),
                session: "historical".into(),
            }),
        );
        let terminal = v.panes[&target].0.clone();
        terminal.update(cx, |_, cx| cx.emit(TerminalEvent::Activity(1)));
    });
    let (mut cx2, view2) = h.reopen();
    super::harness::wait_until(&mut cx2, &view2, "restored shell", |v, cx| {
        v.sidebar.read(cx).sessions_loaded
    });
    assert!(view2.read_with(&cx2, |v, cx| { idle_rows(v, cx).is_empty() }));
}

#[gpui::test]
fn idle_sessions_in_one_worktree_keep_context_and_ignore_replaced_session_end(
    cx: &mut TestAppContext,
) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "idle-replace", |home| {
        repo = home.repo("app");
        std::fs::write(
            &home.config,
            format!("repos = [{}]\n", serde_json::to_string(&repo).unwrap()),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.run(&format!("cd '{}'", repo.display()), "test%");
    h.wait_for("repository cwd", |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .and_then(|i| i.cwd.as_ref())
            == Some(&repo)
    });
    let first = pane(&h);
    h.keys("cmd-d");
    h.wait_prompt();
    let second = pane(&h);
    h.hook_from(
        "claude",
        Some(first.raw()),
        &repo,
        HookKind::SessionStart,
        "old",
    );
    h.hook_from(
        "claude",
        Some(first.raw()),
        &repo,
        HookKind::SessionStart,
        "replacement",
    );
    h.hook_from(
        "codex",
        Some(second.raw()),
        &repo,
        HookKind::Idle,
        "codex-live",
    );
    h.wait_for("two live sessions in one worktree", |v, cx| {
        idle_rows(v, cx).len() == 2
    });
    h.hook_from(
        "claude",
        Some(first.raw()),
        &repo,
        HookKind::SessionEnd,
        "old",
    );
    // A following hook acknowledges that the old end event has been consumed.
    h.hook_from(
        "codex",
        Some(second.raw()),
        &repo,
        HookKind::Idle,
        "barrier",
    );
    h.wait_for("old end event consumed", |v, _| {
        v.ws.pane(second)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "barrier")
    });
    h.read(|v, cx| {
        let rows = idle_rows(v, cx);
        assert_eq!(
            rows.iter().map(|r| r.key).collect::<Vec<_>>(),
            vec![SessionKey::Pane(first), SessionKey::Pane(second)]
        );
        assert!(rows.iter().all(|r| r.location() == "app / main"));
        assert_eq!(rows[0].pane_index, 1);
        assert_eq!(rows[1].pane_index, 2);
        assert_eq!(
            v.ws.pane(first)
                .unwrap()
                .agent_session
                .as_ref()
                .unwrap()
                .session,
            "replacement"
        );
    });
    h.hook_from(
        "claude",
        Some(first.raw()),
        &repo,
        HookKind::SessionEnd,
        "replacement",
    );
    h.wait_for("replacement really ends", |v, cx| {
        idle_rows(v, cx).len() == 1
    });
    assert_eq!(rows(&h), vec![second]);
}
