//! Navigation identity, clipboard routing and restart context regressions.
use super::harness::{Harness, wait_until};
use crate::sidebar_view::SidebarEvent;
use gpui::{ClipboardItem, Modifiers, TestAppContext};
use std::path::Path;

fn open(h: &mut Harness, path: &Path) {
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::OpenWorktree(path.into()), window, cx)
        })
    });
    h.wait_prompt();
}
fn clipboard(h: &mut Harness, text: &str) {
    h.cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())));
}
fn click(h: &mut Harness, selector: &'static str) {
    h.cx.run_until_parked();
    let at = h.cx.debug_bounds(selector).expect(selector).center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
}

#[gpui::test]
fn worktree_navigation_reuses_recent_panes_and_tracks_exact_tabs(cx: &mut TestAppContext) {
    let mut a = std::path::PathBuf::new();
    let mut b = a.clone();
    let mut h = Harness::open(cx, "navigation", |home| {
        a = home.repo("a");
        b = home.repo("b");
        std::fs::write(
            &home.config,
            format!("repos = [\"{}\", \"{}\"]\n", a.display(), b.display()),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for("both repositories", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .all(|r| !r.worktrees.is_empty())
    });
    open(&mut h, &a);
    let first = h.read(|v, _| v.ws.focused_pane().unwrap());
    open(&mut h, &b);
    let count = h.read(|v, _| v.ws.tabs().len());
    open(&mut h, &a);
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), count);
    assert_eq!(h.read(|v, _| v.ws.focused_pane().unwrap()), first);
    h.keys("cmd-t");
    h.wait_prompt();
    let second = h.read(|v, _| v.ws.focused_pane().unwrap());
    open(&mut h, &b);
    open(&mut h, &a);
    assert_eq!(h.read(|v, _| v.ws.focused_pane().unwrap()), second);
    h.read(|v, cx| {
        let s = v.sidebar.read(cx);
        assert_eq!(s.selected.as_ref(), Some(&a));
        assert_eq!(s.active_tab, v.ws.active_tab().map(|t| t.id));
    });
    click(&mut h, "collapse-all");
    assert!(h.read(|v, cx| v.sidebar.read(cx).model.repos.iter().all(|r| r.collapsed)));
    // Background activity must not reveal the deliberately collapsed target.
    h.run("echo keep-collapsed", "keep-collapsed");
    assert!(h.read(|v, cx| v.sidebar.read(cx).model.repos.iter().all(|r| r.collapsed)));
    open(&mut h, &a);
    assert!(h.read(|v, cx| {
        !v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .find(|r| r.path == a)
            .unwrap()
            .collapsed
    }));
}

#[gpui::test]
fn paste_replaces_unicode_selection_and_creates_the_saved_multiline_note(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "form-paste", |home| {
        repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.open_sheet(repo.clone(), window, cx))
    });
    clipboard(&mut h, "feat/wrong\nname");
    h.keys("cmd-v");
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1, "paste never submits");
    h.keys("cmd-a");
    clipboard(&mut h, "feat/paste");
    h.keys("cmd-v");
    h.keys("tab");
    clipboard(&mut h, "Task 🧭 한글\nSecond line");
    h.cx.update(|window, cx| window.dispatch_action(Box::new(crate::terminal_view::Paste), cx));
    h.keys("cmd-a");
    clipboard(&mut h, "Ship 🧭 한글\nKeep this note");
    h.keys("cmd-v");
    h.keys("home right shift-right");
    clipboard(&mut h, "X");
    h.keys("cmd-v");
    // The selected ASCII character, rather than the whole field, is replaced.
    click(&mut h, "form-submit");
    let note = || {
        std::process::Command::new("git")
            .current_dir(&repo)
            .args(["config", "branch.feat/paste.description"])
            .output()
            .unwrap()
    };
    h.wait_for("the pasted branch", |_, _| note().status.success());
    assert_eq!(
        String::from_utf8(note().stdout).unwrap().trim(),
        "SXip 🧭 한글\nKeep this note"
    );
    let dir = h.read(|v, _| {
        v.ws.pane(v.ws.focused_pane().unwrap())
            .unwrap()
            .cwd
            .clone()
            .unwrap()
    });
    assert!(dir.is_dir());
    let first = h.read(|v, _| v.ws.tabs()[0].panes()[0]);
    assert!(
        !h.read(|v, cx| v.pane_text(first, cx))
            .contains("feat/paste"),
        "paste never reaches the shell behind the form"
    );
}

