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
    let index = h.read(|v, _| {
        v.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|(text, _)| text == label)
            .unwrap()
    });
    h.cx.run_until_parked();
    let selector = Box::leak(format!("menu-item-{index}").into_boxed_str());
    let bounds = h.cx.debug_bounds(selector).expect("menu item bounds");
    h.cx.simulate_click(bounds.center(), Modifiers::none());
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
    h.cx.simulate_mouse_move(at, None, crate::platform::link_modifiers());
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
fn parenthesized_space_paths_hover_open_and_reveal_across_wraps(cx: &mut gpui::TestAppContext) {
    let mut h = Harness::open(cx, "parenthesized-paths", |_| {});
    h.wait_prompt();
    let term = h.focused_terminal();
    let cols = h.read(|_, cx| term.read(cx).frame().size.cols as usize);
    // An existing shorter prefix of every fixture path.
    std::fs::create_dir_all(h.home.home.join("Notes")).unwrap();
    for (name, prose, suffix, line, column) in [
        ("workflow.html", "", "", None, None),
        ("작업 문서/설계안.md", "", "", None, None),
        ("작업 문서/details.md", "see ", ":5:2", Some(5), Some(2)),
    ] {
        let full = h
            .home
            .home
            .join(format!("Notes Folder/{}{name}", "d/".repeat(cols / 2)));
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, "example").unwrap();
        let expected = LinkOpen::Path {
            path: full.clone(),
            line,
            column,
            is_dir: false,
        };
        h.run(
            &format!(
                "clear; printf '\\nPlan ({prose}%s{suffix})\\n' '{}'",
                full.display()
            ),
            name,
        );
        h.wait_for("the parenthesized output row after clearing the command", {
            let term = term.clone();
            move |_, cx| {
                let frame = term.read(cx).frame();
                let rows = || (0..frame.size.rows).map(|y| frame.row_text(y));
                rows().any(|t| t.starts_with(&format!("Plan ({prose}")))
                    && !rows().any(|t| t.contains("printf"))
            }
        });
        let row = h.read(|_, cx| {
            let frame = term.read(cx).frame();
            (0..frame.size.rows)
                .find(|y| frame.row_text(*y).starts_with("Plan ("))
                .unwrap()
        });
        let hover = |h: &mut Harness, row: u16, col: u16| {
            let at = h.read(|_, cx| {
                let g = term.read(cx).geometry.unwrap();
                point(
                    g.origin.x + g.cell_width * (f32::from(col) + 0.5),
                    g.origin.y + g.line_height * (f32::from(row) + 0.5),
                )
            });
            h.cx.simulate_mouse_move(at, None, crate::platform::link_modifiers());
            (at, h.read(|_, cx| term.read(cx).hovered_link.clone()))
        };
        let path_start = ("Plan (".len() + prose.len()) as u16;
        let link = hover(&mut h, row, path_start)
            .1
            .expect("a link at the start of the path");
        assert_eq!(link.open, expected);
        assert_eq!(link.cells[0], (row, path_start, link.cells[0].2));
        assert!(link.cells.len() > 1, "fixture must soft-wrap");
        for col in "Plan ".len() as u16..path_start {
            assert_eq!(
                hover(&mut h, row, col).1,
                None,
                "the parenthesis and prose at {col} are not part of the link"
            );
        }
        for (row, start, end) in link.cells.clone() {
            for col in [start, (start + end) / 2, end] {
                let (at, hovered) = hover(&mut h, row, col);
                assert_eq!(hovered.as_ref(), Some(&link), "({col}, {row})");
                h.cx.simulate_click(at, crate::platform::link_modifiers());
                h.cx.run_until_parked();
                assert_eq!(h.system.0.borrow().opened_files.last(), Some(&full));
                h.cx.simulate_mouse_move(at, None, Modifiers::none());
                right_click(&mut h, at);
                assert!(
                    matches!(
                        menu_item(&h, "Open"),
                        MenuAction::OpenPath { path, line: l, column: c }
                            if path == full && l == line && c == column
                    ),
                    "the menu opens the same target at ({col}, {row})"
                );
                choose(&mut h, "Copy absolute path");
                assert_eq!(
                    h.read(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
                        .as_deref(),
                    Some(&*full.to_string_lossy())
                );
            }
        }
    }
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
    // Resolving a relative path requires the shell's cwd report. Output can
    // arrive before the following prompt/OSC 7 notification.
    h.wait_for("the shell to report docs/", |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .and_then(|i| i.cwd.as_ref())
            .is_some_and(|c| c.ends_with("docs"))
    });
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
    let prefix = format!(
        "{}{}",
        "d/".repeat(cols / 2),
        if cols % 2 == 1 { "d" } else { "" }
    );
    let printed = format!("{}/{}file.rs", prefix, "e/".repeat(cols / 2));
    let full = h.home.home.join(&printed);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, "example").unwrap();
    assert!(h.home.home.join(&prefix).is_dir());
    h.run(
        &format!("clear; printf '\\n%s\\n' '{printed}:12:5'"),
        "file.rs",
    );
    h.wait_for("the printed wrapped path after clearing the command", {
        let term = term.clone();
        move |_, cx| {
            let frame = term.read(cx).frame();
            (0..frame.size.rows).any(|r| frame.row_text(r).starts_with("d/d/d/"))
        }
    });
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
        choose(
            &mut h,
            &format!("Reveal in {}", crate::platform::file_manager_name()),
        );
        assert_eq!(h.system.0.borrow().revealed.last(), Some(&full));
    }
}

