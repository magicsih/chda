//! Preparation is an explicit gate between checkout and opening an agent or shell.
use super::harness::Harness;
use crate::workspace_view::{MenuAction, NewTab, preparation::PreparationStage};
use std::path::{Path, PathBuf};

fn click(h: &mut Harness, selector: &'static str) {
    h.cx.run_until_parked();
    let bounds =
        h.cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("Missing preparation control {selector}"));
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    h.cx.run_until_parked();
}
fn show(h: &mut Harness, path: &Path) {
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(MenuAction::PreparationRun(path.into()), window, cx)
        })
    });
}
fn complete(h: &mut Harness, path: &Path) {
    h.wait_for("preparation complete", |v, _| {
        v.preparation_jobs
            .get(path)
            .is_some_and(|j| matches!(j.stage, PreparationStage::Complete))
    });
}
fn fixture(
    cx: &mut TestAppContext,
    name: &str,
    commands: Vec<String>,
    files: Vec<PathBuf>,
    default_action: chda_config::DefaultAction,
) -> (Harness, PathBuf, PathBuf) {
    let mut repo = PathBuf::new();
    let mut h = Harness::open(cx, name, |home| {
        repo = home.repo("app");
        home.fake_claude();
        std::fs::write(repo.join(".gitignore"), "local/\n").unwrap();
        super::harness::git(&repo, &["add", ".gitignore"]);
        super::harness::git(&repo, &["commit", "-q", "-m", "ignore local files"]);
        std::fs::create_dir(repo.join("local")).unwrap();
        std::fs::write(repo.join("local/한글 space.env"), "synthetic fixture only").unwrap();
        let mut config = chda_config::ChdaConfig::default();
        config.repos.push(repo.clone());
        config.default_action = default_action;
        config.repo_preparation.insert(
            repo.clone(),
            chda_config::PreparationPlan { files, commands },
        );
        config.save(&home.config).unwrap();
    });
    h.wait_prompt();
    let r = repo.clone();
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_sheet(r, window, cx)));
    h.type_text("prepared");
    h.keys("enter");
    let path = repo.parent().unwrap().join("app.worktrees/prepared");
    h.wait_for("exact preparation review", |v, _| {
        v.preparation_jobs
            .get(&path)
            .is_some_and(|j| matches!(j.stage, PreparationStage::Review))
    });
    (h, repo, path)
}
use gpui::TestAppContext;

#[gpui::test]
fn imported_setup_does_not_execute_or_open_a_tab_without_confirmation(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "prep-import", |home| {
        repo = home.repo("app");
        let mut config = chda_config::ChdaConfig::default();
        config.repos.push(repo.clone());
        config.repo_preparation.insert(
            repo.clone(),
            chda_config::PreparationPlan {
                files: vec![],
                commands: vec!["printf imported > ran".into()],
            },
        );
        config.save(&home.config).unwrap();
    });
    h.wait_prompt();
    let r = repo.clone();
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_sheet(r, window, cx)));
    h.type_text("prepared");
    h.keys("enter");
    let path = repo.parent().unwrap().join("app.worktrees/prepared");
    h.wait_for("checkout completion", |v, cx| {
        v.sidebar.read(cx).model.worktree_for_path(&path).is_some()
    });
    assert!(path.is_dir());
    assert!(!path.join("ran").exists());
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        1,
        "an imported preparation plan must be confirmed before opening the worktree"
    );
    h.wait_for("preparation review", |v, _| {
        v.preparation_jobs
            .get(&path)
            .is_some_and(|j| matches!(j.stage, PreparationStage::Review))
    });
    click(&mut h, "preparation-start");
    complete(&mut h, &path);
    assert_eq!(
        std::fs::read_to_string(path.join("ran")).unwrap(),
        "imported"
    );
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        1,
        "background success leaves focus and tabs alone"
    );
    click(&mut h, "preparation-open");
    h.wait_prompt();
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
}

