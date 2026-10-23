//! Tab-wide shutdown, pane shortcuts and cancellable agent protection (#108).
use super::harness::{Harness, git};
use crate::workspace_view::MenuAction;
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

fn read_pid(path: &std::path::Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn shell_pid(h: &mut Harness, name: &str) -> u32 {
    h.wait_prompt();
    let path = h.home.home.join(format!("{name}.pid"));
    h.type_text(&format!("echo $$ > '{}'", path.display()));
    h.keys("enter");
    h.wait_for("complete shell pid", |_, _| read_pid(&path).is_some());
    read_pid(&path).unwrap()
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
    // A foreground child stands in for the agent CLI, without a paid request.
    let child_file = h.home.home.join("working-child.pid");
    h.type_text(&format!(
        r#"sh -c 'echo $$ > "{}"; exec sleep 600'"#,
        child_file.display()
    ));
    h.keys("enter");
    h.wait_for("complete foreground child pid", |_, _| {
        read_pid(&child_file).is_some()
    });
    let child_pid = read_pid(&child_file).unwrap();
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
    for pid in [first_pid, child_pid, second_pid, third_pid, survivor_pid] {
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
        [first_pid, child_pid, second_pid, third_pid]
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
    // The graph is its repository's only tab, so closing it shows no tab
    // rather than the terminal of another place.
    click(&mut h, "tab-close-0", 1);
    assert!(h.read(|view, _| view.graphs.is_empty()
        && view.panes.len() == 1
        && view.ws.active_tab().is_none()));
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |view, cx| view.open_git_graph(repo, window, cx))
    });
    h.keys("cmd-w");
    assert!(h.read(|view, _| view.graphs.is_empty() && view.panes.len() == 1));
    let terminal = h.read(|view, _| view.ws.tabs()[0].id);
    h.cx.update(|window, cx| {
        h.view.update(cx, |view, cx| {
            view.request_close(
                crate::workspace_view::CloseTarget::Tab(terminal),
                window,
                cx,
            )
        })
    });
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

/// The worktree's agent status as PROJECT and Update branch see it.
fn worktree_status(h: &mut Harness, repo: &std::path::Path) -> (AgentStatus, String) {
    let event =
        crate::sidebar_view::SidebarEvent::WorktreeMenu(repo.to_path_buf(), gpui::Point::default());
    let status = h.read(|view, cx| {
        view.sidebar
            .read(cx)
            .model
            .worktree_for_path(repo)
            .unwrap()
            .1
            .status()
    });
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |view, cx| view.on_sidebar_event(event, window, cx))
    });
    let update = h.read(|view, _| {
        let menu = view.context_menu.as_ref().unwrap();
        let (_, action) = menu
            .items
            .iter()
            .find(|(label, _)| label.starts_with("Update branch"))
            .unwrap();
        match action {
            MenuAction::Unavailable(reason) => reason.clone(),
            MenuAction::UpdateBranch { .. } => "enabled".into(),
            other => format!("{other:?}"),
        }
    });
    h.view.update(&mut h.cx, |view, cx| {
        view.context_menu = None;
        cx.notify();
    });
    (status, update)
}

