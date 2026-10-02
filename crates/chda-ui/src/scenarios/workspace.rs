//! Tabs, splits, focus and font size.

use gpui::{TestAppContext, px};

use super::harness::Harness;

#[gpui::test]
fn new_tab_split_and_close(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "tabs", |_| {});
    h.wait_prompt();
    h.keys("cmd-t");
    h.keys("cmd-d");
    h.read(|v, _| {
        assert_eq!(v.ws.tabs().len(), 2);
        assert_eq!(v.ws.active_tab().unwrap().panes().len(), 2);
    });
    let right = h.read(|v, _| v.ws.focused_pane());
    h.keys("cmd-alt-left");
    assert_ne!(h.read(|v, _| v.ws.focused_pane()), right);
    h.keys("cmd-alt-right");
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), right);
    h.keys("cmd-w");
    h.read(|v, _| assert_eq!(v.ws.active_tab().unwrap().panes().len(), 1));
    h.keys("cmd-1");
    assert_eq!(h.read(|v, _| v.ws.active_index()), Some(0));
}

#[gpui::test]
fn font_size_shortcuts_apply_to_every_pane(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "font", |home| {
        std::fs::write(&home.ghostty, "font-size = 13\n").unwrap();
    });
    h.wait_prompt();
    h.keys("cmd-d");
    h.keys("cmd-= cmd-=");
    let sizes = |h: &Harness| {
        h.read(|v, cx| {
            let mut s: Vec<_> = v
                .panes
                .values()
                .map(|(p, _)| p.read(cx).settings.font_size)
                .collect();
            s.push(v.settings.font_size);
            s
        })
    };
    assert!(sizes(&h).iter().all(|s| *s == px(15.0)), "{:?}", sizes(&h));
    h.keys("cmd--");
    assert!(sizes(&h).iter().all(|s| *s == px(14.0)));
    h.keys("cmd-0");
    assert!(sizes(&h).iter().all(|s| *s == px(13.0)));
}
