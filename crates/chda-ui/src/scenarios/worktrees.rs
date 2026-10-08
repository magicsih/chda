//! Worktrees in the sidebar: create, delete, missing folders, sessions.

use gpui::TestAppContext;

use super::harness::{Harness, git};
use crate::sidebar_view::SessionPick;
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

    // A terminal open in the worktree, a locked worktree (which
    // `git worktree remove --force` refuses) and untracked build output
    // (which makes removal slow): one confirmation must still do.
    let p = path.clone();
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_worktree(&p, window, cx)));
    h.wait_for("the terminal in the worktree", {
        let path = path.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&path)
                .is_some_and(|(_, w)| w.path == path && w.panes.len() == 1)
        }
    });
    git(&repo, &["worktree", "lock", path.to_str().unwrap()]);
    let deps = path.join("node_modules/pkg");
    std::fs::create_dir_all(&deps).unwrap();
    for i in 0..300 {
        std::fs::write(deps.join(format!("out-{i}.js")), "x").unwrap();
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
    assert!(
        h.read(|v, _| v
            .confirm
            .as_ref()
            .is_some_and(|c| c.lines.iter().any(|l| l.contains("Closes the terminal")))),
        "asks first and names the open terminal"
    );
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.confirm_action(window, cx)));
    assert_eq!(
        h.read(|v, cx| v
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(&path)
            .and_then(|(_, w)| w.busy.clone())),
        Some("deleting".to_owned()),
        "the row says it is being deleted"
    );
    // A second request while it runs is ignored instead of failing.
    let again = MenuAction::DeleteWorktreeAndBranch {
        repo: repo.clone(),
        worktree: path.clone(),
        force: true,
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(again, window, cx))
    });
    assert!(h.read(|v, _| v.confirm.is_none()));
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
    assert!(
        h.read(|v, _| v.notifications.latest() == Some(&format!("Deleted {}", path.display()))),
        "no error: {:?}",
        h.read(|v, _| v.notifications.latest().map(str::to_owned))
    );
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1, "its terminal closed");
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "-q", "refs/heads/feat/x"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(!branch.success(), "the branch is deleted");
    h.wait_for("the moved folder to be emptied", {
        let trash = repo.join(".git/chda-trash");
        move |_, _| !trash.exists()
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
        v.notifications
            .latest()
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

/// Moving the mouse over a session row swaps its age for "Resume" without
/// crashing the window: the swap must not change what is laid out between
/// prepaint and paint ("must call prepaint before paint").
#[gpui::test]
fn hovering_session_rows_does_not_crash(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "hover", |home| {
        repo = home.repo("app");
        config_with_repo(home, &repo);
        for id in ["h1", "h2", "h3"] {
            home.claude_session(&repo, id, &format!("task {id}"));
        }
    });
    h.wait_for("the sessions", {
        let repo = repo.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| w.sessions.len() == 3)
        }
    });
    let sidebar = h.read(|v, _| v.sidebar.clone());
    sidebar.update(&mut h.cx, |s, cx| s.toggle_expanded(&repo, cx));
    h.cx.run_until_parked();
    // Sweep down the sidebar and back up, a frame per step, entering and
    // leaving every row.
    let ys: Vec<f32> = (0..120).map(|i| i as f32 * 3.0).collect();
    for y in ys.iter().chain(ys.iter().rev()) {
        h.cx.simulate_mouse_move(
            gpui::point(gpui::px(120.0), gpui::px(*y)),
            None,
            gpui::Modifiers::none(),
        );
        h.cx.run_until_parked();
    }
}

