//! Worktrees in the sidebar: create, delete, missing folders.

use gpui::TestAppContext;

use super::harness::{Harness, git};
use crate::workspace_view::MenuAction;

fn config_with_repo(home: &super::harness::Home, repo: &std::path::Path) {
    std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
}

#[gpui::test]
fn create_then_delete_a_worktree_in_one_go(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "wt", |home| {
        repo = home.repo("app");
        config_with_repo(home, &repo);
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

    // New worktree sheet: type a branch and press enter.
    let r = repo.clone();
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_sheet(r, window, cx)));
    h.type_text("feat/x");
    h.keys("enter");
    let path = repo.parent().unwrap().join("app.worktrees/feat-x");
    h.wait_for("the new worktree in the sidebar", {
        let path = path.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&path)
                .is_some_and(|(_, w)| w.path == path)
        }
    });
    assert!(path.is_dir());

    // Untracked build output makes removal slow; one confirmation must do.
    for i in 0..300 {
        std::fs::write(path.join(format!("out-{i}.o")), "x").unwrap();
    }
    let action = MenuAction::DeleteWorktreeAndBranch {
        repo: repo.clone(),
        worktree: path.clone(),
        force: false,
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    assert!(h.read(|v, _| v.confirm.is_some()), "asks first");
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.confirm_action(window, cx)));
    h.wait_for("the worktree to be gone after one confirmation", {
        let path = path.clone();
        move |v, cx| {
            !path.exists()
                && v.sidebar.read(cx).model.repos[0]
                    .worktrees
                    .iter()
                    .all(|w| w.path != path)
        }
    });
}

#[gpui::test]
fn missing_worktree_is_marked_and_pruned(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut gone = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "miss", |home| {
        repo = home.repo("app");
        gone = repo.parent().unwrap().join("app.worktrees/old");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "old", gone.to_str().unwrap()],
        );
        std::fs::remove_dir_all(&gone).unwrap();
        config_with_repo(home, &repo);
    });
    h.wait_for("the missing worktree", {
        let gone = gone.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .repos
                .first()
                .is_some_and(|r| r.worktrees.iter().any(|w| w.path == gone && w.missing))
        }
    });
    let g = gone.clone();
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_worktree(&g, window, cx)));
    assert!(h.read(|v, _| {
        v.status_line
            .as_deref()
            .is_some_and(|s| s.contains("no longer exists"))
    }));
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        1,
        "no tab for a missing folder"
    );

    let action = MenuAction::PruneMissing {
        repo: repo.clone(),
        worktree: gone.clone(),
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.wait_for("the record to be pruned", move |v, cx| {
        v.sidebar.read(cx).model.repos[0]
            .worktrees
            .iter()
            .all(|w| w.path != gone)
    });
    git(&repo, &["rev-parse", "--verify", "-q", "refs/heads/old"]);
}

/// S3: a worktree lists the agent sessions that ran in it.
#[gpui::test]
fn worktree_lists_its_agent_sessions(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "sess", |home| {
        repo = home.repo("app");
        config_with_repo(home, &repo);
        home.claude_session(&repo.join("src"), "s-here", "fix the login loop");
        home.claude_session(&home.home.join("elsewhere"), "s-away", "unrelated");
    });
    h.wait_for("the session under the main worktree", {
        let repo = repo.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| {
                    w.sessions.len() == 1
                        && w.sessions[0].id == "s-here"
                        && w.sessions[0].snippet == "fix the login loop"
                })
        }
    });
    assert!(h.read(|v, cx| v.sidebar.read(cx).sessions_loaded));
}

/// S4: merge a finished worktree into main and clean it up.
#[gpui::test]
fn merge_into_main_and_clean_up(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut wt = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "merge", |home| {
        repo = home.repo("app");
        wt = repo.parent().unwrap().join("app.worktrees/done");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "done", wt.to_str().unwrap()],
        );
        std::fs::write(wt.join("feature.txt"), "done\n").unwrap();
        git(&wt, &["add", "feature.txt"]);
        git(&wt, &["commit", "-q", "-m", "add feature"]);
        config_with_repo(home, &repo);
    });
    h.wait_for("the worktree in the sidebar", {
        let wt = wt.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&wt)
                .is_some_and(|(_, w)| w.path == wt)
        }
    });
    let action = MenuAction::MergeAndClean {
        repo: repo.clone(),
        worktree: wt.clone(),
        force: false,
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.wait_for("the worktree to be merged and removed", {
        let wt = wt.clone();
        move |v, cx| {
            !wt.exists()
                && v.sidebar.read(cx).model.repos[0]
                    .worktrees
                    .iter()
                    .all(|w| w.path != wt)
        }
    });
    assert!(repo.join("feature.txt").is_file(), "main has the commit");
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "-q", "refs/heads/done"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(!branch.success(), "the branch is deleted");
}
