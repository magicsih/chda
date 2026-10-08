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
