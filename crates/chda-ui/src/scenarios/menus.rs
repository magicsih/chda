//! Actions the menu bar adds.

use gpui::TestAppContext;

use super::harness::Harness;
use crate::workspace_view::{OpenConfig, ResumeSession, RunClaude};

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
