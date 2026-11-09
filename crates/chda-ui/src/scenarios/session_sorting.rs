//! Scenario tests for session sorting (Custom, Alphabetical, Creation time).
use super::{harness::Harness, sidebar::draw_sidebar_frame};
use chda_config::{ChdaConfig, SessionSortMode};
use gpui::{Modifiers, Pixels, Point, TestAppContext};

fn at(h: &mut Harness, selector: &str) -> Point<Pixels> {
    draw_sidebar_frame(h);
    h.cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center()
}

fn click(h: &mut Harness, selector: &str) {
    let p = at(h, selector);
    h.cx.simulate_click(p, Modifiers::none());
    h.cx.run_until_parked();
}

fn session_titles(h: &Harness) -> Vec<String> {
    h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .map(|r| r.title.clone())
            .collect()
    })
}

#[gpui::test]
fn session_sorting_modes_cycle_and_persist(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "session-sorting", |home| {
        let repo = home.repo("main-repo");
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();

    // Open two more tabs: "Zebra-tab", then "Alpha-tab"
    h.keys("cmd-t");
    h.wait_prompt();
    let tab2 = h.read(|v, _| v.ws.active_tab().unwrap().id);
    h.cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            v.ws.rename_tab(tab2, "Zebra");
            v.sync_panes(cx);
        });
    });

    h.keys("cmd-t");
    h.wait_prompt();
    let tab3 = h.read(|v, _| v.ws.active_tab().unwrap().id);
    h.cx.update(|_, cx| {
        h.view.update(cx, |v, cx| {
            v.ws.rename_tab(tab3, "Alpha");
            v.sync_panes(cx);
        });
    });

    h.wait_for("3 sessions exist", |v, cx| {
        v.sidebar.read(cx).model.sessions.len() == 3
    });

    // 1. Initial sort mode is Custom (created order: Tab 1, Zebra, Alpha)
    let initial_titles = session_titles(&h);
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).session_sort),
        SessionSortMode::Custom
    );

    // 2. Click session-sort-toggle -> Alphabetical (A-Z)
    click(&mut h, "session-sort-toggle");
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).session_sort),
        SessionSortMode::Alphabetical
    );
    let alpha_titles = session_titles(&h);
    // Alpha must come before Zebra
    let alpha_pos = alpha_titles.iter().position(|t| t == "Alpha").unwrap();
    let zebra_pos = alpha_titles.iter().position(|t| t == "Zebra").unwrap();
    assert!(
        alpha_pos < zebra_pos,
        "Alpha should precede Zebra in Alphabetical mode"
    );

    let saved_cfg = ChdaConfig::load(&h.home.config).unwrap();
    assert_eq!(saved_cfg.session_sort, SessionSortMode::Alphabetical);

    // 3. Click session-sort-toggle again -> Creation time (Recent/Newest first)
    click(&mut h, "session-sort-toggle");
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).session_sort),
        SessionSortMode::Creation
    );
    let recent_titles = session_titles(&h);
    // Alpha was opened most recently, then Zebra, then initial tab
    assert_eq!(recent_titles[0], "Alpha");
    assert_eq!(recent_titles[1], "Zebra");

    let saved_cfg = ChdaConfig::load(&h.home.config).unwrap();
    assert_eq!(saved_cfg.session_sort, SessionSortMode::Creation);

    // 4. Click session-sort-toggle again -> Custom mode restored
    click(&mut h, "session-sort-toggle");
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).session_sort),
        SessionSortMode::Custom
    );
    assert_eq!(session_titles(&h), initial_titles);

    let saved_cfg = ChdaConfig::load(&h.home.config).unwrap();
    assert_eq!(saved_cfg.session_sort, SessionSortMode::Custom);
}