#[gpui::test]
fn bare_paths_share_complete_hover_click_and_menu_targets_across_soft_wraps(
    cx: &mut gpui::TestAppContext,
) {
    let mut h = Harness::open(cx, "bare-paths", |_| {});
    h.wait_prompt();
    let term = h.focused_terminal();
    h.wait_for("the terminal frame to match its laid-out grid", {
        let term = term.clone();
        move |_, cx| {
            let t = term.read(cx);
            t.geometry.is_some_and(|g| t.frame().size == g.size)
        }
    });
    let cols = h.read(|_, cx| term.read(cx).frame().size.cols as usize);
    std::fs::write(h.home.home.join("Notes"), "short prefix").unwrap();
    for (directory, name, escaped, line, column) in [
        ("Notes Folder", "file.txt", false, Some(3), Some(2)),
        ("./Notes Folder", "relative.txt", false, Some(2), Some(5)),
        ("(draft dir)", "f.txt", false, Some(3), None),
        ("Notes (draft)", "file.txt", true, None, None),
        ("작업 문서", "설계 (초안).md", false, Some(2), Some(5)),
    ] {
        let full = h
            .home
            .home
            .join(directory)
            .join("d/".repeat(cols / 2))
            .join(name);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, "fixture").unwrap();
        let printed_path = if directory.starts_with("./") {
            format!("./{}", full.strip_prefix(&h.home.home).unwrap().display())
        } else {
            full.to_string_lossy().into_owned()
        };
        let printed = if escaped {
            printed_path.replace(' ', "\\ ")
        } else {
            printed_path
        };
        let suffix = line
            .map(|line| {
                column.map_or_else(|| format!(":{line}"), |column| format!(":{line}:{column}"))
            })
            .unwrap_or_default();
        h.run(
            &format!(
                "clear; printf '\\nBare %s{suffix}\\n' {}",
                chda_term::shell_quote(&printed)
            ),
            name,
        );
        h.wait_for("bare path output without echoed command", {
            let term = term.clone();
            move |_, cx| {
                let frame = term.read(cx).frame();
                (0..frame.size.rows).any(|row| frame.row_text(row).starts_with("Bare "))
                    && !(0..frame.size.rows).any(|row| frame.row_text(row).contains("printf"))
            }
        });
        let row = h.read(|_, cx| {
            let frame = term.read(cx).frame();
            (0..frame.size.rows)
                .find(|row| frame.row_text(*row).starts_with("Bare "))
                .unwrap()
        });
        let position = |h: &Harness, row: u16, col: u16| {
            h.read(|_, cx| {
                let g = term.read(cx).geometry.unwrap();
                point(
                    g.origin.x + g.cell_width * (f32::from(col) + 0.5),
                    g.origin.y + g.line_height * (f32::from(row) + 0.5),
                )
            })
        };
        let at = position(&h, row, 5);
        h.cx.simulate_mouse_move(at, None, crate::platform::link_modifiers());
        let link = h
            .read(|_, cx| term.read(cx).hovered_link.clone())
            .expect("full bare path");
        assert_eq!(
            link.open,
            LinkOpen::Path {
                path: full.clone(),
                line,
                column,
                is_dir: false
            }
        );
        assert!(link.cells.len() > 1, "fixture must wrap");
        for (row, start, end) in link.cells.clone() {
            for col in [start, (start + end) / 2, end] {
                let at = position(&h, row, col);
                h.cx.simulate_mouse_move(at, None, crate::platform::link_modifiers());
                assert_eq!(
                    h.read(|_, cx| term.read(cx).hovered_link.clone()),
                    Some(link.clone())
                );
                h.cx.simulate_click(at, crate::platform::link_modifiers());
                h.cx.run_until_parked();
                assert_eq!(h.system.0.borrow().opened_files.last(), Some(&full));
                right_click(&mut h, at);
                assert!(
                    matches!(menu_item(&h,"Open"),MenuAction::OpenPath{path,line:l,column:c} if path==full && l==line && c==column)
                );
                choose(&mut h, "Copy absolute path");
                assert_eq!(
                    h.read(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
                        .as_deref(),
                    Some(&*full.to_string_lossy())
                );
            }
        }
        let (row, start, _) = link.cells[0];
        let at = position(&h, row, start);
        h.cx.run_until_parked();
        h.cx.simulate_mouse_move(at, None, crate::platform::link_modifiers());
        h.cx.run_until_parked();
        assert_eq!(
            h.read(|_, cx| term.read(cx).hovered_link.clone())
                .map(|link| link.open),
            Some(link.open.clone())
        );
        std::fs::remove_file(&full).unwrap();
        let count = h.system.0.borrow().opened_files.len();
        for _ in 0..2 {
            h.cx.simulate_click(at, crate::platform::link_modifiers());
            h.cx.run_until_parked();
            assert_eq!(
                h.system.0.borrow().opened_files.len(),
                count,
                "deleted path changed to a prefix or was opened"
            );
            assert!(
                h.read(|_, cx| term.read(cx).hovered_link.is_none()),
                "rejected target must lose its underline and path hint"
            );
        }
    }
}

