//! Reordering sidebar repositories by dragging their headers (#130).

use super::harness::Harness;
use gpui::{
    ExternalPaths, FileDropEvent, Modifiers, MouseButton, Pixels, Point, TestAppContext, point, px,
};
use std::path::PathBuf;

fn names(h: &Harness) -> Vec<String> {
    h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .map(|r| r.name.clone())
            .collect()
    })
}

fn collapsed(h: &Harness, name: &str) -> bool {
    h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .collapsed
    })
}

fn header(h: &mut Harness, index: usize) -> Point<Pixels> {
    h.cx.run_until_parked();
    let selector: &'static str = Box::leak(format!("repo-{index}").into_boxed_str());
    h.cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center()
}

fn drag(h: &mut Harness, from: Point<Pixels>, to: Point<Pixels>) {
    let none = Modifiers::none();
    h.cx.simulate_mouse_down(from, MouseButton::Left, none);
    for step in 1..=6 {
        let t = step as f32 / 6.0;
        let at = point(
            from.x + (to.x - from.x) * t,
            from.y + (to.y - from.y) * t + px(1.0),
        );
        h.cx.simulate_mouse_move(at, MouseButton::Left, none);
    }
    h.cx.simulate_mouse_move(to, MouseButton::Left, none);
    h.cx.simulate_mouse_up(to, MouseButton::Left, none);
    h.cx.run_until_parked();
}

fn saved_order(h: &Harness) -> Vec<PathBuf> {
    chda_config::ChdaConfig::load(&h.home.config).unwrap().repos
}

#[gpui::test]
fn dragging_repository_headers_reorders_and_persists(cx: &mut TestAppContext) {
    let mut repos = Vec::new();
    let mut h = Harness::open(cx, "reorder", |home| {
        repos = ["a", "b", "c"].map(|n| home.repo(n)).to_vec();
        let list: Vec<String> = repos
            .iter()
            .map(|r| format!("\"{}\"", r.display()))
            .collect();
        std::fs::write(&home.config, format!("repos = [{}]\n", list.join(", "))).unwrap();
    });
    h.wait_prompt();
    h.wait_for("every repository", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .all(|r| !r.worktrees.is_empty())
    });
    let tabs = h.read(|v, _| v.ws.tabs().len());
    let focused = h.read(|v, _| v.ws.focused_pane());

    // A collapsed group moves like an expanded one and stays collapsed.
    let at = header(&mut h, 1);
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
    assert!(collapsed(&h, "b"));

    // Down: a lands after c. The drag does not toggle a's group.
    let (from, to) = (header(&mut h, 0), header(&mut h, 2));
    drag(&mut h, from, to);
    assert_eq!(names(&h), ["b", "c", "a"]);
    assert!(!collapsed(&h, "a"), "dragging is not a click");
    assert!(collapsed(&h, "b"));
    assert_eq!(
        saved_order(&h),
        [&repos[1], &repos[2], &repos[0]].map(Clone::clone)
    );

    // Up: c lands before the collapsed b.
    let (from, to) = (header(&mut h, 1), header(&mut h, 0));
    drag(&mut h, from, to);
    assert_eq!(names(&h), ["c", "b", "a"]);
    assert!(collapsed(&h, "b"));

    // Dropping outside any repository changes nothing.
    let from = header(&mut h, 0);
    let terminal = h.read(|v, cx| {
        let pane = v.ws.focused_pane().unwrap();
        let g = v.panes[&pane].0.read(cx).geometry.unwrap();
        point(g.origin.x + px(200.0), g.origin.y + px(100.0))
    });
    drag(&mut h, from, terminal);
    assert_eq!(names(&h), ["c", "b", "a"]);
    // Dropping a header on itself changes nothing either.
    let from = header(&mut h, 2);
    drag(&mut h, from, point(from.x + px(30.0), from.y));
    assert_eq!(names(&h), ["c", "b", "a"]);

    // Display order only: tabs, focus and folders are untouched.
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs);
    assert_eq!(h.read(|v, _| v.ws.focused_pane()), focused);
    assert!(repos.iter().all(|r| r.join(".git").exists()));

    // Folders dropped from another app are still added as repositories.
    let dropped = h.home.repo("dropped");
    let at = header(&mut h, 0);
    h.cx.simulate_event(FileDropEvent::Entered {
        position: at,
        paths: ExternalPaths([dropped.clone()].into_iter().collect()),
    });
    h.cx.simulate_event(FileDropEvent::Pending { position: at });
    h.cx.simulate_event(FileDropEvent::Submit { position: at });
    h.cx.run_until_parked();
    h.wait_for("the dropped folder", move |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .any(|r| r.path == dropped)
    });
    assert_eq!(names(&h), ["c", "b", "a", "dropped"]);

    // A refresh keeps the order; so does the next launch.
    h.cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            for r in &repos {
                v.refresh_repo(r.clone(), cx);
            }
        })
    });
    h.cx.run_until_parked();
    assert_eq!(names(&h), ["c", "b", "a", "dropped"]);
    let (mut cx2, view) = h.reopen();
    cx2.run_until_parked();
    let reopened: Vec<String> = cx2.update(|_, cx| {
        view.read(cx)
            .sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .map(|r| r.name.clone())
            .collect()
    });
    assert_eq!(reopened, ["c", "b", "a", "dropped"]);
}

