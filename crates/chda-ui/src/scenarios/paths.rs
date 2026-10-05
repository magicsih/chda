//! Paths in the output: right-click menus for files and folders, cmd-hover.

use gpui::{Entity, Modifiers, MouseButton, Pixels, Point, point};

use super::harness::Harness;
use crate::terminal_view::{LinkOpen, TerminalView};
use crate::workspace_view::MenuAction;

/// Wait for an output row that `matches` and return the window position of
/// the middle of `needle` in it.
fn find_on_screen(
    h: &mut Harness,
    term: &Entity<TerminalView>,
    matches: impl Fn(&str) -> bool + Copy + 'static,
    needle: &str,
) -> Point<Pixels> {
    h.wait_for("the output row", {
        let term = term.clone();
        move |_, cx| {
            let frame = term.read(cx).frame();
            (0..frame.size.rows).any(|y| matches(&frame.row_text(y)))
        }
    });
    h.read(|_, cx| {
        let t = term.read(cx);
        let frame = t.frame();
        let (row, text) = (0..frame.size.rows)
            .map(|y| (y, frame.row_text(y)))
            .find(|(_, text)| matches(text))
            .unwrap();
        let col = text.rfind(needle).unwrap() + needle.len() / 2;
        let g = t.geometry.unwrap();
        point(
            g.origin.x + g.cell_width * (col as f32 + 0.5),
            g.origin.y + g.line_height * (f32::from(row) + 0.5),
        )
    })
}

fn right_click(h: &mut Harness, at: Point<Pixels>) {
    h.cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    h.cx.run_until_parked();
}

/// The open context menu's action labeled `label`.
fn menu_item(h: &Harness, label: &str) -> MenuAction {
    h.read(|v, _| {
        let menu = v.context_menu.as_ref().expect("a menu is open");
        let labels: Vec<_> = menu.items.iter().map(|(l, _)| l.clone()).collect();
        menu.items
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("no {label:?} in {labels:?}"))
            .1
            .clone()
    })
}

fn choose(h: &mut Harness, label: &str) {
    let action = menu_item(h, label);
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.cx.run_until_parked();
}

#[gpui::test]
fn ls_folder_opens_a_terminal_tab_and_cds_there(cx: &mut gpui::TestAppContext) {
    let mut h = Harness::open(cx, "lsdir", |home| {
        std::fs::create_dir_all(home.home.join("proj/build")).unwrap();
    });
    let build = h.home.home.join("proj/build");
    h.wait_prompt();
    h.run("cd proj; clear; ls -la", "build");
    let term = h.focused_terminal();
    let at = find_on_screen(&mut h, &term, |t| t.ends_with(" build"), "build");
    // Wait for the prompt after the listing, so `cd` is offered.
    h.wait_for("the prompt after ls", {
        let term = term.clone();
        move |_, cx| term.read(cx).frame().at_prompt()
    });
    right_click(&mut h, at);

    choose(&mut h, "Open a terminal tab here");
    h.wait_prompt();
    h.run("pwd", &build.to_string_lossy());
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);

    // Back in the first tab: cd into the folder from the same menu.
    // With two tabs the tab bar shows and the grid moves down.
    h.keys("cmd-1");
    let at = find_on_screen(&mut h, &term, |t| t.ends_with(" build"), "build");
    right_click(&mut h, at);
    choose(&mut h, "cd here in this pane");
    h.run("clear; pwd", &build.to_string_lossy());
}