/// S3: sessions are listed by their last message; picking three resumes
/// them in one tab, a pane each.
#[gpui::test]
fn picked_sessions_resume_in_one_tab(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "resume", |home| {
        repo = home.repo("app");
        config_with_repo(home, &repo);
        home.fake_claude();
        for id in ["s1", "s2", "s3"] {
            home.claude_session(&repo, id, &format!("task {id}"));
            // Distinct modification times: the last written is the newest.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
    h.wait_prompt();
    h.wait_for("three sessions, the last active first", {
        let repo = repo.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| {
                    let ids: Vec<&str> = w.sessions.iter().map(|s| s.id.as_str()).collect();
                    ids == ["s3", "s2", "s1"]
                })
        }
    });
    let sidebar = h.read(|v, _| v.sidebar.clone());
    sidebar.update(&mut h.cx, |s, cx| {
        assert_eq!(s.agents[0].short, "CC");
        for id in ["s1", "s2", "s3"] {
            s.toggle_pick(
                SessionPick {
                    worktree: repo.clone(),
                    agent: "claude".into(),
                    session: id.into(),
                },
                cx,
            );
        }
        s.resume_picked(cx);
    });
    h.wait_for("one new tab running the three sessions", |v, cx| {
        let tabs = v.ws.tabs();
        tabs.len() == 2 && {
            let panes = tabs[1].panes();
            panes.len() == 3
                && ["s1", "s2", "s3"].iter().all(|id| {
                    panes
                        .iter()
                        .any(|p| v.pane_text(*p, cx).contains(&format!("--resume {id}")))
                })
        }
    });
    assert!(h.read(|v, cx| v.sidebar.read(cx).picked.is_empty()));
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

fn rev(dir: &std::path::Path, rev: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "rev-parse {rev}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The "Update branch" item of a worktree's context menu.
fn update_item(h: &mut Harness, worktree: &std::path::Path) -> (String, MenuAction) {
    let event = crate::sidebar_view::SidebarEvent::WorktreeMenu(
        worktree.to_path_buf(),
        gpui::Point::default(),
    );
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.on_sidebar_event(event, window, cx))
    });
    h.read(|v, _| {
        v.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|(label, _)| label.starts_with("Update branch"))
            .cloned()
            .unwrap()
    })
}

/// Update a worktree's branch from its upstream only when that is safe, and
/// never rewrite pushed commits on divergence (#28).
#[gpui::test]
fn update_branch_from_upstream_when_safe(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut other = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "pull", |home| {
        repo = home.repo("app");
        let src = repo.parent().unwrap();
        git(src, &["clone", "-q", "--bare", "app", "origin.git"]);
        git(&repo, &["remote", "add", "origin", "../origin.git"]);
        git(&repo, &["fetch", "-q", "origin"]);
        git(&repo, &["branch", "-q", "-u", "origin/main", "main"]);
        git(src, &["clone", "-q", "origin.git", "other"]);
        other = src.join("other");
        git(
            &other,
            &["commit", "-q", "--allow-empty", "-m", "upstream 1"],
        );
        git(&other, &["push", "-q", "origin", "main"]);
        config_with_repo(home, &repo);
    });
    // The first refresh fetches the default branch: main is one behind.
    h.wait_for("main to be behind", {
        let repo = repo.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| w.badges.behind == Some(1))
        }
    });
    let head = |dir: &std::path::Path| rev(dir, "HEAD");

    // Dirty: offered but disabled with the reason. The repository watcher
    // only watches `.git`; edits in the working tree show up on the next
    // periodic refresh, which the test platform's clock does not run, so ask
    // for one.
    std::fs::write(repo.join("scratch.txt"), "x").unwrap();
    let refresh = MenuAction::RefreshRepo(repo.clone());
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(refresh, window, cx))
    });
    h.wait_for("the change badge", {
        let repo = repo.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| w.badges.dirty_count() == 1)
        }
    });
    let (_, action) = update_item(&mut h, &repo);
    assert!(
        matches!(&action, MenuAction::Unavailable(r) if r == "1 uncommitted change(s)"),
        "{action:?}"
    );
    std::fs::remove_file(repo.join("scratch.txt")).unwrap();
    let refresh = MenuAction::RefreshRepo(repo.clone());
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(refresh, window, cx))
    });

    // An agent working here: disabled too.
    h.hook(None, &repo, chda_core::agents::HookKind::PromptSubmitted);
    h.wait_for("the agent to work", {
        let repo = repo.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| w.badges.dirty_count() == 0 && !w.agents.is_empty())
        }
    });
    let (_, action) = update_item(&mut h, &repo);
    assert!(
        matches!(&action, MenuAction::Unavailable(r) if r == "Claude Code is working here"),
        "{action:?}"
    );
    h.hook(None, &repo, chda_core::agents::HookKind::SessionEnd);
    h.wait_for("the agent to end", {
        let repo = repo.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&repo)
                .is_some_and(|(_, w)| w.status() == chda_core::AgentStatus::Idle)
        }
    });

    // Behind only and clean: one click fast-forwards.
    let (label, action) = update_item(&mut h, &repo);
    assert_eq!(label, "Update branch (\u{2193}1)");
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.wait_for("the fast-forward", |v, _| {
        v.notifications.latest() == Some("Fast-forwarded main by 1 commit(s)")
    });
    assert_eq!(head(&repo), head(&other));

    // Diverged with a local commit nobody has: no silent rewrite; the sheet
    // offers a rebase.
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "local"]);
    git(
        &other,
        &["commit", "-q", "--allow-empty", "-m", "upstream 2"],
    );
    git(&other, &["push", "-q", "origin", "main"]);
    let local = head(&repo);
    let action = MenuAction::UpdateBranch {
        repo: repo.clone(),
        worktree: repo.clone(),
        strategy: None,
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.wait_for("the choice", |v, _| v.confirm.is_some());
    assert_eq!(head(&repo), local, "nothing rewritten yet");
    assert!(h.read(|v, _| v.confirm.as_ref().unwrap().lines[1].contains("rebase")));
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.confirm_action(window, cx)));
    h.wait_for("the rebase", |v, _| {
        v.notifications.latest() == Some("Rebased main on 1 new upstream commit(s)")
    });
    assert_eq!(rev(&repo, "HEAD~1"), head(&other), "local commit on top");
}

