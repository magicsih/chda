//! Adding folders: the folder picker, folders that are not git repositories,
//! and where the sidebar's selection goes afterwards.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chda_core::notifications::Severity;
use gpui::{ExternalPaths, FileDropEvent, TestAppContext, point, px, size};

use super::harness::{Harness, git};
use super::sidebar::draw_sidebar_frame;
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

    // Close the inspected menu before sending a shell command.
    h.keys("escape");
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

/// A window whose PROJECT overflows: `app` with 24 more worktrees.
fn crowded_app(home: &super::harness::Home) -> PathBuf {
    let app = home.repo("app");
    for index in 0..24 {
        let branch = format!("task-{index:02}");
        let path = home.home.join(&branch);
        git(
            &app,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                &branch,
                path.to_str().unwrap(),
            ],
        );
    }
    app
}

fn selected(h: &Harness) -> Option<PathBuf> {
    h.read(|v, cx| v.sidebar.read(cx).selected.clone())
}

fn row_bounds(h: &mut Harness, path: &Path) -> Option<gpui::Bounds<gpui::Pixels>> {
    h.cx.debug_bounds(Box::leak(format!("wt:{}", path.display()).into_boxed_str()))
}

/// The whole row is inside PROJECT's visible part.
fn in_view(h: &mut Harness, path: &Path) -> bool {
    let viewport = h.read(|v, cx| v.sidebar.read(cx).project_scroll.bounds());
    row_bounds(h, path)
        .is_some_and(|row| row.top() >= viewport.top() && row.bottom() <= viewport.bottom())
}

/// Draw frames until PROJECT shows the row: a new folder's worktrees load in
/// the background, and the reveal scrolls on a frame.
fn wait_in_view(h: &mut Harness, path: &Path) {
    for _ in 0..250 {
        draw_sidebar_frame(h);
        if in_view(h, path) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (viewport, offset, max) = h.read(|v, cx| {
        let scroll = &v.sidebar.read(cx).project_scroll;
        (scroll.bounds(), scroll.offset(), scroll.max_offset())
    });
    panic!(
        "{} never came into view: row {:?}, PROJECT {viewport:?}, offset {offset:?}, max {max:?}",
        path.display(),
        row_bounds(h, path)
    );
}

fn open_crowded(cx: &mut TestAppContext, name: &str, extra: &mut [PathBuf]) -> (Harness, PathBuf) {
    let mut app = PathBuf::new();
    let mut h = Harness::open(cx, name, |home| {
        app = crowded_app(home);
        for (index, folder) in extra.iter_mut().enumerate() {
            *folder = home.repo(&format!("added-{index}"));
        }
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", app.display())).unwrap();
    });
    h.wait_prompt();
    h.cx.simulate_resize(size(px(960.0), px(480.0)));
    h.wait_for("all worktrees", |v, cx| {
        v.sidebar.read(cx).model.repos[0].worktrees.len() == 25
    });
    // The terminal works in `app`, so it is the current selection.
    h.run(&format!("cd '{}'", app.display()), "test%");
    h.wait_for("app selected", {
        let app = app.clone();
        move |v, cx| v.sidebar.read(cx).selected.as_ref() == Some(&app)
    });
    (h, app)
}

/// #264: the folder just added, from the picker or by a drop, becomes the
/// selected row and is in view at once, even far below PROJECT's visible part.
#[gpui::test]
fn an_added_folder_is_selected_and_in_view_at_once(cx: &mut TestAppContext) {
    let mut added = vec![PathBuf::new(); 3];
    let (mut h, app) = open_crowded(cx, "added-selected", &mut added);
    let (first, second, dropped) = (added[0].clone(), added[1].clone(), added[2].clone());
    let before = h.read(|v, _| {
        let pane = v.ws.focused_pane();
        (
            v.ws.tabs().len(),
            pane,
            pane.and_then(|p| v.ws.pane(p)).and_then(|p| p.cwd.clone()),
        )
    });

    // The picker with two folders: the one added last is selected.
    script(&h, FolderPick::Chosen(vec![first.clone(), second.clone()]));
    h.cx.dispatch_action(AddRepo);
    h.wait_for("the last folder selected", {
        let second = second.clone();
        move |v, cx| v.sidebar.read(cx).selected.as_ref() == Some(&second)
    });
    wait_in_view(&mut h, &second);
    assert_eq!(selected(&h), Some(second.clone()));
    assert_eq!(registered(&h), [app.clone(), first, second.clone()]);

    // Opening nothing: the same tab and pane keep working in `app`.
    let after = h.read(|v, _| {
        let pane = v.ws.focused_pane();
        (
            v.ws.tabs().len(),
            pane,
            pane.and_then(|p| v.ws.pane(p)).and_then(|p| p.cwd.clone()),
        )
    });
    assert_eq!(after, before);

    // A background refresh keeps the selection instead of returning to `app`.
    let extra = h.home.home.join("extra");
    git(
        &second,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "extra",
            extra.to_str().unwrap(),
        ],
    );
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.run_menu_action(MenuAction::RefreshRepo(second.clone()), window, cx)
        })
    });
    h.wait_for("the refreshed worktrees", {
        let second = second.clone();
        move |v, cx| {
            let model = &v.sidebar.read(cx).model;
            model
                .repos
                .iter()
                .any(|r| r.path == second && r.worktrees.len() == 2)
        }
    });
    wait_in_view(&mut h, &second);
    assert_eq!(selected(&h), Some(second));

    // A folder dropped on the sidebar leads to the same place.
    let at = h.read(|v, cx| v.sidebar.read(cx).project_scroll.bounds().center());
    h.cx.simulate_event(FileDropEvent::Entered {
        position: at,
        paths: ExternalPaths([dropped.clone()].into_iter().collect()),
    });
    h.cx.simulate_event(FileDropEvent::Pending { position: at });
    h.cx.simulate_event(FileDropEvent::Submit { position: at });
    h.cx.run_until_parked();
    h.wait_for("the dropped folder selected", {
        let dropped = dropped.clone();
        move |v, cx| v.sidebar.read(cx).selected.as_ref() == Some(&dropped)
    });
    wait_in_view(&mut h, &dropped);
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), before.0);
}

