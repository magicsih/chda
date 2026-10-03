//! Folders that are not git repositories.

use std::path::{Path, PathBuf};

use gpui::TestAppContext;

use super::harness::Harness;
use crate::sidebar_view::SidebarEvent;
use crate::workspace_view::MenuAction;

fn add(h: &mut Harness, folder: &Path) {
    let folder = folder.to_path_buf();
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(SidebarEvent::AddRepos(vec![folder]), window, cx)
        })
    });
    h.cx.run_until_parked();
}

fn is_folder(h: &Harness, path: &Path) -> bool {
    h.read(|v, cx| v.sidebar.read(cx).model.is_folder(path))
}

fn registered(h: &Harness) -> Vec<PathBuf> {
    h.read(|v, _| v.config.repos.clone())
}

#[gpui::test]
fn a_plain_folder_is_added_as_it_is_and_turns_into_a_repository(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "folder", |_| {});
    h.wait_prompt();
    let notes = h.home.home.join("notes");
    std::fs::create_dir_all(&notes).unwrap();

    // Cancel adds nothing.
    add(&mut h, &notes);
    let (message, _) = h.cx.pending_prompt().expect("asks what to do");
    assert_eq!(message, "notes is not a git repository.");
    h.cx.simulate_prompt_answer("Cancel");
    h.cx.run_until_parked();
    assert!(registered(&h).is_empty());

    add(&mut h, &notes);
    h.cx.simulate_prompt_answer("Add as folder");
    h.wait_for("the folder row", {
        let notes = notes.clone();
        move |v, cx| v.sidebar.read(cx).model.is_folder(&notes)
    });
    assert_eq!(registered(&h), std::slice::from_ref(&notes));
    assert!(!notes.join(".git").exists());
    let (rows, palette) = h.read(|v, cx| {
        let model = &v.sidebar.read(cx).model;
        let rows: Vec<_> = model.repos[0]
            .worktrees
            .iter()
            .map(|w| w.path.clone())
            .collect();
        let labels: Vec<String> = v.palette_items(cx).into_iter().map(|i| i.label).collect();
        (rows, labels)
    });
    assert_eq!(rows, std::slice::from_ref(&notes));
    assert!(palette.iter().any(|l| l == "Go to notes"), "{palette:?}");
    assert!(
        !palette.iter().any(|l| l.starts_with("New worktree in")),
        "{palette:?}"
    );

    // Its menu: a terminal, the agents, git init, removal; no git actions.
    let menu = h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                SidebarEvent::WorktreeMenu(
                    notes.clone(),
                    gpui::point(gpui::px(10.), gpui::px(10.)),
                ),
                window,
                cx,
            );
            v.context_menu.as_ref().unwrap().items.clone()
        })
    });
    let labels: Vec<&str> = menu.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels.first(), Some(&"Open terminal"));
    assert!(labels.contains(&"Initialize git"), "{labels:?}");
    assert!(labels.contains(&"Remove from sidebar"), "{labels:?}");
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("worktree") || l.contains("branch")),
        "{labels:?}"
    );

    // `git init` in a terminal: the next refresh shows a repository.
    h.run(
        &format!("cd {} && git init -q -b main", notes.display()),
        "",
    );
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(MenuAction::RefreshRepo(notes.clone()), window, cx)
        })
    });
    h.wait_for("the repository row", {
        let notes = notes.clone();
        move |v, cx| {
            let model = &v.sidebar.read(cx).model;
            !model.is_folder(&notes)
                && model.repos[0]
                    .worktrees
                    .first()
                    .and_then(|w| w.branch.as_deref())
                    == Some("main")
        }
    });
}

#[gpui::test]
fn initialize_git_from_the_question_and_from_the_menu(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "folder-init", |_| {});
    h.wait_prompt();
    let site = h.home.home.join("site");
    let docs = h.home.home.join("docs");
    std::fs::create_dir_all(&site).unwrap();
    std::fs::create_dir_all(&docs).unwrap();

    add(&mut h, &site);
    h.cx.simulate_prompt_answer("Initialize git");
    h.wait_for("site as a repository", {
        let site = site.clone();
        move |v, cx| {
            let model = &v.sidebar.read(cx).model;
            model
                .repos
                .iter()
                .any(|r| r.path == site && !r.folder && !r.worktrees.is_empty())
        }
    });
    assert!(site.join(".git").is_dir());

    add(&mut h, &docs);
    h.cx.simulate_prompt_answer("Add as folder");
    h.wait_for("docs as a folder", {
        let docs = docs.clone();
        move |v, cx| v.sidebar.read(cx).model.is_folder(&docs)
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(MenuAction::InitGit(docs.clone()), window, cx)
        })
    });
    h.wait_for("docs as a repository", {
        let docs = docs.clone();
        move |v, cx| {
            let model = &v.sidebar.read(cx).model;
            model
                .repos
                .iter()
                .any(|r| r.path == docs && !r.folder && !r.worktrees.is_empty())
        }
    });
    assert!(!is_folder(&h, &docs));
    assert_eq!(registered(&h), [site, docs]);
}

#[gpui::test]
fn a_plain_folder_comes_back_after_a_restart_with_its_sessions(cx: &mut TestAppContext) {
    let mut notes = PathBuf::new();
    let mut h = Harness::open(cx, "folder-restart", |home| {
        notes = home.home.join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", notes.display())).unwrap();
        home.claude_session(&notes, "s-notes", "summarize these notes");
    });
    h.wait_prompt();
    h.wait_for("the folder with its session", {
        let notes = notes.clone();
        move |v, cx| {
            let model = &v.sidebar.read(cx).model;
            model.is_folder(&notes)
                && model.repos[0].worktrees[0]
                    .sessions
                    .iter()
                    .any(|s| s.snippet == "summarize these notes")
        }
    });
    assert!(is_folder(&h, &notes));
}