/// Stacked work: a new worktree starts at another branch's HEAD, with a
/// random name when the field is left empty (#27).
#[gpui::test]
fn new_worktree_from_a_branch_with_a_random_name(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut feat = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "from", |home| {
        repo = home.repo("app");
        feat = repo.parent().unwrap().join("app.worktrees/feat-a");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/a",
                feat.to_str().unwrap(),
            ],
        );
        git(&feat, &["commit", "-q", "--allow-empty", "-m", "work on a"]);
        config_with_repo(home, &repo);
    });
    h.wait_for("feat/a in the sidebar", {
        let feat = feat.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&feat)
                .is_some_and(|(_, w)| w.path == feat)
        }
    });
    let count = |h: &Harness| h.read(|v, cx| v.sidebar.read(cx).model.repos[0].worktrees.len());
    let before = count(&h);

    let from_a = MenuAction::NewWorktreeFrom {
        repo: repo.clone(),
        base: "feat/a".into(),
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(from_a.clone(), window, cx))
    });
    h.keys("enter");
    h.wait_for("the new worktree", move |v, cx| {
        v.sidebar.read(cx).model.repos[0].worktrees.len() == before + 1
    });
    let new = h.read(|v, cx| {
        v.sidebar.read(cx).model.repos[0]
            .worktrees
            .iter()
            .find(|w| !w.is_main && w.branch.as_deref() != Some("feat/a"))
            .cloned()
            .unwrap()
    });
    let name = new.branch.clone().unwrap();
    assert!(
        name.split('-').count() == 2 && name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
        "random adjective-noun name: {name}"
    );
    assert_eq!(
        rev(&new.path, "HEAD"),
        rev(&repo, "feat/a"),
        "starts at feat/a"
    );

    // A typed name that is already a branch is refused in the sheet.
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(from_a, window, cx))
    });
    h.cx.run_until_parked();
    assert!(
        h.cx.debug_bounds("branch-hint").is_some(),
        "an empty name explains the random one"
    );
    h.type_text("feat/a");
    h.cx.run_until_parked();
    assert!(
        h.cx.debug_bounds("branch-hint").is_none(),
        "a typed name is not left blank"
    );
    h.keys("enter");
    assert!(
        h.read(|v, _| v
            .sheet_error()
            .is_some_and(|e| e.contains("already exists"))),
        "collision shown in the sheet"
    );
    h.keys("escape");

    // So is a name whose folder exists, in the plain sheet too.
    std::fs::create_dir_all(repo.parent().unwrap().join("app.worktrees/taken")).unwrap();
    let r = repo.clone();
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_sheet(r, window, cx)));
    h.type_text("taken");
    h.keys("enter");
    assert!(
        h.read(|v, _| v
            .sheet_error()
            .is_some_and(|e| e.contains("already exists"))),
        "path collision shown in the sheet"
    );
    assert_eq!(count(&h), before + 1);
    h.keys("escape");

    // An open context menu blocks the rows under it: a right-click on
    // another row only dismisses it, and hovered rows show no tooltip over it.
    let row = |h: &mut Harness, path: &std::path::Path| {
        h.cx.run_until_parked();
        h.cx.debug_bounds(Box::leak(format!("wt:{}", path.display()).into_boxed_str()))
            .unwrap()
            .center()
    };
    let at = row(&mut h, &repo);
    h.cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
    h.cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
    h.cx.run_until_parked();
    assert!(h.read(|v, _| v.context_menu.is_some()));
    let other = row(&mut h, &feat);
    h.cx.simulate_mouse_down(other, gpui::MouseButton::Right, gpui::Modifiers::none());
    h.cx.simulate_mouse_up(other, gpui::MouseButton::Right, gpui::Modifiers::none());
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v.context_menu.is_none()),
        "the row under the menu did not get the click"
    );
}

