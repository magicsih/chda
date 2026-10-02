//! Command palette: moving the selection, and picking a theme.

use gpui::TestAppContext;

use super::harness::Harness;

#[gpui::test]
fn arrow_keys_choose_a_later_entry(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "palette", |_| {});
    h.wait_prompt();
    // New tab, Close pane, Split right: two steps down splits the pane.
    h.keys("cmd-shift-p");
    h.keys("down down enter");
    h.read(|v, _| {
        assert!(v.palette.is_none());
        assert_eq!(v.ws.tabs().len(), 1);
        assert_eq!(v.ws.active_tab().unwrap().panes().len(), 2);
    });
}