#[gpui::test]
fn compiler_error_path_reveals_the_file(cx: &mut gpui::TestAppContext) {
    let mut h = Harness::open(cx, "reveal", |home| {
        std::fs::create_dir_all(home.home.join("src")).unwrap();
        std::fs::write(home.home.join("src/main.rs"), "fn main() {}\n").unwrap();
    });
    let main = h.home.home.join("src/main.rs");
    h.wait_prompt();
    h.type_text("clear; echo 'error at src/main.rs:12:5'");
    h.keys("enter");
    let term = h.focused_terminal();
    let at = find_on_screen(&mut h, &term, |t| t.starts_with("error at"), "main");

    // cmd-hover: a file link with its absolute path.
    h.cx.simulate_mouse_move(at, None, Modifiers::command());
    let open = h.read(|_, cx| term.read(cx).hovered_link.clone().map(|l| l.open));
    assert_eq!(
        open,
        Some(LinkOpen::Path {
            path: main.clone(),
            line: Some(12),
            column: Some(5),
            is_dir: false,
        })
    );
    h.cx.simulate_mouse_move(at, None, Modifiers::none());

    right_click(&mut h, at);
    choose(
        &mut h,
        &format!("Reveal in {}", crate::platform::file_manager_name()),
    );
    assert_eq!(h.system.0.borrow().revealed, vec![main.clone()]);

    right_click(&mut h, at);
    choose(&mut h, "Copy relative path");
    let copied = h.read(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("src/main.rs"));
}

#[gpui::test]
fn quoted_path_with_spaces_is_one_link(cx: &mut gpui::TestAppContext) {
    let mut h = Harness::open(cx, "quoted", |home| {
        std::fs::create_dir_all(home.home.join("My Notes")).unwrap();
        std::fs::write(home.home.join("My Notes/todo.md"), "x\n").unwrap();
    });
    let todo = h.home.home.join("My Notes/todo.md");
    h.wait_prompt();
    h.type_text("clear; printf 'saved \"%s\"\\n' \"$HOME/My Notes/todo.md\"");
    h.keys("enter");
    let term = h.focused_terminal();
    let at = find_on_screen(&mut h, &term, |t| t.starts_with("saved"), "Notes");
    right_click(&mut h, at);
    choose(&mut h, "Copy absolute path");
    let copied = h.read(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some(&*todo.to_string_lossy()));
}

/// Drag `paths` from another app and drop them at `at`.
fn drop_files(h: &mut Harness, at: Point<Pixels>, paths: &[std::path::PathBuf]) {
    use gpui::{ExternalPaths, FileDropEvent};
    h.cx.simulate_event(FileDropEvent::Entered {
        position: at,
        paths: ExternalPaths(paths.iter().cloned().collect()),
    });
    eprintln!(
        "DEBUG active after enter: {}",
        h.cx.update(|_, cx| cx.has_active_drag())
    );
    h.cx.simulate_event(FileDropEvent::Pending { position: at });
    h.cx.simulate_event(FileDropEvent::Submit { position: at });
    h.cx.run_until_parked();
}

#[gpui::test]
fn dropped_files_paste_as_quoted_paths(cx: &mut gpui::TestAppContext) {
    let mut h = Harness::open(cx, "dropf", |home| {
        std::fs::write(home.home.join("My File.txt"), "x\n").unwrap();
        std::fs::write(home.home.join("it's.md"), "x\n").unwrap();
    });
    let files = [h.home.home.join("My File.txt"), h.home.home.join("it's.md")];
    h.wait_prompt();
    h.type_text("clear; printf '[%s]\\n' ");
    let at = h.read(|v, cx| {
        let pane = v.ws.focused_pane().unwrap();
        let g = v.panes[&pane].0.read(cx).geometry.unwrap();
        point(
            g.origin.x + g.cell_width * 5.0,
            g.origin.y + g.line_height * 5.0,
        )
    });
    drop_files(&mut h, at, &files);
    h.keys("enter");
    for file in &files {
        let line = format!("[{}]", file.display());
        h.wait_for(&line.clone(), move |v, cx| {
            v.focused_text(cx).lines().any(|l| l == line)
        });
    }
}