fn note_of(dir: &std::path::Path, branch: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["config", &format!("branch.{branch}.description")])
        .current_dir(dir)
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
}

/// Notes on branches: edited in a sheet, given when creating a worktree or
/// set from outside (`chda note`), shown in the sidebar and found by the
/// palette (#36).
#[gpui::test]
fn branch_notes_show_in_the_sidebar(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut feat = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "note", |home| {
        repo = home.repo("app");
        feat = repo.parent().unwrap().join("app.worktrees/feat");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat",
                feat.to_str().unwrap(),
            ],
        );
        config_with_repo(home, &repo);
    });
    let note_in_sidebar = |path: std::path::PathBuf| {
        move |v: &crate::workspace_view::WorkspaceView, cx: &gpui::App| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&path)
                .and_then(|(_, w)| w.note_title().map(str::to_owned))
        }
    };
    h.wait_for("feat in the sidebar", {
        let feat = feat.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&feat)
                .is_some_and(|(_, w)| w.path == feat && w.note.is_none())
        }
    });

    // Edit note...: two lines, the first is the title.
    let edit = MenuAction::EditNote {
        repo: repo.clone(),
        branch: "feat".into(),
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(edit, window, cx))
    });
    h.type_text("Fix login redirect loop");
    h.keys("shift-enter");
    h.type_text("see #123");
    h.keys("enter");
    assert_eq!(
        note_of(&repo, "feat").as_deref(),
        Some("Fix login redirect loop\nsee #123")
    );
    assert_eq!(
        h.read(note_in_sidebar(feat.clone())).as_deref(),
        Some("Fix login redirect loop")
    );
    let found = h.read(|v, cx| {
        v.palette_items(cx).into_iter().any(|i| {
            matches!(&i.command, crate::palette::PaletteCommand::GoToWorktree(p) if *p == feat)
                && crate::palette::fuzzy_score("redirect", &format!("{} {}", i.label, i.detail))
                    .is_some()
        })
    });
    assert!(found, "the palette finds the worktree by its note");

    // Set from outside, as `chda note` does: the sidebar follows.
    git(
        &repo,
        &["config", "branch.feat.description", "Ship the fix\n"],
    );
    h.wait_for("the new note", {
        let f = note_in_sidebar(feat.clone());
        move |v, cx| f(v, cx).as_deref() == Some("Ship the fix")
    });

    // A note given in the new worktree sheet.
    let r = repo.clone();
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_sheet(r, window, cx)));
    h.type_text("fix-b");
    h.keys("tab");
    h.type_text("Try the other approach");
    h.keys("enter");
    let b = repo.parent().unwrap().join("app.worktrees/fix-b");
    h.wait_for("fix-b with its note", {
        let f = note_in_sidebar(b.clone());
        move |v, cx| f(v, cx).as_deref() == Some("Try the other approach")
    });
    assert_eq!(
        note_of(&repo, "fix-b").as_deref(),
        Some("Try the other approach")
    );

    // Rows without notes stay as they were; an empty note removes it.
    let edit = MenuAction::EditNote {
        repo: repo.clone(),
        branch: "feat".into(),
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(edit, window, cx))
    });
    for _ in 0.."Ship the fix".len() {
        h.keys("backspace");
    }
    h.keys("enter");
    assert_eq!(note_of(&repo, "feat"), None);
    assert_eq!(h.read(note_in_sidebar(feat.clone())), None);
}