#[gpui::test]
fn missing_path_cache_does_not_hide_an_adjacent_legacy_file_or_a_new_file(
    cx: &mut gpui::TestAppContext,
) {
    let mut h = Harness::open(cx, "path-cache", |home| {
        std::fs::write(home.home.join("README.md"), "fixture").unwrap();
    });
    h.wait_prompt();
    let missing = h.home.home.join("not-yet-created");
    h.run(
        "clear; printf 'Cache ./not-yet-created README.md\\n'",
        "Cache",
    );
    h.wait_prompt();
    let term = h.focused_terminal();
    let first = find_on_screen(
        &mut h,
        &term,
        |text| text.starts_with("Cache "),
        "not-yet-created",
    );
    h.cx.simulate_mouse_move(first, None, crate::platform::link_modifiers());
    assert!(h.read(|_, cx| term.read(cx).hovered_link.is_none()));
    let second = find_on_screen(
        &mut h,
        &term,
        |text| text.starts_with("Cache "),
        "README.md",
    );
    h.cx.simulate_mouse_move(second, None, crate::platform::link_modifiers());
    assert!(
        matches!(h.read(|_,cx|term.read(cx).hovered_link.clone()).map(|link|link.open),Some(LinkOpen::Path{path,..}) if path==h.home.home.join("README.md"))
    );
    h.cx.simulate_mouse_move(first, None, crate::platform::link_modifiers());
    assert!(h.read(|_, cx| term.read(cx).hovered_link.is_none()));
    std::fs::write(&missing, "now present").unwrap();
    h.cx.simulate_mouse_move(first, None, crate::platform::link_modifiers());
    assert!(
        matches!(h.read(|_,cx|term.read(cx).hovered_link.clone()).map(|link|link.open),Some(LinkOpen::Path{path,..}) if path==missing)
    );
}

#[gpui::test]
fn a_new_longest_path_replaces_an_existing_prefix_without_new_output(
    cx: &mut gpui::TestAppContext,
) {
    let mut h = Harness::open(cx, "path-growth", |home| {
        std::fs::write(home.home.join("Notes"), "short prefix").unwrap();
    });
    h.wait_prompt();
    h.run("clear; printf 'Saved ./Notes Folder/report.md\\n'", "Saved");
    h.wait_prompt();
    let term = h.focused_terminal();
    let at = find_on_screen(&mut h, &term, |text| text.starts_with("Saved "), "Notes");
    h.cx.simulate_mouse_move(at, None, crate::platform::link_modifiers());
    assert!(
        matches!(h.read(|_,cx|term.read(cx).hovered_link.clone()).map(|link|link.open),Some(LinkOpen::Path{path,..}) if path==h.home.home.join("Notes"))
    );
    let full = h.home.home.join("Notes Folder/report.md");
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, "longest").unwrap();
    h.cx.simulate_click(at, crate::platform::link_modifiers());
    h.cx.run_until_parked();
    assert_eq!(h.system.0.borrow().opened_files.last(), Some(&full));
}
