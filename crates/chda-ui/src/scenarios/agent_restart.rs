//! A managed child exits without losing its pane or exact conversation.
use super::harness::Harness;
use chda_core::agents::HookKind;
use gpui::TestAppContext;

#[gpui::test]
fn failed_terminal_adoption_never_acknowledges_preparation(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "restart-adoption-failure", |_| {});
    h.wait_prompt();
    let env = h.read(|v, _| v.env.clone());
    env.windows.borrow_mut().update.adopting = true;
    h.keys("cmd-t");
    assert!(env.windows.borrow().update.adoption_failed);
    assert!(
        !h.read(|v, cx| v.adoption_ready(cx)),
        "a missing inherited terminal cannot pass prepared/commit"
    );
    assert_eq!(
        h.read(|v, _| v.panes.len()),
        1,
        "original live shell remains untouched"
    );
    env.windows.borrow_mut().update.adopting = false;
}

#[gpui::test]
fn exited_managed_agent_retains_its_pane_and_restarts_the_exact_conversation(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "agent-restart", |home| {
        home.fake_claude();
        home.claude_session(&home.home, "s1", "fixture");
        std::fs::write(
            &home.claude_bin,
            r#"#!/bin/sh
printf 'launch %s\n' "$*"
printf 'run\n' >> "$HOME/cli-runs"
case " $* " in *' --resume s1 '*) exec cat >/dev/null ;; esac
while [ ! -f "$HOME/release-cli" ]; do sleep 0.02; done
printf 'previous-run-final-output\n'
exit 7
"#,
        )
        .unwrap();
    });
    h.wait_prompt();
    let shell = h.read(|v, _| v.ws.focused_pane().unwrap());
    let shell_pid = h.focused_terminal().read_with(&h.cx, |t, _| t.child_pid());
    h.cx.dispatch_action(crate::workspace_view::RunClaude);
    h.cx.run_until_parked();
    let choice = h.cx.debug_bounds("launch-policy-bypass").unwrap();
    h.cx.simulate_click(choice.center(), gpui::Modifiers::default());
    h.keys("enter");
    h.wait_for("managed CLI", |v, cx| {
        v.focused_text(cx).contains("launch --settings")
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let cwd = h.home.home.clone();
    h.hook_session(Some(pane.raw()), &cwd, HookKind::SessionStart, "s1");
    h.wait_for("captured session", move |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_launch
            .as_ref()
            .is_some_and(|c| c.session.as_ref().is_some_and(|s| s.0 == "s1"))
    });
    std::fs::write(cwd.join("release-cli"), "").unwrap();
    h.wait_for("the terminal to observe process exit", move |v, _| {
        !v.panes.contains_key(&pane)
    });
    assert!(
        h.read(|v, _| v.ws.pane(pane).is_some()),
        "managed exit retains the original pane"
    );
    assert!(
        h.cx.debug_bounds("agent-restart").is_some(),
        "exact conversation can restart"
    );
    assert_eq!(
        h.read(|v, cx| v.panes[&shell].0.read(cx).child_pid()),
        shell_pid,
        "other shells survive"
    );
    let bounds = h.cx.debug_bounds("previous-run-toggle").unwrap();
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.cx.run_until_parked();
    let copy = h.cx.debug_bounds("previous-copy").unwrap();
    h.cx.simulate_click(copy.center(), gpui::Modifiers::default());
    h.cx.update(|_, cx| {
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("previous-run-final-output")
        )
    });
    let (cx2, view2) = h.reopen();
    cx2.run_until_parked();
    view2.read_with(&cx2, |v, _| {
        let pane = v.ws.focused_pane().unwrap();
        assert!(
            v.ws.pane(pane)
                .unwrap()
                .managed_run
                .as_ref()
                .unwrap()
                .stopped()
        );
        assert!(
            !v.panes.contains_key(&pane),
            "cold restore does not restart an exited agent"
        );
        assert!(v.ws.pane(pane).unwrap().previous_run.is_some());
    });
    let transcript = h.home.claude_projects.join("project/s1.jsonl");
    let bytes = std::fs::read(&transcript).unwrap();
    std::fs::remove_file(&transcript).unwrap();
    let bounds = h.cx.debug_bounds("agent-restart").unwrap();
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v
            .ws
            .pane(pane)
            .unwrap()
            .managed_run
            .as_ref()
            .unwrap()
            .label())
            .contains("transcript is missing")
    );
    assert!(!h.read(|v, _| v.panes.contains_key(&pane)));
    std::fs::write(&transcript, bytes).unwrap();
    let executable = h.home.claude_bin.clone();
    let backup = executable.with_extension("saved");
    std::fs::rename(&executable, &backup).unwrap();
    let bounds = h.cx.debug_bounds("agent-restart").unwrap();
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v
            .ws
            .pane(pane)
            .unwrap()
            .managed_run
            .as_ref()
            .unwrap()
            .label())
            .contains("executable is unavailable")
    );
    std::fs::rename(backup, executable).unwrap();
    h.view.update(&mut h.cx, |v, _| {
        v.ws.pane_mut(pane)
            .unwrap()
            .agent_launch
            .as_mut()
            .unwrap()
            .cwd = cwd.join("missing-directory")
    });
    let bounds = h.cx.debug_bounds("agent-restart").unwrap();
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v
            .ws
            .pane(pane)
            .unwrap()
            .managed_run
            .as_ref()
            .unwrap()
            .label())
            .contains("directory no longer exists")
    );
    h.view.update(&mut h.cx, |v, _| {
        v.ws.pane_mut(pane)
            .unwrap()
            .agent_launch
            .as_mut()
            .unwrap()
            .cwd = cwd.clone()
    });
    let bounds = h.cx.debug_bounds("agent-restart").unwrap();
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    h.wait_for("same pane exact restart", move |v, cx| {
        v.panes.contains_key(&pane)
            && v.pane_text(pane, cx)
                .replace('\n', "")
                .contains("--resume s1")
    });
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        2,
        "no extra tab on restart"
    );
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(pane));
    assert!(
        h.read(|v, cx| v.pane_text(pane, cx).replace('\n', ""))
            .contains("--dangerously-skip-permissions")
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("cli-runs"))
            .unwrap()
            .lines()
            .count(),
        2,
        "repeated clicks spawn only one successor"
    );
    h.hook_session(Some(pane.raw()), &cwd, HookKind::Stopped, "s1");
    h.wait_for("a completed turn remains live", move |v, _| {
        v.ws.pane(pane).unwrap().agent_live
    });
    assert!(
        h.cx.debug_bounds("agent-restart").is_none(),
        "turn completion is not process exit"
    );
}

