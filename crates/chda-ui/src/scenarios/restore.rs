//! Session restore: layout, directories and agent conversations.

use chda_core::agents::HookKind;
use gpui::TestAppContext;

use super::harness::{Harness, wait_until};

#[gpui::test]
fn tabs_splits_and_directories_come_back(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "restore", |home| {
        std::fs::create_dir_all(home.home.join("work/sub")).unwrap();
    });
    h.wait_prompt();
    let sub = h.home.home.join("work/sub");
    h.run(&format!("cd {}", sub.display()), "sub");
    h.wait_for("the pane to report its directory", {
        let sub = sub.clone();
        move |v, _| {
            let pane = v.ws.focused_pane().unwrap();
            v.ws.pane(pane).unwrap().cwd.as_deref() == Some(sub.as_path())
        }
    });
    h.keys("cmd-d");
    h.keys("cmd-t");
    h.cx.run_until_parked();

    let (cx2, view2) = h.reopen();
    cx2.run_until_parked();
    view2.read_with(&cx2, |v, _| {
        assert_eq!(v.ws.tabs().len(), 2);
        let first = &v.ws.tabs()[0];
        assert_eq!(first.panes().len(), 2);
        let cwd = v.ws.pane(first.panes()[0]).unwrap().cwd.clone();
        assert_eq!(cwd.as_deref(), Some(sub.as_path()));
    });
}

/// A Codex stand-in that prints its arguments, found only through a login
/// shell slower than the launch wait (#243).
fn slow_shell_codex(home: &super::harness::Home) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(&home.bin).unwrap();
    let bin = home.bin.join("codex");
    std::fs::write(
        &bin,
        "#!/usr/bin/env sh\nprintf 'fake-codex %s\\n' \"$*\"\nexec /bin/cat >/dev/null\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(home.home.join("gui-path"), "/usr/bin:/bin:/usr/sbin:/sbin").unwrap();
    std::fs::write(
        home.home.join(".zprofile"),
        format!("sleep 3\nexport PATH='{}':$PATH\n", home.bin.display()),
    )
    .unwrap();
}

#[gpui::test]
fn codex_conversations_wait_for_a_slow_login_shell_path_to_reopen(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "restore-slow-path", |home| {
        slow_shell_codex(home);
        home.codex_session(&home.home, "saved-session");
    });
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    let home = h.home.home.clone();
    h.hook_from(
        "codex",
        Some(pane.raw()),
        &home,
        HookKind::SessionStart,
        "saved-session",
    );
    h.wait_for("the pane to know its conversation", move |v, _| {
        v.ws.pane(pane)
            .and_then(|info| info.agent_session.clone())
            .is_some_and(|a| a.session == "saved-session")
    });

    let (mut cx2, view2) = h.reopen();
    wait_until(
        &mut cx2,
        &view2,
        "Codex to resume once the PATH arrives",
        |v, cx| {
            let first = v.ws.tabs()[0].panes()[0];
            v.pane_text(first, cx)
                .contains("fake-codex resume saved-session")
        },
    );
    view2.read_with(&cx2, |v, _| {
        assert!(
            v.notifications
                .latest()
                .is_none_or(|note| !note.contains("not on PATH")),
            "{:?}",
            v.notifications.latest()
        );
    });
}