#[gpui::test]
fn restart_retains_work_time_without_restoring_runtime_liveness(cx: &mut TestAppContext) {
    use crate::terminal_view::TerminalEvent;
    let mut h = Harness::open(cx, "restore-work-time", |_| {});
    h.wait_prompt();
    let before = crate::terminal_view::now_ms() - 3_600_000;
    let terminal = h.focused_terminal();
    h.cx.update(|_, cx| {
        terminal.update(cx, |_, cx| cx.emit(TerminalEvent::Activity(before)));
    });
    let (mut cx2, view2) = h.reopen();
    wait_until(&mut cx2, &view2, "the restored shell", |v, cx| {
        v.focused_text(cx).contains("test%")
    });
    view2.read_with(&cx2, |v, _| {
        let pane = v.ws.pane(v.ws.focused_pane().unwrap()).unwrap();
        assert_eq!(pane.previous_activity, Some(before));
        assert_eq!(
            pane.last_activity, before,
            "shell initialization is not new work"
        );
        assert!(!pane.agent_live);
    });
}

#[gpui::test]
fn multiple_windows_share_navigation_routes_and_restore_each_layout(cx: &mut TestAppContext) {
    use chda_core::{
        SavedSession,
        agents::{HookEvent, HookKind, ipc},
    };
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "multi-window", |home| {
        repo = home.repo("app");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("repository loaded", |v, cx| {
        !v.sidebar.read(cx).model.repos[0].worktrees.is_empty()
    });
    open(&mut h, &repo);
    let env = h.read(|v, _| v.env.clone());
    let handle = h.cx.update(|_, cx| {
        crate::open_workspace_window(chda_config::load(&env.ghostty, None), None, env.clone(), cx)
    });
    let mut second_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let second = handle.root(&mut second_cx).unwrap();
    wait_until(
        &mut second_cx,
        &second,
        "second repository loaded",
        |v, cx| !v.sidebar.read(cx).model.repos[0].worktrees.is_empty(),
    );
    // A deliberate new tab makes another pane in the same worktree.
    second_cx.update(|window, cx| {
        second.update(cx, |v, cx| {
            v.run_menu_action(
                crate::workspace_view::MenuAction::OpenTerminal(repo.clone()),
                window,
                cx,
            );
        })
    });
    wait_until(&mut second_cx, &second, "second shell", |v, cx| {
        v.focused_text(cx).contains("test%")
    });
    let selected = second.read_with(&second_cx, |v, _| v.ws.focused_pane().unwrap());
    let first_count = h.read(|v, _| v.ws.tabs().len());
    open(&mut h, &repo);
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        first_count,
        "navigation must reuse the other window"
    );
    assert_eq!(env.windows.borrow().active, Some(handle.window_id()));
    let event = HookEvent {
        agent: "claude".into(),
        session_id: "conversation-two".into(),
        cwd: repo.clone(),
        kind: HookKind::SessionStart,
        timestamp: crate::terminal_view::now_ms(),
        pane: Some(selected.raw()),
    };
    ipc::send(&ipc::socket_path(&h.home.data), &event).unwrap();
    wait_until(&mut second_cx, &second, "pane-owned event", |v, _| {
        v.ws.pane(selected).unwrap().agent_live
    });
    h.read(|v, _| assert!(!v.ws.pane(v.ws.focused_pane().unwrap()).unwrap().agent_live));
    let saved = SavedSession::load(&h.home.data).unwrap();
    assert_eq!(saved.windows.len(), 2);
    assert_eq!(saved.windows[saved.active_window].tabs.len(), 2);
    assert!(
        serde_json::to_string(&saved.windows[saved.active_window])
            .unwrap()
            .contains("conversation-two")
    );
    // Closing the original window must not tear down another window's receiver.
    h.cx.update(|window, _| window.remove_window());
    let event = HookEvent {
        kind: HookKind::PromptSubmitted,
        ..event
    };
    ipc::send(&ipc::socket_path(&h.home.data), &event).unwrap();
    wait_until(
        &mut second_cx,
        &second,
        "events after the original window closes",
        |v, _| {
            v.ws.pane(selected).unwrap().agent.as_ref().unwrap().status
                == chda_core::AgentStatus::Working
        },
    );
    assert_eq!(SavedSession::load(&h.home.data).unwrap().windows.len(), 1);
}
