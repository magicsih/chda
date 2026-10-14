//! `chda mcp`'s requests as the app receives them over the hook socket:
//! create a worktree, open a tab, list worktrees.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chda_core::agents::control::{Reply, Request};
use chda_core::agents::ipc;
use gpui::TestAppContext;

use super::harness::{Harness, git};

/// Send `request` the way `chda mcp` does and wait for the app's reply.
pub(super) fn ask(h: &mut Harness, request: Request) -> Reply {
    let socket = ipc::socket_path(&h.home.data);
    let slot = Arc::new(Mutex::new(None));
    let filled = Arc::clone(&slot);
    std::thread::spawn(move || {
        *filled.lock().unwrap() = Some(ipc::request(&socket, &request).unwrap());
    });
    let waiting = Arc::clone(&slot);
    h.wait_for("the app's reply", move |_, _| {
        waiting.lock().unwrap().is_some()
    });
    slot.lock().unwrap().take().unwrap()
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[gpui::test]
fn an_agent_creates_a_worktree_opens_tabs_and_lists_worktrees(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "mcp", |home| {
        repo = home.repo("app");
        home.fake_claude();
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("the main worktree", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .first()
            .is_some_and(|r| !r.worktrees.is_empty())
    });
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "second"]);
    let head = git_out(&repo, &["rev-parse", "HEAD~1"]);

    // From inside the repository: a new branch starting at the first
    // commit, with a note, and Claude Code started in a tab there.
    let reply = ask(
        &mut h,
        Request::CreateWorktree {
            cwd: repo.clone(),
            branch: Some("agent/task".into()),
            base: Some(head.clone()),
            note: Some("Fix login redirect".into()),
            open: true,
            agent: Some("claude".into()),
        },
    );
    assert!(reply.ok, "{}", reply.message);
    let path = std::path::PathBuf::from(reply.data["path"].as_str().unwrap());
    assert_eq!(
        path,
        repo.parent().unwrap().join("app.worktrees/agent-task")
    );
    assert_eq!(git_out(&path, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git_out(&repo, &["config", "branch.agent/task.description"]),
        "Fix login redirect"
    );
    assert!(reply.message.contains("confirm"));
    h.confirm_agent_launch();
    h.wait_for("the agent's tab in the new worktree", {
        let path = path.clone();
        move |v, cx| {
            v.ws.tabs().len() == 2
                && v.sidebar
                    .read(cx)
                    .model
                    .worktree_for_path(&path)
                    .is_some_and(|(_, w)| w.path == path && w.panes.len() == 1)
        }
    });
    h.wait_for("Claude Code running there", |v, cx| {
        v.focused_text(cx).contains("fake-claude")
    });

    // Without a branch name chda picks a free random one; a tab is not
    // required.
    let reply = ask(
        &mut h,
        Request::CreateWorktree {
            cwd: path.clone(),
            branch: None,
            base: None,
            note: None,
            open: false,
            agent: None,
        },
    );
    assert!(reply.ok, "{}", reply.message);
    let random = reply.data["branch"].as_str().unwrap().to_owned();
    assert_eq!(random.split('-').count(), 2, "{random}");
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);

    let reply = ask(
        &mut h,
        Request::OpenTab {
            path: repo.clone(),
            agent: None,
        },
    );
    assert!(reply.ok, "{}", reply.message);
    h.wait_for("a third tab", |v, _| v.ws.tabs().len() == 3);
    let reply = ask(
        &mut h,
        Request::OpenTab {
            path: repo.join("missing"),
            agent: None,
        },
    );
    assert!(!reply.ok);
    let reply = ask(
        &mut h,
        Request::OpenTab {
            path: repo.clone(),
            agent: Some("codex".into()),
        },
    );
    assert!(reply.message.contains("not on PATH"), "{}", reply.message);

    h.wait_for("all three worktrees in the sidebar", move |v, cx| {
        v.sidebar.read(cx).model.repos[0].worktrees.len() == 3
    });
    let reply = ask(&mut h, Request::ListWorktrees { cwd: path.clone() });
    let worktrees = reply.data["worktrees"].as_array().unwrap();
    assert_eq!(worktrees.len(), 3);
    let task = worktrees
        .iter()
        .find(|w| w["branch"] == "agent/task")
        .unwrap();
    assert_eq!(task["note"], "Fix login redirect");
    assert_eq!(task["main"], false);
}

