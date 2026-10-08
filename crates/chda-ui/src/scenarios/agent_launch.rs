//! Permission selection is explicit for a managed launch.
use super::harness::Harness;
use chda_core::agents::{HookKind, PermissionPolicy};
use gpui::TestAppContext;

#[gpui::test]
fn supported_launches_show_the_inherited_policy_before_starting(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "launch-policy", |home| home.fake_claude());
    h.wait_prompt();
    h.cx.dispatch_action(crate::workspace_view::RunClaude);
    h.cx.run_until_parked();
    assert!(
        h.cx.debug_bounds("launch-policy-inherit").is_some(),
        "supported launch has an inherited-policy selector"
    );
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        1,
        "no process starts before the launch choice"
    );
    h.keys("enter");
    h.wait_for("inherited Claude command", |v, cx| {
        v.focused_text(cx).contains("fake-claude")
    });
    assert!(
        !h.read(|v, cx| v.focused_text(cx))
            .contains("--dangerously-skip-permissions")
    );
}

#[gpui::test]
fn explicit_policy_and_options_survive_resume_and_restore(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "policy-resume", |home| {
        home.fake_claude();
        home.claude_session(&home.home, "exact-conversation", "fixture");
        std::fs::write(&home.config, "[[agent-presets]]\nname = 'Saved'\nagent = 'claude'\nargs = ['--model', 'captured-model']\n").unwrap();
    });
    h.wait_prompt();
    let cwd = h.home.home.clone();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::RunPreset(cwd.clone(), "Saved".into()),
                window,
                cx,
            )
        })
    });
    h.cx.run_until_parked();
    let bounds = h.cx.debug_bounds("launch-policy-bypass").unwrap();
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.confirm_agent_launch();
    h.wait_for("explicit bypass with model", |v, cx| {
        let text = v.focused_text(cx).replace('\n', "");
        text.contains("--model captured-model") && text.contains("--dangerously-skip-permissions")
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.hook_session(
        Some(pane.raw()),
        &cwd,
        HookKind::SessionStart,
        "exact-conversation",
    );
    h.wait_for("captured exact session", move |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_launch
            .as_ref()
            .is_some_and(|c| {
                c.session
                    .as_ref()
                    .is_some_and(|s| s.0 == "exact-conversation")
            })
    });
    std::fs::write(&h.home.config, "[[agent-presets]]\nname = 'Saved'\nagent = 'claude'\nargs = ['--model', 'different-model']\n").unwrap();
    let (mut cx2, view2) = h.reopen();
    super::harness::wait_until(&mut cx2, &view2, "restored captured command", |v, cx| {
        v.focused_text(cx)
            .replace('\n', "")
            .contains("--resume exact-conversation")
    });
    view2.read_with(&cx2, |v, cx| {
        let text = v.focused_text(cx).replace('\n', "");
        assert!(text.contains("--model captured-model"), "{text}");
        assert!(text.contains("--dangerously-skip-permissions"), "{text}");
        assert!(!text.contains("different-model"));
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.resume_sessions(
                vec![crate::sidebar_view::SessionPick {
                    worktree: cwd.clone(),
                    agent: "claude".into(),
                    session: "exact-conversation".into(),
                }],
                window,
                cx,
            )
        })
    });
    h.cx.run_until_parked();
    assert!(
        h.cx.debug_bounds("launch-policy-bypass").is_none(),
        "captured choices are read-only"
    );
    h.confirm_agent_launch();
    h.wait_for("exact resume with captured options", |v, cx| {
        v.focused_text(cx)
            .replace('\n', "")
            .contains("--resume exact-conversation")
    });
    h.read(|v, cx| {
        assert!(
            v.focused_text(cx)
                .replace('\n', "")
                .contains("--model captured-model")
        );
        assert_eq!(
            v.ws.pane(v.ws.focused_pane().unwrap())
                .unwrap()
                .agent_launch
                .as_ref()
                .unwrap()
                .policy,
            PermissionPolicy::Bypass
        );
    });
}

#[gpui::test]
fn conflicting_preset_keeps_the_sheet_and_does_not_start(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "policy-conflict", |home| {
        home.fake_claude();
        std::fs::write(&home.config,"[[agent-presets]]\nname = 'Plan'\nagent = 'claude'\nargs = ['--permission-mode', 'plan']\n").unwrap();
    });
    h.wait_prompt();
    let cwd = h.home.home.clone();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::RunPreset(cwd, "Plan".into()),
                window,
                cx,
            )
        })
    });
    h.cx.run_until_parked();
    let bounds = h.cx.debug_bounds("launch-policy-bypass").unwrap();
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.confirm_agent_launch();
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
    assert!(h.cx.debug_bounds("launch-start").is_some());
    h.keys("escape");
    h.run("echo cancelled-choice", "cancelled-choice");
}