/// The title bar opens the focused worktree's root in the picked app, and
/// the pick is remembered (#81).
#[gpui::test]
fn title_bar_opens_the_worktree_in_the_picked_app(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "openin", |home| {
        repo = home.repo("app");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        config_with_repo(home, &repo);
    });
    h.wait_prompt();
    h.run(&format!("cd {}/src", repo.display()), "");
    h.wait_for("the shell in src/", |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .and_then(|i| i.cwd.as_ref())
            .is_some_and(|c| c.ends_with("src"))
    });
    h.wait_for("the worktree in the sidebar", {
        let repo = repo.clone();
        move |v, cx| v.sidebar.read(cx).model.worktree_for_path(&repo).is_some()
    });
    let click = |h: &mut Harness, selector: &'static str| {
        h.cx.run_until_parked();
        let bounds =
            h.cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is drawn"));
        h.cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        h.cx.run_until_parked();
    };

    // The buttons sit on the line through the middle of the window buttons.
    let middle = crate::platform::WINDOW_BUTTONS_TOP + crate::platform::WINDOW_BUTTONS_HEIGHT / 2.0;
    for selector in ["open-in-pick", "open-in-go"] {
        let center = h.cx.debug_bounds(selector).unwrap().center().y;
        assert!(
            (f32::from(center) - middle).abs() < 0.6,
            "{selector} centered at {center:?}, window buttons at {middle}"
        );
    }

    // Nothing picked yet: the first installed app, at the worktree root.
    click(&mut h, "open-in-go");
    assert_eq!(
        h.system.0.borrow().opened_in,
        vec![("finder".to_owned(), repo.clone())]
    );

    // Pick VS Code from the picker's menu.
    click(&mut h, "open-in-pick");
    let pick = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .expect("the app menu is open")
            .items
            .iter()
            .find(|(label, _)| label == "VS Code")
            .map(|(_, action)| action.clone())
            .expect("VS Code is offered")
    });
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(pick, window, cx))
    });
    click(&mut h, "open-in-go");
    assert_eq!(
        h.system.0.borrow().opened_in.last(),
        Some(&("vscode".to_owned(), repo.clone()))
    );
    let saved = std::fs::read_to_string(&h.home.config).unwrap();
    assert!(saved.contains("open-in = \"vscode\""), "{saved}");

    // The palette offers every installed app for the same folder.
    let items = h.read(|v, cx| v.palette_items(cx));
    let open_in: Vec<_> = items
        .iter()
        .filter(|i| i.label.starts_with("Open in "))
        .map(|i| (i.label.as_str(), i.detail.clone()))
        .collect();
    let root = repo.to_string_lossy().into_owned();
    assert_eq!(
        open_in,
        [("Open in Finder", root.clone()), ("Open in VS Code", root)]
    );
}