/// Stop and close, from the pane shortcut or a tab's close button, ends the
/// working status its Claude Code or Codex agent gave the worktree in PROJECT
/// and Update branch, as does a pane whose process ends under a working
/// agent; another agent pane in the branch keeps it working (#154).
#[gpui::test]
fn stop_and_close_clears_the_worktree_working_status(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "close-status", |home| {
        repo = home.repo("app");
        let src = repo.parent().unwrap();
        git(src, &["clone", "-q", "--bare", "app", "origin.git"]);
        git(&repo, &["remote", "add", "origin", "../origin.git"]);
        git(&repo, &["fetch", "-q", "origin"]);
        git(&repo, &["branch", "-q", "-u", "origin/main", "main"]);
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("an updatable main worktree", {
        let repo = repo.clone();
        move |view, cx| {
            view.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| chda_core::update_blocker(w).is_none())
        }
    });
    let cd = |h: &mut Harness, repo: &std::path::Path| {
        h.wait_prompt();
        let pane = focused(h);
        // A redundant cd would leave another prompt racing the agent's hook.
        if !h.read(|view, _| view.ws.pane(pane).and_then(|p| p.cwd.as_deref()) == Some(repo)) {
            h.run(
                &format!("cd {}", chda_term::shell_quote(&repo.to_string_lossy())),
                "test%",
            );
        }
        let repo = repo.to_path_buf();
        h.wait_for("the pane in the branch", move |view, _| {
            view.ws.pane(pane).and_then(|p| p.cwd.as_ref()) == Some(&repo)
        });
        pane
    };
    // Like an agent CLI, a foreground command holds the pane, so the shell
    // draws no prompt while hooks or titles report the turn.
    let run_agent = |h: &mut Harness, name: &str, command: &str| {
        let started = h.home.home.join(format!("{name}.started"));
        h.type_text(&format!("touch '{}'; {command}", started.display()));
        h.keys("enter");
        h.wait_for("the stand-in agent", move |_, _| started.exists());
    };
    let working = |h: &mut Harness, pane: PaneId, agent: &'static str| {
        h.wait_for("a working agent", move |view, _| {
            view.ws
                .pane(pane)
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.agent == agent && a.status == AgentStatus::Working)
        });
    };
    let focus = |h: &mut Harness, pane: PaneId| {
        let row = format!("session-pane-{}", pane.raw());
        click(h, Box::leak(row.into_boxed_str()), 1);
        assert_eq!(focused(h), pane);
    };
    // The first tab stays outside the branch, so the window survives.
    h.keys("cmd-t");
    let claude = cd(&mut h, &repo);
    run_agent(&mut h, "claude", "sleep 600");
    h.keys("cmd-d");
    let second = cd(&mut h, &repo);
    run_agent(&mut h, "second", "sleep 600");
    for (pane, session) in [(claude, "first-session"), (second, "second-session")] {
        h.hook_session(Some(pane.raw()), &repo, HookKind::PromptSubmitted, session);
        working(&mut h, pane, "claude");
    }
    h.keys("cmd-t");
    let codex = cd(&mut h, &repo);
    // Codex reports its turn only in its title.
    run_agent(
        &mut h,
        "codex",
        r"printf '\033]0;codex | Working\007'; sleep 600",
    );
    working(&mut h, codex, "codex");
    assert_eq!(
        worktree_status(&mut h, &repo),
        (AgentStatus::Working, "Claude Code is working here".into())
    );

    // Stop and close one Claude Code pane: the other keeps the branch working.
    focus(&mut h, second);
    h.keys("cmd-w");
    assert!(h.read(|view, _| view.confirm.is_some()));
    click(&mut h, "confirm-ok", 1);
    assert!(h.read(|view, _| view.ws.pane(second).is_none()));
    assert_eq!(
        worktree_status(&mut h, &repo),
        (AgentStatus::Working, "Claude Code is working here".into())
    );
    // Closing the last one ends Claude Code's status; Codex still works.
    focus(&mut h, claude);
    h.keys("cmd-w");
    assert!(h.read(|view, _| view.confirm.is_some()));
    click(&mut h, "confirm-ok", 1);
    assert!(h.read(|view, _| view.ws.pane(claude).is_none()));
    assert_eq!(
        worktree_status(&mut h, &repo),
        (AgentStatus::Working, "Codex is working here".into())
    );

    // Stop and close the Codex tab from its close button.
    focus(&mut h, codex);
    click(&mut h, "tab-close-0", 1);
    assert!(h.read(|view, _| view.confirm.is_some()));
    click(&mut h, "confirm-ok", 1);
    assert!(h.read(|view, _| view.ws.pane(codex).is_none()));
    let (status, update) = worktree_status(&mut h, &repo);
    assert_ne!(status, AgentStatus::Working);
    assert_eq!(update, "enabled");

    // An agent pane whose process ends without a session end.
    h.keys("cmd-t");
    let ending = cd(&mut h, &repo);
    // Reading the PTY child avoids an extra shell command and its prompt
    // racing the following out-of-band hook. exec keeps this process ID.
    let pid = h.read(|view, cx| {
        view.panes[&ending]
            .0
            .read(cx)
            .child_pid()
            .expect("the ending pane has a shell process")
    });
    run_agent(&mut h, "ending", "exec sleep 600");
    h.hook_session(
        Some(ending.raw()),
        &repo,
        HookKind::PromptSubmitted,
        "ending-session",
    );
    working(&mut h, ending, "claude");
    assert_eq!(worktree_status(&mut h, &repo).0, AgentStatus::Working);
    Command::new("kill").arg(pid.to_string()).status().unwrap();
    h.wait_for("the ended shell's pane to close", move |view, _| {
        view.ws.pane(ending).is_none()
    });
    let (status, update) = worktree_status(&mut h, &repo);
    assert_ne!(status, AgentStatus::Working);
    assert_eq!(update, "enabled");
    assert_eq!(h.read(|view, _| view.ws.tabs().len()), 1);
    assert!(h.read(|view, _| !view.panes.contains_key(&ending)));
}

