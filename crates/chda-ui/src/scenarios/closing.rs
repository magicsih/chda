//! Tab-wide shutdown, pane shortcuts and cancellable agent protection (#108).
use super::harness::Harness;
use chda_core::agents::HookKind;
use chda_core::{AgentStatus, PaneId, SavedWindow};
use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext};
use std::process::Command;

fn click(h: &mut Harness, selector: &'static str, count: usize) {
    h.cx.run_until_parked();
    let at = h.cx.debug_bounds(selector).expect(selector).center();
    h.cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: count,
        first_mouse: false,
    });
    h.cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: count,
    });
    h.cx.run_until_parked();
}

fn shell_pid(h: &mut Harness, name: &str) -> u32 {
    h.wait_prompt();
    let path = h.home.home.join(format!("{name}.pid"));
    h.type_text(&format!("echo $$ > '{}'", path.display()));
    h.keys("enter");
    h.wait_for("shell pid", |_, _| path.exists());
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .unwrap()
        .status
        .success()
}

fn focused(h: &Harness) -> PaneId {
    h.read(|view, _| view.ws.focused_pane().unwrap())
}

#[gpui::test]
fn tab_close_confirms_inactive_working_splits_and_preserves_cancelled_processes(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx, "close-working", |_| {});
    let working = focused(&h);
    let target = h.read(|view, _| view.ws.active_tab().unwrap().id);
    let first_pid = shell_pid(&mut h, "first");
    h.keys("cmd-d");
    let waiting = focused(&h);
    let second_pid = shell_pid(&mut h, "second");
    h.keys("cmd-shift-d");
    let third_pid = shell_pid(&mut h, "third");
    h.keys("cmd-t");
    let original = focused(&h);
    let survivor_pid = shell_pid(&mut h, "survivor");
    h.view.update(&mut h.cx, |view, cx| {
        view.ws.rename_tab(target, "Background build");
        cx.notify();
    });
    h.hook_session(
        Some(working.raw()),
        &h.home.home,
        HookKind::PromptSubmitted,
        "build-session",
    );
    h.hook_session(
        Some(waiting.raw()),
        &h.home.home,
        HookKind::WaitingInput,
        "permission-session",
    );
    h.wait_for("working and waiting hooks", |view, _| {
        view.ws
            .pane(working)
            .unwrap()
            .agent
            .as_ref()
            .is_some_and(|a| a.status == AgentStatus::Working)
            && view
                .ws
                .pane(waiting)
                .unwrap()
                .agent
                .as_ref()
                .is_some_and(|a| a.status == AgentStatus::WaitingInput)
    });
    assert_eq!(h.system.0.borrow().badge, 1);
    // A double click on the close button must not select or rename its parent tab.
    click(&mut h, "tab-close-0", 2);
    assert_eq!(focused(&h), original);
    h.read(|view, _| {
        let confirm = view.confirm.as_ref().unwrap();
        assert_eq!(confirm.lines.len(), 2);
        assert!(
            confirm
                .lines
                .iter()
                .any(|line| line.contains("working") && line.contains("build-session"))
        );
        assert!(
            confirm
                .lines
                .iter()
                .any(|line| line.contains("waiting for input")
                    && line.contains("permission-session"))
        );
        assert_eq!(view.ws.tab_title(&view.ws.tabs()[0]), "Background build");
    });
    // Even if another shortcut changes selection, cancellation restores the captured focus.
    h.keys("cmd-1");
    click(&mut h, "confirm-cancel", 1);
    assert_eq!(focused(&h), original);
    assert!(h.read(|view, _| view.confirm.is_none()));
    let focus_file = h.home.home.join("cancel-focus.pid");
    h.type_text(&format!("echo $$ > '{}'", focus_file.display()));
    h.keys("enter");
    h.wait_for("cancel restored keyboard focus", |_, _| focus_file.exists());
    assert_eq!(
        std::fs::read_to_string(focus_file).unwrap().trim(),
        survivor_pid.to_string()
    );
    for pid in [first_pid, second_pid, third_pid, survivor_pid] {
        assert!(alive(pid));
    }
    click(&mut h, "tab-close-0", 1);
    h.keys("escape");
    assert!(h.read(|view, _| view.confirm.is_none()));
    assert_eq!(focused(&h), original);
    assert_eq!(h.read(|view, _| view.panes.len()), 4);
    click(&mut h, "tab-close-0", 1);
    click(&mut h, "confirm-ok", 1);
    h.wait_for("target tab removed", |view, _| {
        view.ws.tabs().len() == 1 && view.panes.len() == 1
    });
    h.wait_for("target shells stopped", |_, _| {
        [first_pid, second_pid, third_pid]
            .iter()
            .all(|pid| !alive(*pid))
    });
    assert!(alive(survivor_pid));
    assert_eq!(h.system.0.borrow().badge, 0);
    assert_eq!(focused(&h), original);
    assert_eq!(SavedWindow::load(&h.home.data).unwrap().tabs.len(), 1);
    assert!(
        h.cx.debug_bounds("tab-close-0").is_some(),
        "a single tab keeps its close button"
    );
    // Typing after cancellation/approval has returned focus to the surviving shell.
    h.run("printf 'survivor-ok\\n'", "survivor-ok");
}

