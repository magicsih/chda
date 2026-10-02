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
