//! User-defined `[[commands]]` in a tab, a split and a floating terminal
//! (#251).
use gpui::TestAppContext;

use super::harness::Harness;

const COMMANDS: &str = r#"
[[commands]]
name = "in-tab"
run = "printf 'tab-%s\n' ran; exec cat"
key = "cmd-shift-y"

[[commands]]
name = "in-split"
run = "printf 'split-%s\n' ran; exec cat"
open = "split"

[[commands]]
name = "float-fails"
run = "printf 'float-%s\n' failing; exit 3"
open = "floating"

[[commands]]
name = "float-succeeds"
run = "printf 'float-%s\n' ok; sleep 1"
open = "floating"

[[commands]]
name = "float-waits"
run = "exec cat"
open = "floating"
"#;

fn palette(h: &mut Harness, label: &str) {
    h.keys("cmd-shift-p");
    h.type_text(label);
    h.keys("enter");
}

#[gpui::test]
fn commands_run_in_a_tab_a_split_and_a_floating_terminal(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "commands", |home| {
        std::fs::write(&home.config, COMMANDS).unwrap();
    });
    h.wait_prompt();
    // A shortcut runs it in a new tab.
    h.keys("cmd-shift-y");
    h.wait_for("the tab command", |v, cx| {
        v.ws.tabs().len() == 2 && v.focused_text(cx).contains("tab-ran")
    });
    // The palette runs another beside the focused pane.
    palette(&mut h, "Run in-split");
    h.wait_for("the split command", |v, cx| {
        v.ws.active_tab().unwrap().panes().len() == 2 && v.focused_text(cx).contains("split-ran")
    });

    // A failing floating command stays with its exit status.
    let tabs = h.read(|v, _| v.ws.tabs().len());
    palette(&mut h, "Run float-fails");
    h.wait_for("the failure", |v, _| {
        v.floating
            .as_ref()
            .is_some_and(|f| f.exited.as_deref() == Some("Exited — code 3"))
    });
    let pane = h.read(|v, _| v.floating.as_ref().unwrap().pane);
    h.read(|v, cx| {
        assert!(v.pane_text(pane, cx).contains("float-failing"));
        assert_eq!(v.ws.tabs().len(), tabs, "no tab for a floating command");
        assert!(
            v.sidebar
                .read(cx)
                .model
                .sessions
                .iter()
                .all(|row| row.key != chda_core::SessionKey::Pane(pane)),
            "not a Sessions row"
        );
    });
    h.keys("escape");
    assert!(h.read(|v, _| v.floating.is_none()));
    assert!(h.read(|v, _| v.ws.pane(pane).is_none()));

    // A successful one closes by itself.
    palette(&mut h, "Run float-succeeds");
    assert!(h.read(|v, _| v.floating.is_some()));
    h.wait_for("the floating terminal to close", |v, _| {
        v.floating.is_none()
    });

    // Escape belongs to the running program; cmd-w twice stops it.
    palette(&mut h, "Run float-waits");
    assert!(h.read(|v, _| v.floating.is_some()));
    h.keys("escape");
    assert!(h.read(|v, _| v.floating.is_some()), "Escape never stops it");
    h.keys("cmd-w");
    assert!(h.read(|v, _| v.floating.as_ref().is_some_and(|f| f.stop_armed)));
    assert_eq!(
        h.read(|v, _| v.ws.tabs().len()),
        tabs,
        "cmd-w left the tabs alone"
    );
    h.keys("cmd-w");
    assert!(h.read(|v, _| v.floating.is_none()));
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs);
}

#[gpui::test]
fn broken_command_entries_are_reported_once_and_the_rest_still_run(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "commands-errors", |home| {
        std::fs::write(
            &home.config,
            r#"
[[commands]]
name = "good"
run = "printf 'good-%s\n' ran; exec cat"
key = "cmd-shift-y"

[[commands]]
name = "clash"
run = "true"
key = "cmd-t"

[[commands]]
name = "nowhere"
run = "true"
open = "window"
"#,
        )
        .unwrap();
    });
    h.wait_prompt();
    let problems = |h: &Harness| {
        h.read(|v, _| {
            v.notifications
                .newest()
                .iter()
                .filter(|e| e.message.contains("command"))
                .map(|e| e.message.clone())
                .collect::<Vec<_>>()
        })
    };
    let reported = problems(&h);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(
        reported[0].contains("cmd-t is a built-in shortcut"),
        "{reported:?}"
    );
    assert!(reported[0].contains("unknown `open`"), "{reported:?}");
    // cmd-t still opens a tab; the valid command keeps its shortcut.
    h.keys("cmd-t");
    h.wait_prompt();
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
    h.keys("cmd-shift-y");
    h.wait_for("the valid command", |v, cx| {
        v.focused_text(cx).contains("good-ran")
    });
    // A live fix needs no restart and is not reported again.
    std::fs::write(
        &h.home.config,
        "[[commands]]\nname = \"fixed\"\nrun = \"printf 'fixed-%s\\\\n' ran; exec cat\"\nkey = \"cmd-shift-u\"\n",
    )
    .unwrap();
    h.wait_for("the reload", |v, _| {
        v.config.commands.first().is_some_and(|c| c.name == "fixed")
    });
    assert_eq!(problems(&h).len(), 1, "the old problem is not repeated");
    h.keys("cmd-shift-u");
    h.wait_for("the fixed command", |v, cx| {
        v.focused_text(cx).contains("fixed-ran")
    });
}
