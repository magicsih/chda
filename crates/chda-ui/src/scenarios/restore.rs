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
        let note = v.status_line.clone().unwrap_or_default();
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