/// Notes follow the focused worktree, while explicit tab names win (#106).
#[gpui::test]
fn title_bar_follows_branch_notes_and_split_focus(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut feat = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "title-note", |home| {
        repo = home.repo("app");
        feat = repo.parent().unwrap().join("feature");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat",
                feat.to_str().unwrap(),
            ],
        );
        git(
            &repo,
            &["config", "branch.main.description", "\n Main task\nDetails"],
        );
        git(
            &repo,
            &["config", "branch.feat.description", "Feature task"],
        );
        config_with_repo(home, &repo);
    });
    h.wait_prompt();
    h.run(&format!("cd {}", repo.display()), "");
    h.wait_for("main note in the title", |v, cx| {
        v.title_bar_text(cx).0 == "app - Main task"
    });
    assert_eq!(
        h.read(|v, cx| v.title_bar_text(cx).1),
        format!(
            "Repository: app\n\n Main task\nDetails\nBranch: main\n{}",
            repo.display()
        )
    );
    assert_eq!(h.read(|v, cx| v.open_in_folder(cx)), Some(repo.clone()));

    h.keys("cmd-d");
    h.wait_prompt();
    h.run(&format!("cd {}", feat.display()), "");
    h.wait_for("split note", |v, cx| {
        v.title_bar_text(cx).0 == "app - Feature task"
    });
    h.keys("cmd-alt-left");
    assert_eq!(h.read(|v, cx| v.title_bar_text(cx).0), "app - Main task");
    h.keys("cmd-alt-right");
    assert_eq!(h.read(|v, cx| v.title_bar_text(cx).0), "app - Feature task");

    h.keys("cmd-t");
    h.wait_prompt();
    h.run(&format!("cd {}", repo.display()), "");
    h.wait_for("second tab note", |v, cx| {
        v.title_bar_text(cx).0 == "app - Main task"
    });
    h.keys("cmd-1");
    assert_eq!(h.read(|v, cx| v.title_bar_text(cx).0), "app - Feature task");

    let edit = MenuAction::EditNote {
        repo: repo.clone(),
        branch: "feat".into(),
    };
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(edit, window, cx))
    });
    for _ in 0.."Feature task".len() {
        h.keys("backspace");
    }
    h.type_text("Updated task");
    h.keys("enter");
    assert_eq!(h.read(|v, cx| v.title_bar_text(cx).0), "app - Updated task");
    git(
        &repo,
        &[
            "config",
            "branch.feat.description",
            "External task\nFull note",
        ],
    );
    h.wait_for("external edit", |v, cx| {
        v.title_bar_text(cx).0 == "app - External task"
    });
    h.cx.run_until_parked();
    assert!(
        h.cx.debug_bounds("title-text:app - External task")
            .is_some(),
        "external notes redraw the title"
    );
    let tab = h.read(|v, _| v.ws.active_tab().unwrap().id);
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.ws.rename_tab(tab, "My task");
            v.focus(window, cx);
        })
    });
    assert_eq!(
        h.read(|v, cx| v.title_bar_text(cx).0),
        "app - External task"
    );
    assert!(
        h.read(|v, cx| v.title_bar_text(cx).1)
            .contains("Full note\nBranch: feat")
    );
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.ws.rename_tab(tab, "");
            v.focus(window, cx);
        })
    });
    git(&repo, &["config", "branch.feat.description", " \n\t"]);
    h.wait_for("whitespace fallback", |v, cx| {
        v.title_bar_text(cx).0 == "app - feat"
    });
    git(&repo, &["config", "--unset", "branch.feat.description"]);
    h.wait_for("removed note", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .worktree_for_path(&feat)
            .unwrap()
            .1
            .note
            .is_none()
    });
    assert_eq!(h.read(|v, cx| v.title_bar_text(cx).0), "app - feat");
}