/// Send `request` like [`ask`] without waiting; the reply lands in the slot.
fn ask_later(h: &Harness, request: Request) -> Arc<Mutex<Option<Reply>>> {
    let socket = ipc::socket_path(&h.home.data);
    let slot = Arc::new(Mutex::new(None));
    let filled = Arc::clone(&slot);
    std::thread::spawn(move || {
        *filled.lock().unwrap() = Some(ipc::request(&socket, &request).unwrap());
    });
    slot
}

fn answered(h: &mut Harness, slot: &Arc<Mutex<Option<Reply>>>) -> Reply {
    let waiting = Arc::clone(slot);
    h.wait_for("the app's reply", move |_, _| {
        waiting.lock().unwrap().is_some()
    });
    slot.lock().unwrap().take().unwrap()
}

/// A coordinating agent lists the sessions of its repository, reads a
/// pane's output and waits for another agent to finish (#249).
#[gpui::test]
fn an_agent_lists_reads_and_waits_for_other_sessions_of_its_repository(cx: &mut TestAppContext) {
    use chda_core::agents::HookKind;
    let (mut repo, mut other) = Default::default();
    let mut h = Harness::open(cx, "mcp-coordinate", |home| {
        repo = home.repo("app");
        other = home.repo("lib");
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\", \"{}\"]\n",
                repo.display(),
                other.display()
            ),
        )
        .unwrap();
        // The helper's conversation: the user's prompt and its answer.
        home.claude_session(&repo, "helper", "write the tests");
        let transcript = home.claude_projects.join("project/helper.jsonl");
        let answer = serde_json::json!({"type":"assistant","sessionId":"helper","uuid":"a1","message":{"role":"assistant","content":[{"type":"text","text":"Tests written and passing."}]}});
        let text = std::fs::read_to_string(&transcript).unwrap();
        std::fs::write(&transcript, format!("{text}{answer}\n")).unwrap();
    });
    h.wait_prompt();
    h.run(
        &format!("cd '{}'; printf 'in-%s\\n' app", repo.display()),
        "in-app",
    );
    let helper = h.read(|v, _| v.ws.focused_pane().unwrap());
    // The helper agent holds its terminal, as a real one does: a shell
    // prompt would end it.
    h.type_text("cat");
    h.keys("enter");
    h.hook_session(Some(helper.raw()), &repo, HookKind::SessionStart, "helper");
    h.hook_session(
        Some(helper.raw()),
        &repo,
        HookKind::PromptSubmitted,
        "helper",
    );
    h.wait_for("the helper working", move |v, _| {
        v.ws.pane(helper).is_some_and(|i| {
            i.agent_live
                && i.agent
                    .as_ref()
                    .is_some_and(|a| a.status == chda_core::AgentStatus::Working)
        })
    });
    h.keys("cmd-d");
    h.wait_prompt();
    h.run(
        &format!("cd '{}'; printf 'out-%s\\n' visible", repo.display()),
        "out-visible",
    );
    let shell = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.keys("cmd-t");
    h.wait_prompt();
    h.run(
        &format!("cd '{}'; printf 'in-%s\\n' lib", other.display()),
        "in-lib",
    );
    let elsewhere = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.wait_for("panes in their repositories", move |v, _| {
        [helper, shell, elsewhere]
            .iter()
            .all(|p| v.ws.pane(*p).is_some_and(|i| i.repo.is_some()))
    });
    let focused = h.read(|v, _| v.ws.focused_pane());

    let listed = ask(&mut h, Request::ListSessions { cwd: repo.clone() });
    assert!(listed.ok, "{}", listed.message);
    let sessions = listed.data["sessions"].as_array().unwrap().clone();
    let helper_row = sessions
        .iter()
        .find(|s| s["pane"] == helper.raw())
        .expect("the helper agent is listed");
    assert_eq!(helper_row["status"], "working", "{sessions:#?}");
    assert_eq!(helper_row["agent"], "claude");
    assert!(
        sessions.iter().all(|s| s["pane"] != elsewhere.raw()),
        "only the caller's repository: {sessions:?}"
    );

    // The shell's screen up to its prompt, never the line being typed.
    h.type_text("unfinished-draft");
    let read = ask(
        &mut h,
        Request::ReadOutput {
            cwd: repo.clone(),
            pane: shell.raw(),
            lines: Some(50),
        },
    );
    assert!(read.ok, "{}", read.message);
    let text = read.data["text"].as_str().unwrap();
    assert!(text.contains("out-visible"), "{text}");
    assert!(!text.contains("unfinished-draft"), "{text}");
    // An agent's latest answer comes from its exact transcript.
    let answer = ask(
        &mut h,
        Request::ReadOutput {
            cwd: repo.clone(),
            pane: helper.raw(),
            lines: None,
        },
    );
    assert_eq!(answer.data["source"], "transcript", "{}", answer.message);
    assert_eq!(answer.data["text"], "Tests written and passing.");
    // Another repository's pane stays out of reach.
    let refused = ask(
        &mut h,
        Request::ReadOutput {
            cwd: repo.clone(),
            pane: elsewhere.raw(),
            lines: None,
        },
    );
    assert!(!refused.ok && refused.message.contains("not in your repository"));

    // Wait for the helper to finish its turn.
    let waiting = ask_later(
        &h,
        Request::WaitForStatus {
            cwd: repo.clone(),
            pane: helper.raw(),
            states: vec!["turn_complete".into(), "waiting_for_input".into()],
            timeout_ms: Some(20_000),
        },
    );
    h.hook_session(Some(helper.raw()), &repo, HookKind::Stopped, "helper");
    let done = answered(&mut h, &waiting);
    assert_eq!(done.data["state"], "turn_complete", "{}", done.message);
    assert_eq!(done.data["timed_out"], false);
    // A state it already has answers at once.
    let now = ask(
        &mut h,
        Request::WaitForStatus {
            cwd: repo.clone(),
            pane: helper.raw(),
            states: vec!["turn_complete".into()],
            timeout_ms: None,
        },
    );
    assert_eq!(now.data["waited_ms"], 0);
    // The timeout is honored.
    let slow = ask_later(
        &h,
        Request::WaitForStatus {
            cwd: repo.clone(),
            pane: helper.raw(),
            states: vec!["working".into()],
            timeout_ms: Some(1_000),
        },
    );
    h.wait_for("the wait to register", |v, _| !v.status_waits.is_empty());
    h.cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    let expired = answered(&mut h, &slow);
    assert_eq!(expired.data["timed_out"], true);
    assert_eq!(expired.data["state"], "turn_complete");
    // A closing pane ends the wait.
    let closing = ask_later(
        &h,
        Request::WaitForStatus {
            cwd: repo.clone(),
            pane: shell.raw(),
            states: vec!["working".into()],
            timeout_ms: Some(20_000),
        },
    );
    h.wait_for("the wait to register", |v, _| !v.status_waits.is_empty());
    assert_eq!(
        h.read(|v, _| v.ws.focused_pane()),
        focused,
        "tools never move focus"
    );
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                crate::sidebar_view::SidebarEvent::FocusPane(shell),
                window,
                cx,
            )
        })
    });
    h.cx.run_until_parked();
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(shell));
    h.keys("cmd-w");
    let closed = answered(&mut h, &closing);
    assert_eq!(closed.data["state"], "closed", "{}", closed.message);
}