/// Click a PROJECT worktree row, which opens or focuses its terminal.
fn open_worktree(h: &mut Harness, path: &std::path::Path) {
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|w, cx| w.simulate_next_frame(cx));
    }
    let selector: &'static str = Box::leak(format!("wt:{}", path.display()).into_boxed_str());
    click(h, selector, 1);
}

#[gpui::test]
fn exit_closes_the_tab_and_the_window_stays_on_its_branch(cx: &mut TestAppContext) {
    let (mut a, mut z) = (std::path::PathBuf::new(), std::path::PathBuf::new());
    let mut h = Harness::open(cx, "exit-stays", |home| {
        let repo = home.repo("project");
        a = home.home.join("feature-a");
        z = home.home.join("feature-z");
        for (branch, path) in [("feature/a", &a), ("feature/z", &z)] {
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    branch,
                    path.to_str().unwrap(),
                ],
            );
        }
        let mut config = chda_config::ChdaConfig::default();
        config.repos.push(repo.clone());
        config.save(&home.config).unwrap();
    });
    h.wait_prompt();
    h.wait_for("worktrees", |v, cx| {
        v.sidebar.read(cx).model.repos[0].worktrees.len() == 3
    });
    let selected = |h: &Harness| h.read(|v, cx| v.sidebar.read(cx).selected.clone());
    let active = |h: &Harness| h.read(|v, _| v.ws.active_tab().map(|t| t.id));
    // Tabs: home, a, z, then a second a at the end.
    open_worktree(&mut h, &a);
    h.wait_prompt();
    let first_a = active(&h);
    open_worktree(&mut h, &z);
    h.wait_prompt();
    open_worktree(&mut h, &a);
    assert_eq!(active(&h), first_a);
    h.keys("cmd-t");
    h.wait_prompt();
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 4);

    // exit shows the nearest tab of the same branch, not z beside it.
    h.type_text("exit");
    h.keys("enter");
    h.wait_for("the second a tab to close", |v, _| v.ws.tabs().len() == 3);
    assert_eq!(active(&h), first_a);
    assert_eq!(selected(&h), Some(a.clone()));

    // The branch's last tab: no tab shows; the window and branch stay.
    h.type_text("exit");
    h.keys("enter");
    h.wait_for("the last a tab to close", |v, _| v.ws.tabs().len() == 2);
    h.read(|v, _| {
        assert!(
            v.ws.active_tab().is_none(),
            "another branch's tab took over"
        );
        assert!(v.ws.focused_pane().is_none());
    });
    assert_eq!(selected(&h), Some(a.clone()), "the sidebar moved away");
    assert!(h.cx.debug_bounds("tab-new").is_some());

    // + opens a terminal in that branch again.
    click(&mut h, "tab-new", 1);
    h.wait_prompt();
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 3);
    assert_eq!(
        h.read(|v, _| v.ws.pane(v.ws.focused_pane().unwrap()).unwrap().cwd.clone()),
        Some(a.clone())
    );

    // cmd-w follows the same rule; with other tabs left it closes nothing more.
    h.keys("cmd-w");
    assert!(h.read(|v, _| v.ws.active_tab().is_none() && v.ws.tabs().len() == 2));
    h.keys("cmd-w");
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
    assert_eq!(selected(&h), Some(a.clone()));

    // Closing the window's last tab keeps the window; cmd-w then closes it.
    let ids: Vec<_> = h.read(|v, _| v.ws.tabs().iter().map(|t| t.id).collect());
    for id in ids {
        h.cx.update(|window, cx| {
            h.view.update(cx, |v, cx| {
                v.request_close(crate::workspace_view::CloseTarget::Tab(id), window, cx)
            })
        });
    }
    h.cx.run_until_parked();
    assert!(h.read(|v, _| v.ws.is_empty()));
    assert!(
        h.cx.debug_bounds("tab-new").is_some(),
        "the empty window stays"
    );
    let windows = h.cx.windows().len();
    h.keys("cmd-w");
    assert_eq!(h.cx.windows().len(), windows - 1);
}