#[gpui::test]
fn no_session_identity_disables_restart_and_restore_does_not_create_a_conversation(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "restart-no-id", |home| {
        home.fake_claude();
        std::fs::write(
            &home.claude_bin,
            "#!/bin/sh\nprintf 'unknown conversation\\n'\nexit 0\n",
        )
        .unwrap();
    });
    h.wait_prompt();
    h.cx.dispatch_action(crate::workspace_view::RunClaude);
    h.keys("enter");
    h.wait_for("exit without a conversation ID", |v, _| {
        v.ws.tabs().len() == 2
            && v.ws
                .focused_pane()
                .is_some_and(|pane| !v.panes.contains_key(&pane))
    });
    assert!(h.cx.debug_bounds("agent-restart").is_none());
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
    let owned = h.read(|v, _| {
        v.ws.pane(v.ws.focused_pane().unwrap())
            .unwrap()
            .previous_run
            .clone()
            .unwrap()
    });
    let owned_path = h.home.data.join("agent-output").join(owned);
    let unrelated = h.home.data.join("agent-output/keep-user-file.txt");
    std::fs::write(&unrelated, "unrelated").unwrap();
    let (cx2, view2) = h.reopen();
    cx2.run_until_parked();
    view2.read_with(&cx2, |v, _| {
        let pane = v.ws.focused_pane().unwrap();
        assert!(
            v.ws.pane(pane)
                .unwrap()
                .managed_run
                .as_ref()
                .unwrap()
                .stopped()
        );
        assert!(!v.panes.contains_key(&pane));
    });
    h.keys("cmd-w");
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
    assert!(
        !owned_path.exists(),
        "closing the pane prunes its own output after saving the app manifest"
    );
    assert!(unrelated.exists(), "unrelated data files are untouched");
}
