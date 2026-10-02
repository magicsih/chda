//! Session restore.

use gpui::TestAppContext;

use super::harness::Harness;

#[gpui::test]
fn tabs_splits_and_directories_come_back(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "restore", |home| {
        std::fs::create_dir_all(home.home.join("work/sub")).unwrap();
    });
    h.wait_prompt();
    let sub = h.home.home.join("work/sub");
    h.run(&format!("cd {}", sub.display()), "sub");
    h.wait_for("the pane to report its directory", {
        let sub = sub.clone();
        move |v, _| {
            let pane = v.ws.focused_pane().unwrap();
            v.ws.pane(pane).unwrap().cwd.as_deref() == Some(sub.as_path())
        }
    });
    h.keys("cmd-d");
    h.keys("cmd-t");
    h.cx.run_until_parked();

    let (cx2, view2) = h.reopen();
    cx2.run_until_parked();
    view2.read_with(&cx2, |v, _| {
        assert_eq!(v.ws.tabs().len(), 2);
        let first = &v.ws.tabs()[0];
        assert_eq!(first.panes().len(), 2);
        let cwd = v.ws.pane(first.panes()[0]).unwrap().cwd.clone();
        assert_eq!(cwd.as_deref(), Some(sub.as_path()));
    });
}