/// #264: a collapsed PROJECT section and a collapsed group open for the folder
/// the user adds, including one the sidebar already lists, and stay open.
#[gpui::test]
fn adding_a_folder_opens_a_collapsed_project_and_group(cx: &mut TestAppContext) {
    let (mut shelf, mut other) = (PathBuf::new(), PathBuf::new());
    let mut h = Harness::open(cx, "added-collapsed", |home| {
        shelf = home.repo("shelf");
        other = home.repo("other");
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\", \"{}\"]\nproject-collapsed = true\ncollapsed-repos = [\"{}\"]\n",
                shelf.display(),
                other.display(),
                shelf.display()
            ),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for("both repositories", |v, cx| {
        v.sidebar.read(cx).model.repos.len() == 2
    });
    h.read(|v, cx| {
        let sidebar = v.sidebar.read(cx);
        assert!(v.config.project_collapsed && sidebar.project_collapsed);
        assert!(sidebar.model.repos[0].collapsed, "shelf starts collapsed");
    });

    // `shelf` is listed already; adding it again leads to its row all the same.
    script(&h, FolderPick::Chosen(vec![shelf.clone()]));
    h.cx.dispatch_action(AddRepo);
    h.wait_for("shelf selected", {
        let shelf = shelf.clone();
        move |v, cx| v.sidebar.read(cx).selected.as_ref() == Some(&shelf)
    });
    wait_in_view(&mut h, &shelf);
    h.read(|v, cx| {
        let sidebar = v.sidebar.read(cx);
        assert!(!v.config.project_collapsed && !sidebar.project_collapsed);
        assert!(!sidebar.model.repos[0].collapsed);
    });
    let saved = chda_config::ChdaConfig::load(&h.home.config).unwrap();
    assert!(!saved.project_collapsed && saved.collapsed_repos.is_empty());
    assert_eq!(registered(&h), [shelf, other]);
}

/// #264: cancelling the picker, a picker that cannot open and a plain folder
/// the user declines leave the selection and PROJECT's scroll position alone.
#[gpui::test]
fn a_cancelled_or_failed_add_keeps_the_selection_and_scroll(cx: &mut TestAppContext) {
    let (mut h, app) = open_crowded(cx, "added-cancelled", &mut []);
    let scroll = h.read(|v, cx| v.sidebar.read(cx).project_scroll.clone());
    scroll.set_offset(point(px(0.0), px(-120.0)));
    for _ in 0..3 {
        draw_sidebar_frame(&mut h);
    }
    let offset = scroll.offset();
    assert!(offset.y < px(0.0), "PROJECT starts scrolled down");

    for pick in [
        FolderPick::Cancelled,
        FolderPick::Unavailable("no open panel".into()),
    ] {
        script(&h, pick);
        h.cx.dispatch_action(AddRepo);
        h.cx.run_until_parked();
        wait_idle(&mut h);
    }
    let notes = h.home.home.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    add(&mut h, &notes);
    h.cx.pending_prompt().expect("asks what to do");
    h.cx.simulate_prompt_answer("Cancel");
    h.cx.run_until_parked();
    wait_idle(&mut h);
    for _ in 0..3 {
        draw_sidebar_frame(&mut h);
    }

    assert_eq!(selected(&h), Some(app.clone()));
    assert_eq!(scroll.offset(), offset);
    assert_eq!(registered(&h), [app]);
}
