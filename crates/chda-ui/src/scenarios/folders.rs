//! Adding folders: the folder picker, and folders that are not git
//! repositories.

use std::path::{Path, PathBuf};

use chda_core::notifications::Severity;
use gpui::TestAppContext;

use super::harness::Harness;
use crate::platform::FolderPick;
use crate::sidebar_view::SidebarEvent;
use crate::workspace_view::{AddRepo, MenuAction};

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

fn errors(h: &Harness) -> usize {
    h.read(|v, _| {
        v.notifications
            .newest()
            .iter()
            .filter(|e| e.severity == Severity::Error)
            .count()
    })
}

/// Let the next folder picker answer `pick`.
fn script(h: &Harness, pick: FolderPick) {
    h.system.0.borrow_mut().folder_picks.push_back(pick);
}

fn picker_prompts(h: &Harness) -> Vec<String> {
    h.system.0.borrow().folder_prompts.clone()
}

/// Run `echo <word>` in the focused terminal and wait for its output line.
fn echo(h: &mut Harness, word: &str) {
    h.type_text(&format!("echo {word}"));
    h.keys("enter");
    let word = word.to_owned();
    h.wait_for(&format!("{word} printed"), move |v, cx| {
        v.focused_text(cx).lines().any(|line| line.trim() == word)
    });
}

fn wait_idle(h: &mut Harness) {
    h.wait_for("no update operation in flight", |v, _| {
        !v.env.windows.borrow().update.has_operations()
    });
}

/// #181: macOS failed to create the folder picker and chda aborted.
#[gpui::test]
fn add_repository_survives_an_unavailable_picker_and_retries(cx: &mut TestAppContext) {
    let mut existing = PathBuf::new();
    let mut h = Harness::open(cx, "picker", |home| {
        existing = home.repo("existing");
        std::fs::write(
            &home.config,
            format!("repos = [\"{}\"]\n", existing.display()),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for("the saved repository", {
        let existing = existing.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .repos
                .iter()
                .any(|r| r.path == existing)
        }
    });
    let config = std::fs::read(&h.home.config).unwrap();
    let tabs = h.read(|v, _| v.ws.tabs().len());
    let errors_before = errors(&h);

    // The sidebar button while a command is half typed: one error, and
    // nothing else changes.
    script(&h, FolderPick::Unavailable("no open panel".into()));
    h.type_text("echo picker-");
    h.cx.run_until_parked();
    let add = h.cx.debug_bounds("add-repo").expect("the + repo button");
    h.cx.simulate_click(add.center(), gpui::Modifiers::none());
    h.cx.run_until_parked();
    assert_eq!(picker_prompts(&h), ["Add repository"]);
    assert_eq!(errors(&h), errors_before + 1);
    assert_eq!(
        h.read(|v, _| v.notifications.latest().map(str::to_owned))
            .as_deref(),
        Some(
            "Couldn't open the folder picker. Try Add repository again, \
             or drop the folder on the sidebar."
        )
    );
    assert_eq!(std::fs::read(&h.home.config).unwrap(), config);
    assert_eq!(registered(&h), std::slice::from_ref(&existing));
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs);
    wait_idle(&mut h);
    // Typing continues in the same terminal, with the unfinished command.
    h.type_text("survived");
    h.keys("enter");
    h.wait_for("the half-typed command's output", |v, cx| {
        v.focused_text(cx)
            .lines()
            .any(|line| line.trim() == "picker-survived")
    });

    // cmd-shift-o, cancelled: no message.
    script(&h, FolderPick::Cancelled);
    h.keys("cmd-shift-o");
    assert_eq!(picker_prompts(&h).len(), 2);
    assert_eq!(errors(&h), errors_before + 1);
    assert_eq!(std::fs::read(&h.home.config).unwrap(), config);
    wait_idle(&mut h);
    echo(&mut h, "after-cancel");

    // The palette's Add repository, once the picker works again.
    let added = h.home.repo("added");
    script(&h, FolderPick::Chosen(vec![added.clone()]));
    h.keys("cmd-shift-p");
    h.type_text("Add repository");
    h.keys("enter");
    h.wait_for("the picked repository", {
        let added = added.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .repos
                .iter()
                .any(|r| r.path == added)
        }
    });
    assert_eq!(registered(&h), [existing.clone(), added.clone()]);
    echo(&mut h, "after-palette");

    // The menu item's action, with several folders at once.
    let (one, two) = (h.home.repo("one"), h.home.repo("two"));
    script(&h, FolderPick::Chosen(vec![one.clone(), two.clone()]));
    h.cx.dispatch_action(AddRepo);
    h.wait_for("both picked repositories", {
        let (one, two) = (one.clone(), two.clone());
        move |v, cx| {
            let model = &v.sidebar.read(cx).model;
            [&one, &two]
                .iter()
                .all(|p| model.repos.iter().any(|r| &r.path == *p))
        }
    });
    assert_eq!(registered(&h), [existing, added, one.clone(), two.clone()]);
    let saved = std::fs::read_to_string(&h.home.config).unwrap();
    assert!(
        saved.contains(&*one.to_string_lossy()) && saved.contains(&*two.to_string_lossy()),
        "{saved}"
    );
    assert_eq!(picker_prompts(&h), ["Add repository"; 4]);
    assert_eq!(errors(&h), errors_before + 1);
    wait_idle(&mut h);
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