#[gpui::test]
fn dropped_folder_on_the_sidebar_becomes_a_repository(cx: &mut gpui::TestAppContext) {
    let mut h = Harness::open(cx, "dropr", |_| {});
    let repo = h.home.repo("dropped");
    let file = h.home.home.join("note.txt");
    std::fs::write(&file, "x\n").unwrap();
    h.wait_prompt();
    // Typing first: drops must still land while the last input was a key.
    h.type_text("x");
    let at = h.read(|v, cx| {
        let pane = v.ws.focused_pane().unwrap();
        let g = v.panes[&pane].0.read(cx).geometry.unwrap();
        point(g.origin.x / 2.0, g.origin.y + gpui::px(200.0))
    });
    drop_files(&mut h, at, &[repo.clone(), file]);
    h.wait_for("the dropped repository in the sidebar", move |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .any(|r| r.path == repo)
    });
    assert_eq!(h.read(|v, cx| v.sidebar.read(cx).model.repos.len()), 1);
}

/// A Markdown file in the output: right-click "Preview Markdown" renders it
/// to a local page and opens that; the palette lists the files in the
/// pane's folder.
#[gpui::test]
fn markdown_path_previews_in_the_browser(cx: &mut gpui::TestAppContext) {
    let mut h = Harness::open(cx, "mdprev", |home| {
        std::fs::create_dir_all(home.home.join("docs")).unwrap();
        std::fs::write(
            home.home.join("docs/notes.md"),
            "# Release notes\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        )
        .unwrap();
    });
    h.wait_prompt();
    h.run("cd docs; clear; ls", "notes.md");
    let term = h.focused_terminal();
    let at = find_on_screen(&mut h, &term, |t| t.contains("notes.md"), "notes");
    right_click(&mut h, at);
    choose(&mut h, "Preview Markdown");
    let page = h
        .system
        .0
        .borrow()
        .opened_files
        .last()
        .cloned()
        .expect("a page was opened");
    assert!(page.extension().is_some_and(|e| e == "html"), "{page:?}");
    let html = std::fs::read_to_string(&page).unwrap();
    assert!(html.contains("<h1>Release notes</h1>") && html.contains("<table>"));

    // The palette offers the folder's Markdown files.
    h.wait_for("the shell to report docs/", |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .and_then(|i| i.cwd.as_ref())
            .is_some_and(|c| c.ends_with("docs"))
    });
    let items = h.read(|v, cx| v.palette_items(cx));
    assert!(
        items
            .iter()
            .any(|i| i.label == "Preview Markdown: notes.md"),
        "{:?}",
        items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );
}

#[gpui::test]
fn wrapped_path_actions_use_the_full_target_instead_of_an_existing_prefix(
    cx: &mut gpui::TestAppContext,
) {
    let mut h = Harness::open(cx, "wrapped-path", |_| {});
    h.wait_prompt();
    let term = h.focused_terminal();
    let cols = h.read(|_, cx| term.read(cx).frame().size.cols as usize);
    let prefix = format!("{}{}", "d/".repeat(cols / 2), if cols % 2 == 1 { "d" } else { "" });
    let printed = format!("{}/{}file.rs", prefix, "e/".repeat(cols / 2));
    let full = h.home.home.join(&printed);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, "example").unwrap();
    assert!(h.home.home.join(&prefix).is_dir());
    h.run(
        &format!("clear; printf '\\n%s\\n' '{printed}:12:5'"),
        "file.rs",
    );
    let segments = h.read(|_, cx| {
        let t = term.read(cx);
        let frame = t.frame();
        let row = (0..frame.size.rows)
            .find(|r| frame.row_text(*r).starts_with("d/d/d/"))
            .unwrap();
        let links = chda_term::links_at(&frame, 3, row);
        assert_eq!(links[0].cells.len(), 3);
        links[0].cells.clone()
    });
    for (row, start, _) in segments {
        let at = h.read(|_, cx| {
            let g = term.read(cx).geometry.unwrap();
            point(
                g.origin.x + g.cell_width * (start as f32 + 1.5),
                g.origin.y + g.line_height * (row as f32 + 0.5),
            )
        });
        right_click(&mut h, at);
        choose(&mut h, "Reveal in Finder");
        assert_eq!(h.system.0.borrow().revealed.last(), Some(&full));
    }
}
