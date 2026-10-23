//! The same reopen handler used by Dock and Finder, without OS input.
use super::harness::{Harness, wait_until};
use gpui::{TestAppContext, VisualTestContext};

#[gpui::test]
fn reopen_after_the_last_window_closes_opens_one_usable_window(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_quit_mode(gpui::QuitMode::Explicit));
    let mut h = Harness::open(cx, "dock-reopen-local", |_| {});
    h.wait_prompt();
    let env = h.read(|v, _| v.env.clone());
    h.cx.update(|window, _| window.remove_window());
    h.cx.run_until_parked();
    assert!(env.windows.borrow().entries.is_empty());

    cx.update(|cx| crate::reopen_workspace(env.clone(), cx));
    assert_eq!(env.windows.borrow().entries.len(), 1);
    let entry = env.windows.borrow().entries[0].clone();
    let view = entry.view.upgrade().unwrap();
    let mut reopened = VisualTestContext::from_window(entry.window, cx);
    wait_until(&mut reopened, &view, "reopened shell", |v, cx| {
        v.focused_prompt_ready(cx)
    });
    reopened.simulate_input("printf '\\ndock-reopened-marker\\n'");
    reopened.simulate_keystrokes("enter");
    wait_until(&mut reopened, &view, "reopened input", |v, cx| {
        v.focused_text(cx).contains("\ndock-reopened-marker\n")
    });

    // An existing window and its unfinished palette input survive repeat opens.
    reopened.simulate_keystrokes(crate::platform::shortcut("cmd-shift-p"));
    reopened.simulate_input("New tab");
    for _ in 0..2 {
        cx.update(|cx| crate::reopen_workspace(env.clone(), cx));
    }
    assert_eq!(env.windows.borrow().entries.len(), 1);
    assert_eq!(env.windows.borrow().entries[0].window, entry.window);
    reopened.update(|window, cx| {
        let (palette, _) = view.read(cx).palette.as_ref().unwrap();
        assert!(palette.read(cx).input_focus(cx).is_focused(window));
    });
    reopened.simulate_keystrokes(crate::platform::shortcut("escape"));
    reopened.simulate_input("printf '\\nstill-usable-marker\\n'");
    reopened.simulate_keystrokes("enter");
    wait_until(
        &mut reopened,
        &view,
        "input after repeat reopen",
        |v, cx| v.focused_text(cx).contains("\nstill-usable-marker\n"),
    );
}

#[gpui::test]
fn reopen_restores_the_last_detached_window_without_restarting_its_shells(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_quit_mode(gpui::QuitMode::Explicit));
    let mut h = Harness::open_service(cx, "dock-reopen-service", |_| {});
    h.wait_prompt();
    let env = h.read(|v, _| v.env.clone());
    // Keep a different closed window available in Sessions.
    h.keys("cmd-shift-n");
    let other = env.windows.borrow().entries.last().unwrap().clone();
    let mut other_cx = VisualTestContext::from_window(other.window, cx);
    let other_view = other.view.upgrade().unwrap();
    wait_until(&mut other_cx, &other_view, "other shell", |v, cx| {
        v.focused_prompt_ready(cx)
    });
    other_cx.update(|window, _| window.remove_window());
    other_cx.run_until_parked();
    h.keys("cmd-d");
    h.wait_prompt();
    h.run(
        "REOPEN_VALUE=same-shell; printf '\\nbefore-reopen\\n'",
        "\nbefore-reopen\n",
    );
    let (focused, pids) = h.read(|v, cx| {
        (
            v.ws.focused_pane().unwrap(),
            v.panes
                .iter()
                .map(|(p, (t, _))| (*p, t.read(cx).child_pid()))
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(pids.len(), 2);
    h.cx.update(|window, _| window.remove_window());
    h.cx.run_until_parked();
    assert!(env.windows.borrow().entries.is_empty());
    assert_eq!(env.windows.borrow().detached.len(), 2);
    let client = env.session_service.as_ref().unwrap();
    let sessions = client.state().unwrap().sessions.len();

    for _ in 0..2 {
        cx.update(|cx| crate::reopen_workspace(env.clone(), cx));
        assert_eq!(env.windows.borrow().entries.len(), 1);
        assert_eq!(env.windows.borrow().detached.len(), 1);
        let entry = env.windows.borrow().entries[0].clone();
        let view = entry.view.upgrade().unwrap();
        let mut reopened = VisualTestContext::from_window(entry.window, cx);
        wait_until(
            &mut reopened,
            &view,
            "same split shells and output",
            |v, cx| {
                v.ws.focused_pane() == Some(focused)
                    && v.panes.len() == pids.len()
                    && pids.iter().all(|(p, pid)| {
                        v.panes
                            .get(p)
                            .is_some_and(|(t, _)| t.read(cx).child_pid() == *pid)
                    })
                    && v.focused_text(cx).contains("\nbefore-reopen\n")
            },
        );
        reopened.simulate_input("printf '\\nafter-reopen-%s\\n' \"$REOPEN_VALUE\"");
        reopened.simulate_keystrokes("enter");
        wait_until(
            &mut reopened,
            &view,
            "same shell variable and input",
            |v, cx| v.focused_text(cx).contains("\nafter-reopen-same-shell\n"),
        );
        assert_eq!(client.state().unwrap().sessions.len(), sessions);
        reopened.update(|window, _| window.remove_window());
        reopened.run_until_parked();
    }
    assert_eq!(env.windows.borrow().detached.len(), 2);
}

#[gpui::test]
fn reopen_uses_the_active_window_and_waits_during_quit_or_update(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_quit_mode(gpui::QuitMode::Explicit));
    let mut h = Harness::open(cx, "dock-reopen-active", |_| {});
    h.wait_prompt();
    let env = h.read(|v, _| v.env.clone());
    let original = env.windows.borrow().entries[0].window;
    h.keys("cmd-shift-n");
    env.windows.borrow_mut().active = Some(original.window_id());
    cx.update(|cx| crate::reopen_workspace(env.clone(), cx));
    assert_eq!(env.windows.borrow().entries.len(), 2);
    assert_eq!(env.windows.borrow().active, Some(original.window_id()));
    cx.update(|cx| assert_eq!(cx.active_window(), Some(original)));
    let entries = env.windows.borrow().entries.clone();
    for entry in entries {
        cx.update(|cx| {
            entry
                .window
                .update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        cx.run_until_parked();
    }
    env.windows.borrow_mut().update.frozen = true;
    cx.update(|cx| crate::reopen_workspace(env.clone(), cx));
    assert!(env.windows.borrow().entries.is_empty());
    env.windows.borrow_mut().update.frozen = false;
    env.windows.borrow_mut().quitting = true;
    cx.update(|cx| crate::reopen_workspace(env.clone(), cx));
    assert!(env.windows.borrow().entries.is_empty());
}