#[gpui::test]
fn agent_conversations_reopen_in_their_panes(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "restore-agents", |home| {
        home.fake_claude();
        home.claude_session(&home.home, "s1", "fix the login loop");
    });
    h.wait_prompt();
    h.keys("cmd-d");
    h.wait_prompt();
    h.keys("cmd-shift-d");
    h.wait_prompt();
    let panes: [_; 3] = h.read(|v, _| v.ws.tabs()[0].panes()).try_into().unwrap();
    let home = h.home.home.clone();
    // A running conversation, one that ended, and one whose transcript is gone.
    h.hook_session(Some(panes[0].raw()), &home, HookKind::SessionStart, "s1");
    h.hook_session(Some(panes[1].raw()), &home, HookKind::PromptSubmitted, "s2");
    h.hook_session(Some(panes[1].raw()), &home, HookKind::SessionEnd, "s2");
    h.hook_session(Some(panes[2].raw()), &home, HookKind::SessionStart, "gone");
    h.wait_for("the panes to know their conversations", move |v, _| {
        let session = |p| {
            v.ws.pane(p)
                .unwrap()
                .agent_session
                .clone()
                .map(|a| a.session)
        };
        session(panes[0]).as_deref() == Some("s1")
            && session(panes[1]).is_none()
            && session(panes[2]).as_deref() == Some("gone")
    });

    let (mut cx2, view2) = h.reopen();
    wait_until(&mut cx2, &view2, "the conversation to reopen", |v, cx| {
        let first = v.ws.tabs()[0].panes()[0];
        v.pane_text(first, cx).contains("fake-claude")
            && v.pane_text(first, cx).contains("--settings")
    });
    view2.read_with(&cx2, |v, cx| {
        let panes = v.ws.tabs()[0].panes();
        assert_eq!(panes.len(), 3, "same split layout");
        let first = v.pane_text(panes[0], cx);
        assert!(first.contains("--resume s1"), "{first}");
        let conversation = v.ws.pane(panes[0]).unwrap().agent_session.clone();
        assert_eq!(conversation.map(|a| a.session).as_deref(), Some("s1"));
        assert!(v.ws.pane(panes[1]).unwrap().agent_session.is_none());
        assert!(v.ws.pane(panes[2]).unwrap().agent_session.is_none());
        let note = v
            .notifications
            .latest()
            .map(str::to_owned)
            .unwrap_or_default();
        assert!(note.contains("session gone is gone"), "{note}");
    });
    wait_until(&mut cx2, &view2, "shells in the other panes", |v, cx| {
        let panes = v.ws.tabs()[0].panes();
        v.pane_text(panes[1], cx).contains("test%") && v.pane_text(panes[2], cx).contains("test%")
    });

    // A new shell prompt means the agent exited: the pane forgets it.
    h.run("echo agent-exited", "agent-exited");
    h.wait_for("the prompt to clear the conversation", move |v, _| {
        v.ws.pane(panes[2]).unwrap().agent_session.is_none()
    });

    // With restore-agents off, the conversation's pane gets a shell.
    std::fs::write(&h.home.config, "restore-agents = false\n").unwrap();
    let (mut cx3, view3) = h.reopen();
    wait_until(&mut cx3, &view3, "a shell in the first pane", |v, cx| {
        let first = v.ws.tabs()[0].panes()[0];
        v.pane_text(first, cx).contains("test%")
    });
    view3.read_with(&cx3, |v, _| {
        let first = v.ws.tabs()[0].panes()[0];
        assert!(v.ws.pane(first).unwrap().agent_session.is_none());
    });
}

#[gpui::test]
fn full_screen_keeps_the_windowed_size_for_the_next_launch(cx: &mut TestAppContext) {
    use gpui::{px, size};
    let mut h = Harness::open(cx, "restore-full-screen", |_| {});
    h.wait_prompt();
    h.cx.simulate_resize(size(px(900.0), px(600.0)));
    h.cx.run_until_parked();
    let saved = |h: &Harness| {
        let bounds = chda_core::SavedWindow::load(&h.home.data)
            .and_then(|w| w.bounds)
            .expect("saved bounds");
        (bounds.width, bounds.height)
    };
    assert_eq!(saved(&h), (900.0, 600.0));

    // Native full screen resizes the window to the screen.
    h.cx.update(|window, _| window.toggle_fullscreen());
    h.cx.simulate_resize(size(px(1710.0), px(1107.0)));
    h.cx.run_until_parked();
    assert_eq!(
        saved(&h),
        (900.0, 600.0),
        "a full-screen frame is not saved as the windowed size"
    );

    h.cx.update(|window, _| window.toggle_fullscreen());
    h.cx.simulate_resize(size(px(1000.0), px(700.0)));
    h.cx.run_until_parked();
    assert_eq!(
        saved(&h),
        (1000.0, 700.0),
        "leaving full screen records again"
    );
}