#[gpui::test]
fn dragging_near_project_edges_scrolls_only_project(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "reorder-scroll", |home| {
        let list: Vec<String> = (0..12)
            .map(|i| format!("\"{}\"", home.repo(&format!("repo-{i:02}")).display()))
            .collect();
        std::fs::write(&home.config, format!("repos = [{}]\n", list.join(", "))).unwrap();
    });
    h.wait_prompt();
    h.cx.simulate_resize(gpui::size(px(960.0), px(360.0)));
    h.wait_for("every repository", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .repos
            .iter()
            .all(|r| !r.worktrees.is_empty())
    });
    for _ in 0..8 {
        h.keys("cmd-t");
        h.wait_prompt();
    }
    h.cx.run_until_parked();
    let handles = |h: &Harness| {
        h.read(|v, cx| {
            let s = v.sidebar.read(cx);
            (s.sessions_scroll.clone(), s.project_scroll.clone())
        })
    };
    let (sessions, project) = handles(&h);
    assert!(sessions.max_offset().y > px(0.0) && project.max_offset().y > px(0.0));
    sessions.set_offset(point(px(0.0), px(-20.0)));
    h.view
        .update(&mut h.cx, |v, cx| v.sidebar.update(cx, |_, cx| cx.notify()));
    h.cx.run_until_parked();
    let list = h.cx.debug_bounds("project-list").expect("PROJECT's list");
    let above =
        h.cx.debug_bounds("sessions-list")
            .expect("Sessions' list")
            .center();
    let none = Modifiers::none();
    let from = header(&mut h, 0);
    h.cx.simulate_mouse_down(from, MouseButton::Left, none);
    // Near PROJECT's bottom edge: PROJECT scrolls down, Sessions stays.
    let bottom = point(from.x, list.bottom() - px(8.0));
    for step in 0..6 {
        h.cx.simulate_mouse_move(
            point(bottom.x, bottom.y - px(step as f32 % 2.0)),
            MouseButton::Left,
            none,
        );
    }
    h.cx.run_until_parked();
    let scrolled = project.offset().y;
    assert!(scrolled < px(0.0), "PROJECT follows the drag down");
    assert_eq!(
        sessions.offset().y,
        px(-20.0),
        "Sessions never scrolls for a drag"
    );
    // Above PROJECT, over Sessions: PROJECT scrolls back up, Sessions stays.
    for step in 0..3 {
        h.cx.simulate_mouse_move(
            point(above.x, above.y + px(step as f32)),
            MouseButton::Left,
            none,
        );
    }
    h.cx.run_until_parked();
    assert!(project.offset().y > scrolled, "PROJECT follows the drag up");
    assert_eq!(sessions.offset().y, px(-20.0));
    // Released outside the sidebar, nothing moves.
    let terminal = h.read(|v, cx| {
        let pane = v.ws.focused_pane().unwrap();
        let g = v.panes[&pane].0.read(cx).geometry.unwrap();
        point(g.origin.x + px(200.0), g.origin.y + px(100.0))
    });
    h.cx.simulate_mouse_move(terminal, MouseButton::Left, none);
    h.cx.simulate_mouse_up(terminal, MouseButton::Left, none);
    h.cx.run_until_parked();
    assert_eq!(names(&h)[0], "repo-00");
    assert_eq!(sessions.offset().y, px(-20.0));
}