#[gpui::test]
fn shortcut_closes_only_the_focused_pane_and_review_does_not_confirm(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "close-shortcut", |_| {});
    h.wait_prompt();
    let first = focused(&h);
    h.keys("cmd-d");
    let second = focused(&h);
    h.wait_prompt();
    h.hook(Some(first.raw()), &h.home.home, HookKind::PromptSubmitted);
    h.wait_for("working first pane", |view, _| {
        view.ws.pane(first).unwrap().agent.is_some()
    });
    h.keys("cmd-w");
    assert!(h.read(|view, _| view.confirm.is_none() && view.ws.pane(second).is_none()));
    assert_eq!(focused(&h), first);
    h.hook(Some(first.raw()), &h.home.home, HookKind::WaitingInput);
    h.wait_for("waiting first pane", |view, _| {
        view.ws.pane(first).unwrap().agent.as_ref().unwrap().status == AgentStatus::WaitingInput
    });
    h.keys("cmd-w");
    assert!(h.read(|view, _| view.confirm.is_some()));
    h.keys("escape");
    assert!(h.read(|view, _| view.ws.pane(first).is_some() && view.confirm.is_none()));
    // Review is completed work. Set it without a focus event clearing it first.
    h.view.update(&mut h.cx, |view, cx| {
        view.ws
            .set_agent_status(first, "claude", AgentStatus::Review, 10);
        cx.notify();
    });
    h.keys("cmd-w");
    assert!(
        h.read(|view, _| view.ws.is_empty() && view.panes.is_empty() && view.confirm.is_none())
    );
    assert!(!SavedWindow::path(&h.home.data).exists());
}

#[gpui::test]
fn immediate_tab_close_covers_shell_review_idle_and_graphs(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "close-immediate", |_| {});
    h.wait_prompt();
    for status in [None, Some(AgentStatus::Review), Some(AgentStatus::Idle)] {
        let first = focused(&h);
        h.keys("cmd-d cmd-t");
        h.wait_prompt();
        let survivor = focused(&h);
        if let Some(status) = status {
            h.view.update(&mut h.cx, |view, cx| {
                view.ws.set_agent_status(first, "claude", status, 1);
                cx.notify();
            });
        }
        click(&mut h, "tab-close-0", 1);
        assert_eq!(focused(&h), survivor);
        assert!(h.read(|view, _| view.confirm.is_none()
            && view.ws.tabs().len() == 1
            && view.panes.len() == 1));
    }
    let repo = h.home.repo("graph");
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |view, cx| view.open_git_graph(repo.clone(), window, cx))
    });
    click(&mut h, "tab-close-0", 1); // Graph is the sole tab in its repository group.
    assert!(h.read(|view, _| view.graphs.is_empty() && view.panes.len() == 1));
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |view, cx| view.open_git_graph(repo, window, cx))
    });
    h.keys("cmd-w");
    assert!(h.read(|view, _| view.graphs.is_empty() && view.panes.len() == 1));
    click(&mut h, "tab-close-0", 1);
    assert!(h.read(|view, _| view.ws.is_empty() && view.panes.is_empty()));
    assert!(!SavedWindow::path(&h.home.data).exists());
}

#[gpui::test]
fn confirmed_close_never_targets_a_replacement_tab(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "close-stable-id", |_| {});
    h.wait_prompt();
    let target = h.read(|view, _| view.ws.active_tab().unwrap().id);
    let pane = focused(&h);
    h.hook(Some(pane.raw()), &h.home.home, HookKind::PromptSubmitted);
    h.wait_for("working target", |view, _| {
        view.ws.pane(pane).unwrap().agent.is_some()
    });
    h.keys("cmd-t");
    h.wait_prompt();
    let original = focused(&h);
    click(&mut h, "tab-close-0", 1);
    // The target can end independently while the confirmation is open.
    h.view.update(&mut h.cx, |view, cx| {
        view.ws.close_tab(target);
        view.panes.remove(&pane);
        cx.notify();
    });
    h.keys("cmd-t");
    h.wait_for("replacement pane", |view, _| view.ws.tabs().len() == 2);
    let replacement = focused(&h);
    click(&mut h, "confirm-ok", 1);
    assert_eq!(focused(&h), original);
    assert!(h.read(|view, _| view.ws.tabs().len() == 2
        && view.ws.pane(replacement).is_some()
        && view.confirm.is_none()));
    assert_eq!(SavedWindow::load(&h.home.data).unwrap().tabs.len(), 2);
}

#[gpui::test]
fn last_graph_shortcut_removes_restore_data(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "close-last-graph", |_| {});
    h.wait_prompt();
    let terminal = h.read(|view, _| view.ws.active_tab().unwrap().id);
    let repo = h.home.repo("graph");
    h.cx.update(|window, cx| {
        h.view.update(cx, |view, cx| {
            view.open_git_graph(repo, window, cx);
            view.request_close(
                crate::workspace_view::CloseTarget::Tab(terminal),
                window,
                cx,
            );
        })
    });
    assert!(h.read(|view, _| view.panes.is_empty() && view.graphs.len() == 1));
    assert!(SavedWindow::path(&h.home.data).exists());
    h.keys("cmd-w");
    assert!(
        h.read(|view, _| view.ws.is_empty() && view.graphs.is_empty() && view.confirm.is_none())
    );
    assert!(!SavedWindow::path(&h.home.data).exists());
}