#[gpui::test]
fn preparation_failure_retry_keeps_edits_and_requires_changed_plan_confirmation(
    cx: &mut TestAppContext,
) {
    let (mut h, repo, path) = fixture(
        cx,
        "prep-retry",
        vec!["test -f 'local/한글 space.env'; printf synthetic-private-output; exit 7".into()],
        vec!["local/한글 space.env".into()],
        chda_config::DefaultAction::Terminal,
    );
    click(&mut h, "preparation-start");
    h.wait_for("setup failure", |v, _| {
        v.preparation_jobs.get(&path).is_some_and(
            |j| matches!(&j.stage, PreparationStage::Failed(e) if e.contains("exit 7")),
        )
    });
    assert!(path.join("local/한글 space.env").exists());
    std::fs::write(path.join("local/한글 space.env"), "user edit").unwrap();
    let saved = chda_config::ChdaConfig::load(&h.home.config).unwrap();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(MenuAction::EditPreparation(repo.clone()), window, cx)
        })
    });
    click(&mut h, "preparation-command-0");
    h.keys("cmd-a");
    h.type_text("printf retried > prepared-marker");
    click(&mut h, "preparation-save");
    h.wait_for("changed settings saved", |v, _| {
        v.config.repo_preparation[&repo].commands == vec!["printf retried > prepared-marker"]
    });
    assert_eq!(saved.repo_preparation[&repo].commands.len(), 1);
    show(&mut h, &path);
    click(&mut h, "preparation-retry");
    h.wait_for("retry must be reviewed", |v, _| {
        v.preparation_jobs
            .get(&path)
            .is_some_and(|j| matches!(j.stage, PreparationStage::Review))
    });
    assert!(!path.join("prepared-marker").exists());
    click(&mut h, "preparation-start");
    complete(&mut h, &path);
    assert_eq!(
        std::fs::read_to_string(path.join("local/한글 space.env")).unwrap(),
        "user edit"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("local/한글 space.env")).unwrap(),
        "synthetic fixture only"
    );
    assert!(path.join("prepared-marker").exists());
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
    assert!(
        h.system
            .0
            .borrow()
            .notifications
            .iter()
            .all(|(_, body, _)| !body.contains("synthetic-private-output"))
    );
}

#[gpui::test]
fn cancel_blocks_opening_until_explicit_skip_and_keeps_launch_policy_confirmation(
    cx: &mut TestAppContext,
) {
    let (mut h, _, path) = fixture(
        cx,
        "prep-cancel",
        vec!["sleep 30; printf wrong > later".into()],
        vec![],
        chda_config::DefaultAction::Claude,
    );
    click(&mut h, "preparation-start");
    h.wait_for("setup command running", |v, _| {
        v.preparation_jobs.get(&path).is_some_and(|j| {
            matches!(
                j.stage,
                PreparationStage::Running(chda_core::PreparationProgress::Command { .. })
            )
        })
    });
    let p = path.clone();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(
                MenuAction::RunAgent(p, chda_core::agents::AgentId::Claude),
                window,
                cx,
            )
        })
    });
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
    click(&mut h, "preparation-cancel");
    h.wait_for("owned setup cancelled", |v, _| {
        v.preparation_jobs
            .get(&path)
            .is_some_and(|j| matches!(j.stage, PreparationStage::Cancelled))
    });
    assert!(path.is_dir() && !path.join("later").exists());
    click(&mut h, "preparation-skip");
    h.confirm_agent_launch();
    h.wait_for("explicit inherited Claude launch", |v, cx| {
        v.focused_text(cx).contains("fake-claude")
    });
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
    assert!(h.read(|v, _| v.preparation_jobs.is_empty()));
}

