//! Actions the menu bar adds.

use gpui::TestAppContext;

use super::harness::Harness;
use crate::workspace_view::{MenuAction, OpenConfig, ResumeSession, RunClaude};

#[gpui::test]
fn settings_writes_a_missing_config_file(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "settings", |_| {});
    h.wait_prompt();
    assert!(!h.home.config.exists());
    h.cx.dispatch_action(OpenConfig);
    h.cx.run_until_parked();
    let text = std::fs::read_to_string(&h.home.config).unwrap();
    assert!(text.contains("restore-session"), "{text}");
    assert_eq!(
        h.system.0.borrow().opened_files,
        vec![h.home.config.clone()]
    );
}

#[gpui::test]
fn agent_items_explain_why_nothing_ran(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "agentmenu", |_| {});
    h.wait_prompt();
    h.cx.dispatch_action(ResumeSession);
    h.cx.run_until_parked();
    assert!(h.read(|v, _| v.palette.is_none()));
    assert!(
        h.read(|v, _| v.status_line.clone())
            .is_some_and(|s| s.contains("No agent sessions"))
    );

    // The test adapter reports Claude Code as not installed.
    h.cx.dispatch_action(RunClaude);
    h.cx.run_until_parked();
    assert!(
        h.read(|v, _| v.status_line.clone())
            .is_some_and(|s| s.contains("not on PATH"))
    );
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 1);
}

#[gpui::test]
fn agent_presets_start_from_the_palette_and_the_sidebar_menu(cx: &mut TestAppContext) {
    let mut repo = std::path::PathBuf::new();
    let mut h = Harness::open(cx, "presets", |home| {
        repo = home.repo("app");
        home.fake_claude();
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\"]\n\n[[agent-presets]]\nname = \"Opus\"\nagent = \"claude\"\nargs = [\"--model\", \"opus\"]\n",
                repo.display()
            ),
        )
        .unwrap();
    });
    h.wait_prompt();
    h.wait_for("the repository's worktree", {
        let repo = repo.clone();
        move |v, cx| v.sidebar.read(cx).model.worktree_for_path(&repo).is_some()
    });

    h.keys("cmd-shift-p");
    h.type_text("Run Opus in app");
    h.keys("enter");
    h.wait_for("the preset in a new tab", |v, cx| {
        v.ws.tabs().len() == 2 && v.focused_text(cx).contains("--model opus")
    });
    let text = h.read(|v, cx| v.focused_text(cx));
    assert!(text.contains("fake-claude --settings"), "{text}");

    let action = MenuAction::RunPreset(repo.clone(), "Opus".into());
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.run_menu_action(action, window, cx))
    });
    h.wait_for("a second preset tab", |v, cx| {
        v.ws.tabs().len() == 3 && v.focused_text(cx).contains("--model opus")
    });
}