#[gpui::test]
fn setup_completion_preserves_other_tab_and_unfinished_input(cx: &mut TestAppContext) {
    let (mut h, _, path) = fixture(
        cx,
        "prep-focus",
        vec!["sleep 0.3; printf done > done".into()],
        vec![],
        chda_config::DefaultAction::Terminal,
    );
    click(&mut h, "preparation-start");
    click(&mut h, "preparation-hide");
    h.cx.dispatch_action(NewTab);
    h.wait_prompt();
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.type_text("unfinished-other-pane");
    h.wait_for("unfinished input echoed in the other pane", |v, cx| {
        v.focused_text(cx).contains("unfinished-other-pane")
    });
    complete(&mut h, &path);
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), Some(pane));
    assert!(
        h.read(|v, cx| v.focused_text(cx))
            .contains("unfinished-other-pane")
    );
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
    assert!(path.join("done").exists());
}
#[gpui::test]
fn closing_the_owner_window_cancels_preparation_even_if_its_view_is_retained(
    cx: &mut TestAppContext,
) {
    let (mut h, _, path) = fixture(
        cx,
        "prep-close",
        vec!["sleep 30; touch orphaned".into()],
        vec![],
        chda_config::DefaultAction::Terminal,
    );
    click(&mut h, "preparation-start");
    h.wait_for("owned setup running", |v, _| {
        v.preparation_jobs.get(&path).is_some_and(|j| {
            matches!(
                j.stage,
                PreparationStage::Running(chda_core::PreparationProgress::Command { .. })
            )
        })
    });
    h.cx.update(|window, _| window.remove_window());
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v.preparation_jobs.is_empty()),
        "closed windows must cancel workers without waiting for the view's final reference to drop"
    );
    assert!(!path.join("orphaned").exists());
}
#[gpui::test]
fn another_window_cannot_launch_an_agent_in_an_unconfirmed_worktree(cx: &mut TestAppContext) {
    let (mut h, _, path) = fixture(
        cx,
        "prep-windows",
        vec!["touch prepared".into()],
        vec![],
        chda_config::DefaultAction::Terminal,
    );
    let env = h.read(|v, _| v.env.clone());
    let handle = h.cx.update(|_, cx| {
        crate::open_workspace_window(chda_config::load(&env.ghostty, None), None, env.clone(), cx)
    });
    let mut other_cx = gpui::VisualTestContext::from_window(handle.into(), cx);
    let other = handle.root(&mut other_cx).unwrap();
    super::harness::wait_until(&mut other_cx, &other, "other shell", |v, cx| {
        v.focused_text(cx).contains("test%")
    });
    other_cx.update(|window, cx| {
        other.update(cx, |v, cx| {
            v.run_menu_action(
                MenuAction::RunAgent(path.clone(), chda_core::agents::AgentId::Claude),
                window,
                cx,
            )
        })
    });
    assert_eq!(other.read_with(&other_cx, |v, _| v.ws.tabs().len()), 1);
    assert!(other.read_with(&other_cx, |v, _| {
        v.notifications
            .latest()
            .is_some_and(|s| s.contains("original window"))
    }));
    assert!(!path.join("prepared").exists());
    show(&mut h, &path);
    click(&mut h, "preparation-start");
    complete(&mut h, &path);
    click(&mut h, "preparation-finish");
    other_cx.update(|window, cx| {
        other.update(cx, |v, cx| {
            v.run_menu_action(
                MenuAction::RunAgent(path, chda_core::agents::AgentId::Claude),
                window,
                cx,
            )
        })
    });
    other_cx.run_until_parked();
    assert!(other_cx.debug_bounds("launch-start").is_some());
}

#[gpui::test]
fn mcp_preparation_and_open_requests_cannot_bypass_user_confirmation(cx: &mut TestAppContext) {
    use chda_core::agents::control::Request;
    let mut repo = PathBuf::new();
    let mut h = Harness::open(cx, "prep-mcp", |home| {
        repo = home.repo("app");
        let mut config = chda_config::ChdaConfig::default();
        config.repos.push(repo.clone());
        config.repo_preparation.insert(
            repo.clone(),
            chda_config::PreparationPlan {
                files: vec![],
                commands: vec!["printf mcp > marker".into()],
            },
        );
        config.save(&home.config).unwrap();
    });
    h.wait_prompt();
    let reply = super::mcp::ask(
        &mut h,
        Request::CreateWorktree {
            cwd: repo,
            branch: Some("mcp-prep".into()),
            base: None,
            note: None,
            open: false,
            agent: None,
        },
    );
    assert!(reply.ok && reply.message.contains("review preparation"));
    let path = PathBuf::from(reply.data["path"].as_str().unwrap());
    h.wait_for("MCP preparation review", |v, _| {
        v.preparation_jobs
            .get(&path)
            .is_some_and(|j| matches!(j.stage, PreparationStage::Review))
    });
    let reply = super::mcp::ask(
        &mut h,
        Request::OpenTab {
            path: path.clone(),
            agent: None,
        },
    );
    assert!(reply.ok && reply.data["preparation_pending"] == true);
    assert!(!path.join("marker").exists());
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
    show(&mut h, &path);
    click(&mut h, "preparation-start");
    complete(&mut h, &path);
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
    click(&mut h, "preparation-finish");
    let reply = super::mcp::ask(&mut h, Request::OpenTab { path, agent: None });
    assert!(reply.ok);
    h.wait_prompt();
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
}
