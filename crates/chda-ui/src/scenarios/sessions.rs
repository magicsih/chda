//! Sessions and PROJECT scroll on their own (#182). Session clicks, tab
//! switches, status changes, output and closing never move Sessions; the
//! focused branch is highlighted and revealed with minimal movement inside
//! PROJECT only, without expanding a collapsed PROJECT section. Reports
//! without a pane never take over a live agent's row (#205).

use super::harness::{Harness, git, wait_until};
use crate::sidebar_view::SidebarEvent;
use chda_core::agents::{ChildEvent, ChildState, HookEvent, HookKind, ipc};
use chda_core::{AgentStatus, PaneId, SessionKey, TabId};
use gpui::{Bounds, Modifiers, Pixels, ScrollHandle, TestAppContext, point, px, size};
use std::path::{Path, PathBuf};

/// Worktrees besides `main`, so PROJECT overflows a short window.
const WORKTREES: usize = 24;

struct Fixture {
    h: Harness,
    /// `task-00` ... `task-23`, in PROJECT's (name) order.
    worktrees: Vec<PathBuf>,
}

/// Two repositories, the first with many worktrees, in a 960x360 window,
/// with a terminal tab for each of `tabs` worktrees from the bottom of
/// PROJECT, so both lists overflow.
fn open(cx: &mut TestAppContext, name: &str, tabs: usize, extra: &str) -> Fixture {
    let mut worktrees = Vec::new();
    let extra = extra.to_owned();
    let mut h = Harness::open(cx, name, |home| {
        let repo = home.repo("app");
        for index in 0..WORKTREES {
            let branch = format!("task-{index:02}");
            let path = home.home.join(&branch);
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &branch,
                    path.to_str().unwrap(),
                ],
            );
            worktrees.push(path);
        }
        let other = home.repo("other");
        let config = format!(
            "repos = [\"{}\", \"{}\"]\n{}",
            repo.display(),
            other.display(),
            extra.replace("{repo}", &repo.display().to_string())
        );
        std::fs::write(&home.config, config).unwrap();
    });
    h.wait_prompt();
    h.cx.simulate_resize(size(px(960.0), px(360.0)));
    h.wait_for("every worktree", |v, cx| {
        v.sidebar.read(cx).model.repos[0].worktrees.len() == WORKTREES + 1
    });
    h.view.update(&mut h.cx, |v, cx| {
        v.sidebar.update(cx, |s, cx| {
            s.model.set_sort(chda_core::SortOrder::Name);
            cx.notify();
        })
    });
    for path in worktrees.iter().rev().take(tabs) {
        navigate(&mut h, SidebarEvent::OpenWorktree(path.clone()));
        h.wait_prompt();
    }
    settle(&mut h);
    Fixture { h, worktrees }
}

fn navigate(h: &mut Harness, event: SidebarEvent) {
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |v, cx| v.on_sidebar_event(event, window, cx))
    });
}

/// Deliver the frames a reveal waits for: the test platform has no display
/// frame loop.
fn settle(h: &mut Harness) {
    for _ in 0..3 {
        h.cx.run_until_parked();
        h.cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    h.cx.run_until_parked();
}

fn sessions_scroll(h: &Harness) -> ScrollHandle {
    h.read(|v, cx| v.sidebar.read(cx).sessions_scroll.clone())
}

fn project_scroll(h: &Harness) -> ScrollHandle {
    h.read(|v, cx| v.sidebar.read(cx).project_scroll.clone())
}

fn offsets(h: &Harness) -> (Pixels, Pixels) {
    (sessions_scroll(h).offset().y, project_scroll(h).offset().y)
}

/// Scroll a list as the user would, and redraw: setting an offset alone
/// does not repaint, so clicks would hit the previous positions.
fn scroll_to(h: &mut Harness, handle: ScrollHandle, y: Pixels) {
    handle.set_offset(point(px(0.0), y.clamp(-handle.max_offset().y, px(0.0))));
    h.view
        .update(&mut h.cx, |v, cx| v.sidebar.update(cx, |_, cx| cx.notify()));
    settle(h);
}

fn scroll_project(h: &mut Harness, y: Pixels) {
    let handle = project_scroll(h);
    scroll_to(h, handle, y);
}

fn scroll_sessions(h: &mut Harness, y: Pixels) {
    let handle = sessions_scroll(h);
    scroll_to(h, handle, y);
}

fn keys(h: &Harness) -> Vec<SessionKey> {
    h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .map(|r| r.key)
            .collect()
    })
}

fn selector(key: SessionKey) -> String {
    match key {
        SessionKey::Pane(pane) => format!("session-pane-{}", pane.raw()),
        SessionKey::Tab(tab) => format!("session-tab-{tab:?}"),
    }
}

fn bounds(h: &mut Harness, selector: &str) -> Option<Bounds<Pixels>> {
    h.cx.run_until_parked();
    h.cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

fn click(h: &mut Harness, selector: &str) {
    let at = bounds(h, selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
}

fn wheel(h: &mut Harness, selector: &str, dy: f32) {
    let at = bounds(h, selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center();
    h.cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(dy))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    settle(h);
}

fn worktree_row(path: &Path) -> String {
    format!("wt:{}", path.display())
}

/// Where PROJECT should end up to show `path`: unchanged when it is fully
/// visible below the sticky repository strip, else moved only to the
/// nearest edge (#149).
fn minimal_offset(h: &mut Harness, path: &Path) -> Pixels {
    let row = bounds(h, &worktree_row(path)).expect("the worktree row is laid out");
    let scroll = project_scroll(h);
    let viewport = scroll.bounds();
    let top = viewport.top() + px(32.0);
    let delta = if row.top() < top {
        top - row.top()
    } else if row.bottom() > viewport.bottom() {
        viewport.bottom() - row.bottom()
    } else {
        px(0.0)
    };
    (scroll.offset().y + delta).clamp(-scroll.max_offset().y, px(0.0))
}

/// The worktree row is entirely inside PROJECT's viewport, below the
/// sticky repository name when that is shown.
fn assert_revealed(h: &mut Harness, path: &Path, expected: Pixels) {
    let actual = project_scroll(h).offset().y;
    assert!(
        (actual - expected).abs() < px(2.0),
        "PROJECT moves only as far as needed: actual {actual:?}, expected {expected:?}"
    );
    let row = bounds(h, &worktree_row(path)).unwrap();
    let viewport = project_scroll(h).bounds();
    assert!(
        row.top() >= viewport.top() && row.bottom() <= viewport.bottom() + px(0.5),
        "{row:?} inside {viewport:?}"
    );
    let header = bounds(h, "repo-0").unwrap();
    if header.top() < viewport.top() {
        let sticky = bounds(h, "sticky-repo-context").expect("the repository stays named");
        assert!((sticky.top() - viewport.top()).abs() < px(1.0));
        assert!(row.top() >= sticky.bottom() - px(0.5));
    }
}

fn selected(h: &Harness) -> Option<PathBuf> {
    h.read(|v, cx| v.sidebar.read(cx).selected.clone())
}

fn focused(h: &Harness) -> (TabId, PaneId) {
    h.read(|v, _| (v.ws.active_tab().unwrap().id, v.ws.focused_pane().unwrap()))
}

/// Split the focused tab, `cd` the new pane into `path` and start a live
/// idle agent in each of the tab's two panes.
fn agents_in_split(h: &mut Harness, path: &Path) -> (PaneId, PaneId) {
    let (_, first) = focused(h);
    let first_cwd = h.read(|v, _| v.ws.pane(first).unwrap().cwd.clone().unwrap());
    h.keys("cmd-d");
    h.wait_prompt();
    let (_, second) = focused(h);
    h.run(&format!("cd '{}'", path.display()), "test%");
    h.wait_for("the split in its worktree", |v, _| {
        v.ws.pane(second).unwrap().cwd.as_deref() == Some(path)
    });
    h.hook_session(Some(first.raw()), &first_cwd, HookKind::SessionStart, "a");
    h.hook_from(
        "codex",
        Some(second.raw()),
        path,
        HookKind::SessionStart,
        "b",
    );
    h.wait_for("two live agents", move |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
        [first, second].iter().all(|pane| {
            rows.iter()
                .any(|r| r.key == SessionKey::Pane(*pane) && r.live_idle())
        })
    });
    (first, second)
}

#[gpui::test]
fn sessions_and_project_scroll_independently_when_both_overflow(cx: &mut TestAppContext) {
    let mut f = open(cx, "sessions-scroll", 10, "");
    let h = &mut f.h;
    let sidebar = bounds(h, "sidebar").expect("the sidebar is drawn");
    let sessions = bounds(h, "sessions-list").expect("Sessions has its own list");
    let project = bounds(h, "project-list").expect("PROJECT has its own list");
    assert!(
        sessions.size.height <= sidebar.size.height * 0.4 + px(0.5),
        "Sessions stops at 40% of the sidebar: {sessions:?} in {sidebar:?}"
    );
    assert!(
        sessions.bottom() <= project.top(),
        "Sessions sits above PROJECT"
    );
    let header = bounds(h, "project-section-toggle").unwrap();
    assert!(sessions.bottom() <= header.top() && header.bottom() <= project.top());
    assert!(
        sessions_scroll(h).max_offset().y > px(0.0),
        "Sessions overflows"
    );
    assert!(
        project_scroll(h).max_offset().y > px(0.0),
        "PROJECT overflows"
    );

    for (list, dy) in [
        ("sessions-list", -40.0),
        ("project-list", -60.0),
        ("sessions-list", 15.0),
        ("project-list", 25.0),
    ] {
        let (sessions, project) = offsets(h);
        wheel(h, list, dy);
        let (after_sessions, after_project) = offsets(h);
        if list == "sessions-list" {
            assert_ne!(after_sessions, sessions, "the wheel moves Sessions");
            assert_eq!(after_project, project, "Sessions' wheel leaves PROJECT");
        } else {
            assert_ne!(after_project, project, "the wheel moves PROJECT");
            assert_eq!(after_sessions, sessions, "PROJECT's wheel leaves Sessions");
        }
    }
    // Reaching the end of one list does not hand the wheel to the other.
    let project = offsets(h).1;
    wheel(h, "sessions-list", -5000.0);
    let max = sessions_scroll(h).max_offset().y;
    assert!((offsets(h).0 + max).abs() < px(0.5));
    assert_eq!(offsets(h).1, project);
    let sessions = offsets(h).0;
    wheel(h, "project-list", -5000.0);
    assert_eq!(offsets(h).0, sessions);

    // Both regions stay usable at the narrowest sidebar.
    h.view.update(&mut h.cx, |v, cx| {
        v.config.sidebar_width = 180;
        cx.notify();
    });
    settle(h);
    let sessions = bounds(h, "sessions-list").unwrap();
    let project = bounds(h, "project-list").unwrap();
    assert!(sessions.size.width <= px(181.0));
    assert!(sessions.size.height > px(40.0) && project.size.height > px(40.0));
    assert!(bounds(h, "add-repo").is_some() && bounds(h, "sessions-section-toggle").is_some());

    // Collapsing PROJECT gives Sessions the remaining height.
    click(h, "project-section-toggle");
    settle(h);
    assert!(bounds(h, "project-list").is_none());
    let sessions = bounds(h, "sessions-list").unwrap();
    let sidebar = bounds(h, "sidebar").unwrap();
    assert!(sessions.size.height > sidebar.size.height * 0.4 + px(1.0));
    assert!(bounds(h, "add-repo").is_some(), "+ repo stays reachable");
}

#[gpui::test]
fn session_clicks_keep_sessions_still_and_reveal_only_in_project(cx: &mut TestAppContext) {
    let mut f = open(cx, "sessions-click", 9, "");
    let worktrees = f.worktrees.clone();
    let h = &mut f.h;
    // The first tab's agent heads the list; the newest tab (task-15) holds a
    // split with another agent in task-20.
    let home = h.read(|v, _| v.ws.tabs()[0].panes()[0]);
    h.hook_session(
        Some(home.raw()),
        &h.home.home.clone(),
        HookKind::SessionStart,
        "h",
    );
    let (first, second) = agents_in_split(h, &worktrees[20]);
    let task22 = h.read(|v, _| v.ws.tabs()[2].id);
    navigate(h, SidebarEvent::FocusTab(h.read(|v, _| v.ws.tabs()[0].id)));
    settle(h);
    let first_cwd = h.read(|v, _| v.ws.pane(first).unwrap().cwd.clone().unwrap());
    assert_eq!(
        &keys(h)[..3],
        &[
            SessionKey::Pane(home),
            SessionKey::Pane(first),
            SessionKey::Pane(second)
        ]
    );

    // PROJECT at its top, so these branches are below its viewport, and
    // Sessions one row down, so the first row is hidden.
    scroll_project(h, px(0.0));
    let sessions = sessions_scroll(h);
    let top = bounds(h, &selector(SessionKey::Pane(home))).unwrap();
    let viewport = sessions.bounds();
    scroll_to(
        h,
        sessions.clone(),
        sessions.offset().y - (top.bottom() - viewport.top()),
    );
    settle(h);
    let kept = offsets(h).0;
    assert!(kept < px(0.0), "Sessions is scrolled away from its top");

    for (key, worktree) in [
        (SessionKey::Pane(second), worktrees[20].clone()),
        (SessionKey::Pane(first), first_cwd.clone()),
        (SessionKey::Tab(task22), worktrees[22].clone()),
        (SessionKey::Pane(second), worktrees[20].clone()),
    ] {
        scroll_project(h, px(0.0));
        settle(h);
        let expected = minimal_offset(h, &worktree);
        let row = bounds(h, &selector(key)).unwrap();
        assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom() + px(0.5));
        click(h, &selector(key));
        settle(h);
        match key {
            SessionKey::Pane(pane) => {
                assert_eq!(focused(h).1, pane, "the row's own pane is focused")
            }
            SessionKey::Tab(tab) => assert_eq!(focused(h).0, tab, "the row's tab is focused"),
        }
        assert_eq!(offsets(h).0, kept, "a session click never scrolls Sessions");
        assert_eq!(
            h.read(|v, cx| v.sidebar.read(cx).focused_session()),
            Some(key)
        );
        assert_eq!(
            selected(h).as_ref(),
            Some(&worktree),
            "PROJECT follows focus"
        );
        assert_revealed(h, &worktree, expected);
    }
    // A visible target keeps PROJECT exactly where it is.
    let project = offsets(h).1;
    click(h, &selector(SessionKey::Pane(second)));
    settle(h);
    assert_eq!(offsets(h), (kept, project));

    // A tab switch follows the same rule: the app group's third tab.
    scroll_project(h, px(0.0));
    settle(h);
    let tab_worktree = worktrees[WORKTREES - 3].clone();
    let expected = minimal_offset(h, &tab_worktree);
    h.keys("cmd-3");
    settle(h);
    assert_eq!(selected(h).as_ref(), Some(&tab_worktree));
    assert_eq!(offsets(h).0, kept);
    assert_revealed(h, &tab_worktree, expected);

    // A collapsed repository group opens for the branch it reveals.
    scroll_project(h, px(0.0));
    settle(h);
    click(h, "repo-0");
    assert!(h.read(|v, cx| v.sidebar.read(cx).model.repos[0].collapsed));
    click(h, &selector(SessionKey::Pane(first)));
    settle(h);
    assert!(h.read(|v, cx| !v.sidebar.read(cx).model.repos[0].collapsed));
    assert_eq!(offsets(h).0, kept);

    // A collapsed PROJECT section stays collapsed; only the highlight moves.
    click(h, "project-section-toggle");
    assert!(bounds(h, "repo-0").is_none());
    click(h, &selector(SessionKey::Pane(second)));
    settle(h);
    assert_eq!(focused(h).1, second);
    assert_eq!(selected(h).as_ref(), Some(&worktrees[20]));
    assert!(bounds(h, "repo-0").is_none(), "PROJECT stays collapsed");
    assert!(h.read(|v, _| v.config.project_collapsed));
    assert_eq!(offsets(h).0, kept);
    // Expanding it shows the highlighted branch.
    click(h, "project-section-toggle");
    settle(h);
    let row = bounds(h, &worktree_row(&worktrees[20])).unwrap();
    let viewport = project_scroll(h).bounds();
    assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom() + px(0.5));
    assert_eq!(offsets(h).0, kept);
}

#[gpui::test]
fn status_changes_keep_session_rows_in_place(cx: &mut TestAppContext) {
    use crate::terminal_view::{TerminalEvent, now_ms};

    let mut f = open(cx, "sessions-status", 8, "");
    let worktrees = f.worktrees.clone();
    let h = &mut f.h;
    let (first, second) = agents_in_split(h, &worktrees[3]);
    let first_cwd = h.read(|v, _| v.ws.pane(first).unwrap().cwd.clone().unwrap());
    let background = h.read(|v, _| {
        let tab = &v.ws.tabs()[0];
        v.panes[&tab.panes()[0]].0.clone()
    });
    navigate(h, SidebarEvent::FocusTab(h.read(|v, _| v.ws.tabs()[1].id)));
    settle(h);
    let order = keys(h);
    let terminals = order
        .iter()
        .filter(|k| matches!(k, SessionKey::Tab(_)))
        .count();
    assert_eq!(order.len(), terminals + 2, "one row per agent pane");
    assert_eq!(
        &order[..2],
        &[SessionKey::Pane(first), SessionKey::Pane(second)]
    );
    scroll_sessions(h, px(-30.0));
    scroll_project(h, px(-90.0));
    settle(h);
    let kept = offsets(h);
    let focus = focused(h);
    let highlight = selected(h);

    let check = |h: &mut Harness, what: &str| {
        settle(h);
        assert_eq!(keys(h), order, "{what}: rows keep their identity and place");
        assert_eq!(offsets(h), kept, "{what}: neither region scrolls");
        assert_eq!(focused(h), focus, "{what}: focus stays");
        assert_eq!(selected(h), highlight, "{what}: PROJECT highlight stays");
    };
    for (pane, cwd, agent, session) in [
        (first, first_cwd.as_path(), "claude", "a"),
        (second, worktrees[3].as_path(), "codex", "b"),
    ] {
        for (kind, status) in [
            (HookKind::PromptSubmitted, AgentStatus::Working),
            (HookKind::WaitingInput, AgentStatus::WaitingInput),
            (HookKind::Stopped, AgentStatus::Review),
            (HookKind::PromptSubmitted, AgentStatus::Working),
        ] {
            h.hook_from(agent, Some(pane.raw()), cwd, kind, session);
            h.wait_for("the new status", move |v, _| {
                v.ws.pane(pane)
                    .and_then(|p| p.agent.as_ref())
                    .is_some_and(|a| a.status == status)
            });
            check(h, &format!("{agent} {status:?}"));
        }
        h.hook_from(agent, Some(pane.raw()), cwd, HookKind::Idle, session);
        h.hook_from(
            agent,
            Some(pane.raw()),
            cwd,
            HookKind::SessionStart,
            session,
        );
        h.wait_for("idle again", move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .sessions
                .iter()
                .any(|r| r.key == SessionKey::Pane(pane) && r.live_idle())
        });
        check(h, &format!("{agent} idle"));
        assert_eq!(
            keys(h)
                .iter()
                .filter(|k| **k == SessionKey::Pane(pane))
                .count(),
            1,
            "a live idle agent is listed once"
        );
    }
    // Output in a background tab.
    for _ in 0..3 {
        h.cx.update(|_, cx| {
            background.update(cx, |_, cx| cx.emit(TerminalEvent::Activity(now_ms())))
        });
        check(h, "background output");
    }
    // A provider-confirmed child under the first agent.
    ipc::send(
        &ipc::socket_path(&h.home.data),
        &HookEvent {
            agent: "claude".into(),
            session_id: "a".into(),
            cwd: first_cwd.clone(),
            kind: HookKind::Stopped,
            timestamp: now_ms(),
            pane: Some(first.raw()),
            child: Some(ChildEvent {
                id: "child".into(),
                label: Some("Explore".into()),
                state: ChildState::Working,
                started: true,
            }),
        },
    )
    .unwrap();
    h.wait_for("the child group", |v, cx| {
        !v.sidebar.read(cx).child_groups.is_empty()
    });
    check(h, "child update");
    let parent = bounds(h, &selector(SessionKey::Pane(first))).unwrap();
    let child = bounds(h, &format!("child-{}-child", first.raw())).expect("child row");
    let next = bounds(h, &selector(SessionKey::Pane(second))).unwrap();
    assert!(parent.bottom() <= child.top() && child.bottom() <= next.top());
}

/// The only pane of a worktree, beside terminal tabs, for an agent that
/// reports through its pane while another agent, started outside chda,
/// reports for the same worktree without one (#205).
struct SharedWorktree {
    pane: PaneId,
    worktree: PathBuf,
    /// The Sessions order once the pane's agent is live.
    order: Vec<SessionKey>,
}

impl SharedWorktree {
    /// Three terminal tabs; the last one's worktree is shared. Another tab
    /// stays focused, so nothing counts as looked at.
    fn open(cx: &mut TestAppContext, name: &str) -> (Fixture, Self) {
        let mut f = open(cx, name, 3, "");
        let worktree = f.worktrees[WORKTREES - 3].clone();
        let h = &mut f.h;
        let (_, pane) = focused(h);
        let path = worktree.clone();
        h.wait_for("the pane in its worktree", move |v, _| {
            v.ws.pane(pane).unwrap().cwd.as_deref() == Some(path.as_path())
        });
        navigate(h, SidebarEvent::FocusTab(h.read(|v, _| v.ws.tabs()[0].id)));
        settle(h);
        let shared = Self {
            pane,
            worktree,
            order: Vec::new(),
        };
        (f, shared)
    }

    /// Start the pane's own agent and remember where its row is.
    fn start(&mut self, h: &mut Harness) {
        self.own(h, HookKind::SessionStart, AgentStatus::Idle);
        let pane = self.pane;
        h.wait_for("a live agent row", move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .sessions
                .iter()
                .any(|r| r.key == SessionKey::Pane(pane) && r.live_idle())
        });
        settle(h);
        self.order = keys(h);
        assert_eq!(self.order.len(), 4, "one agent row and three terminal rows");
        assert_eq!(self.order[0], SessionKey::Pane(pane), "agents come first");
    }

    /// An event from the pane's own agent, which names the pane.
    fn own(&self, h: &mut Harness, kind: HookKind, status: AgentStatus) {
        let pane = self.pane;
        h.hook_session(Some(pane.raw()), &self.worktree, kind, "own");
        h.wait_for("the agent's own status", move |v, _| {
            v.ws.pane(pane)
                .and_then(|p| p.agent.as_ref())
                .is_some_and(|a| a.status == status)
        });
    }

    /// An event from the other agent: it names no pane, and the worktree's
    /// attention still follows it.
    fn other(&self, h: &mut Harness, kind: HookKind, attention: AgentStatus) {
        h.hook_session(None, &self.worktree, kind, "other");
        let path = self.worktree.clone();
        h.wait_for("the worktree's attention", move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&path)
                .and_then(|(_, w)| w.agents.get("claude").copied())
                == Some(attention)
        });
    }

    /// The pane's row keeps its key, place, life and own status.
    fn check(&self, h: &mut Harness, status: AgentStatus, what: &str) {
        settle(h);
        assert_eq!(keys(h), self.order, "{what}: rows keep their key and place");
        let pane = self.pane;
        let row = h.read(|v, cx| {
            v.sidebar
                .read(cx)
                .model
                .sessions
                .iter()
                .find(|r| r.key == SessionKey::Pane(pane))
                .cloned()
        });
        let row = row.unwrap_or_else(|| panic!("{what}: the agent row is listed"));
        assert!(row.agent_live, "{what}: the agent stays live");
        assert_eq!(
            row.status,
            Some(status),
            "{what}: the pane keeps its own status"
        );
    }
}

#[gpui::test]
fn reports_without_a_pane_after_the_live_agents_own_keep_its_row(cx: &mut TestAppContext) {
    let (mut f, mut shared) = SharedWorktree::open(cx, "sessions-nopane-a");
    let h = &mut f.h;
    shared.start(h);
    for (own, status, other, attention) in [
        (
            HookKind::PromptSubmitted,
            AgentStatus::Working,
            HookKind::Stopped,
            AgentStatus::Review,
        ),
        (
            HookKind::Stopped,
            AgentStatus::Review,
            HookKind::PromptSubmitted,
            AgentStatus::Working,
        ),
        (
            HookKind::PromptSubmitted,
            AgentStatus::Working,
            HookKind::SessionEnd,
            AgentStatus::Idle,
        ),
    ] {
        shared.own(h, own, status);
        shared.check(h, status, &format!("own {own:?}"));
        shared.other(h, other, attention);
        shared.check(h, status, &format!("other {other:?} after own {own:?}"));
    }
}

#[gpui::test]
fn reports_without_a_pane_before_the_live_agents_own_keep_its_row(cx: &mut TestAppContext) {
    let (mut f, mut shared) = SharedWorktree::open(cx, "sessions-nopane-b");
    let h = &mut f.h;
    // Before the pane's agent starts, a report without a pane is attention
    // only: the pane shows it, but no agent row is listed.
    shared.other(h, HookKind::Stopped, AgentStatus::Review);
    settle(h);
    let pane = shared.pane;
    assert!(h.read(|v, _| {
        let info = v.ws.pane(pane).unwrap();
        !info.agent_live && info.agent.as_ref().map(|a| a.status) == Some(AgentStatus::Review)
    }));
    assert!(
        !keys(h).contains(&SessionKey::Pane(pane)),
        "a report without a pane creates no agent row"
    );
    shared.start(h);
    let mut status = AgentStatus::Idle;
    for (other, attention, own, next) in [
        (
            HookKind::Stopped,
            AgentStatus::Review,
            HookKind::WaitingInput,
            AgentStatus::WaitingInput,
        ),
        (
            HookKind::PromptSubmitted,
            AgentStatus::Working,
            HookKind::Stopped,
            AgentStatus::Review,
        ),
        (
            HookKind::SessionEnd,
            AgentStatus::Idle,
            HookKind::PromptSubmitted,
            AgentStatus::Working,
        ),
    ] {
        shared.other(h, other, attention);
        shared.check(h, status, &format!("other {other:?} before own {own:?}"));
        shared.own(h, own, next);
        status = next;
        shared.check(h, status, &format!("own {own:?}"));
    }
}

/// A worktree row fully visible in PROJECT, below the sticky strip.
fn visible_worktree(h: &mut Harness, worktrees: &[PathBuf]) -> PathBuf {
    let viewport = project_scroll(h).bounds();
    worktrees
        .iter()
        .find(|path| {
            bounds(h, &worktree_row(path)).is_some_and(|row| {
                row.top() >= viewport.top() + px(32.0) && row.bottom() <= viewport.bottom()
            })
        })
        .expect("a visible worktree row")
        .clone()
}

#[gpui::test]
fn project_navigation_does_not_scroll_sessions(cx: &mut TestAppContext) {
    let mut f = open(
        cx,
        "sessions-project",
        9,
        "starred = [{ repo = \"{repo}\", branch = \"task-12\" }]\n",
    );
    let worktrees = f.worktrees.clone();
    let h = &mut f.h;
    scroll_sessions(h, px(-24.0));
    settle(h);
    let kept = offsets(h).0;
    assert!(kept < px(0.0));
    let check = |h: &mut Harness, path: &Path, expected: Pixels, what: &str| {
        settle(h);
        assert_eq!(offsets(h).0, kept, "{what} leaves Sessions alone");
        assert_eq!(
            selected(h).as_deref(),
            Some(path),
            "{what} highlights its target"
        );
        assert_revealed(h, path, expected);
    };

    // A visible worktree row: its click opens a tab and moves nothing.
    scroll_project(h, px(-40.0));
    settle(h);
    let target = visible_worktree(h, &worktrees[..10]);
    let expected = project_scroll(h).offset().y;
    click(h, &worktree_row(&target));
    h.wait_prompt();
    check(h, &target, expected, "a worktree click");

    // The palette's Go to worktree, for a branch below PROJECT's viewport.
    scroll_project(h, px(0.0));
    settle(h);
    let target = worktrees[20].clone();
    let expected = minimal_offset(h, &target);
    assert!(expected < px(0.0), "the target starts below the viewport");
    h.keys("cmd-shift-p");
    h.type_text("Go to app/task-20");
    h.keys("enter");
    h.wait_prompt();
    check(h, &target, expected, "Go to worktree");

    // STARRED, for a branch below PROJECT's viewport.
    scroll_project(h, px(0.0));
    settle(h);
    let target = worktrees[12].clone();
    let expected = minimal_offset(h, &target);
    click(h, "starred-0");
    h.wait_prompt();
    check(h, &target, expected, "STARRED");

    // A notification whose pane is gone, for a branch above the viewport.
    let target = worktrees[0].clone();
    let scroll = project_scroll(h);
    scroll_to(h, scroll.clone(), -scroll.max_offset().y);
    settle(h);
    let expected = minimal_offset(h, &target);
    assert!(
        expected > -scroll.max_offset().y,
        "the target starts above the viewport"
    );
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_notification_click(
                crate::platform::NotificationTarget {
                    pane: Some(u64::MAX),
                    worktree: target.clone(),
                },
                window,
                cx,
            )
        })
    });
    check(h, &target, expected, "a notification for a closed pane");

    // Explicit PROJECT navigation expands a collapsed PROJECT section.
    click(h, "project-section-toggle");
    assert!(bounds(h, "repo-0").is_none());
    let target = worktrees[20].clone();
    navigate(h, SidebarEvent::OpenWorktree(target.clone()));
    settle(h);
    assert!(
        bounds(h, "repo-0").is_some(),
        "PROJECT opens for its own navigation"
    );
    assert!(!h.read(|v, _| v.config.project_collapsed));
    assert_eq!(offsets(h).0, kept);
    assert_eq!(selected(h).as_ref(), Some(&target));

    // Background output and refreshes move neither region.
    scroll_project(h, px(0.0));
    settle(h);
    let before = offsets(h);
    h.run("echo still-here", "still-here");
    let repo = h.read(|v, cx| v.sidebar.read(cx).model.repos[0].path.clone());
    h.cx.update(|_, cx| h.view.update(cx, |v, cx| v.refresh_repo(repo, cx)));
    settle(h);
    assert_eq!(offsets(h), before);
}

#[gpui::test]
fn closing_sessions_keeps_both_regions_in_place(cx: &mut TestAppContext) {
    let mut f = open(cx, "sessions-close", 9, "");
    let worktrees = f.worktrees.clone();
    let h = &mut f.h;
    let (first, idle) = agents_in_split(h, &worktrees[4]);
    let agent_tab = focused(h).0;
    // Work in another tab and close rows that belong to others.
    let active = h.read(|v, _| v.ws.tabs()[3].id);
    navigate(h, SidebarEvent::FocusTab(active));
    settle(h);
    let focus = focused(h);
    // Sessions starts one row down, so its offset survives two fewer rows.
    let sessions = sessions_scroll(h);
    let top = bounds(h, &selector(SessionKey::Pane(first))).unwrap();
    scroll_to(
        h,
        sessions.clone(),
        sessions.offset().y - (top.bottom() - sessions.bounds().top()),
    );
    scroll_project(h, px(-80.0));
    settle(h);
    let kept = offsets(h);
    assert!(kept.0 < px(0.0) && kept.1 < px(0.0));
    let viewport = sessions.bounds();
    let terminal = h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .filter(|r| r.tab != focus.0 && r.kind == chda_core::SessionKind::Terminal)
            .map(|r| r.tab)
            .collect::<Vec<_>>()
    });
    let terminal = *terminal
        .iter()
        .find(|tab| {
            bounds(h, &format!("session-close-tab-{tab:?}"))
                .is_some_and(|b| b.top() >= viewport.top() && b.bottom() <= viewport.bottom())
        })
        .expect("a visible background terminal row");
    let tabs = h.read(|v, _| v.ws.tabs().len());
    assert!(bounds(h, &format!("session-close-tab-{active:?}")).is_some());
    assert!(
        bounds(h, &format!("session-close-pane-{}", first.raw())).is_some(),
        "a live idle agent's pane can close from its row"
    );

    click(h, &format!("session-close-tab-{terminal:?}"));
    settle(h);
    assert!(h.read(|v, _| v.confirm.is_none()));
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), tabs - 1);
    assert!(h.read(|v, _| !v.ws.tabs().iter().any(|t| t.id == terminal)));
    assert_eq!(
        focused(h),
        focus,
        "the background tab closes without activating it"
    );
    assert_eq!(offsets(h), kept);

    click(h, &format!("session-close-pane-{}", idle.raw()));
    h.wait_for("the idle pane closed", move |v, _| {
        v.ws.pane(idle).is_none()
    });
    settle(h);
    assert!(
        h.read(|v, _| v.ws.pane(first).is_some()),
        "only its own pane"
    );
    assert!(h.read(|v, _| v.ws.tabs().iter().any(|t| t.id == agent_tab)));
    assert_eq!(focused(h), focus);
    assert_eq!(offsets(h), kept);
    assert!(!keys(h).contains(&SessionKey::Pane(idle)));
}

#[gpui::test]
fn sessions_collapse_persists_and_migrates_legacy_keys(cx: &mut TestAppContext) {
    let legacy = "active-collapsed = true\nidle-agents-collapsed = true\n";
    let mut h = Harness::open(cx, "sessions-legacy", |home| {
        std::fs::write(&home.config, legacy).unwrap();
    });
    h.wait_prompt();
    h.wait_for("the session rows", |v, cx| {
        !v.sidebar.read(cx).model.sessions.is_empty()
    });
    let tab = focused(&h).0;
    let row = selector(SessionKey::Tab(tab));
    assert!(h.read(|v, cx| v.sidebar.read(cx).sessions_collapsed));
    assert!(bounds(&mut h, "sessions-section-toggle").is_some());
    assert!(bounds(&mut h, "sessions-list").is_none());
    assert!(bounds(&mut h, &row).is_none());
    assert_eq!(
        std::fs::read_to_string(&h.home.config).unwrap(),
        legacy,
        "loading never rewrites the file"
    );
    // The label switch never expands the section; the next save drops the
    // legacy keys.
    let label = h.read(|v, _| v.config.active_label);
    click(&mut h, "sessions-label-toggle");
    assert_ne!(h.read(|v, _| v.config.active_label), label);
    assert!(bounds(&mut h, &row).is_none());
    let saved = std::fs::read_to_string(&h.home.config).unwrap();
    assert!(saved.contains("sessions-collapsed = true"), "{saved}");
    assert!(!saved.contains("active-collapsed") && !saved.contains("idle-agents-collapsed"));

    click(&mut h, "sessions-section-toggle");
    assert!(bounds(&mut h, &row).is_some());
    assert!(
        !chda_config::ChdaConfig::load(&h.home.config)
            .unwrap()
            .sessions_collapsed
    );
    click(&mut h, "sessions-section-toggle");
    assert!(bounds(&mut h, &row).is_none());
    assert!(
        chda_config::ChdaConfig::load(&h.home.config)
            .unwrap()
            .sessions_collapsed
    );

    // The preference survives a restart.
    let (mut cx2, view2) = h.reopen();
    wait_until(&mut cx2, &view2, "restored preference", |v, cx| {
        v.sidebar.read(cx).sessions_collapsed && !v.sidebar.read(cx).model.sessions.is_empty()
    });
    // Edited outside chda, a file with only one legacy section collapsed
    // expands Sessions; both collapsed collapse it again.
    std::fs::write(&h.home.config, "active-collapsed = true\n").unwrap();
    h.wait_for("one legacy key", |v, cx| {
        !v.sidebar.read(cx).sessions_collapsed
    });
    assert!(bounds(&mut h, &row).is_some());
    std::fs::write(&h.home.config, legacy).unwrap();
    h.wait_for("both legacy keys", |v, cx| {
        v.sidebar.read(cx).sessions_collapsed
    });
    assert!(bounds(&mut h, &row).is_none());
}

/// Collapse PROJECT, then let a session click schedule a reveal it cannot
/// draw yet.
fn reveal_while_collapsed(h: &mut Harness, worktrees: &[PathBuf]) {
    scroll_project(h, px(0.0));
    click(h, "project-section-toggle");
    assert!(bounds(h, "repo-0").is_none());
    let tab = h.read(|v, _| v.ws.tabs()[1].id);
    click(h, &selector(SessionKey::Tab(tab)));
    settle(h);
    assert_eq!(selected(h).as_ref(), Some(&worktrees[WORKTREES - 1]));
}

#[gpui::test]
fn collapsed_project_reveals_only_the_current_highlight_after_a_cd(cx: &mut TestAppContext) {
    let mut f = open(cx, "sessions-stale-cd", 9, "");
    let worktrees = f.worktrees.clone();
    let h = &mut f.h;
    reveal_while_collapsed(h, &worktrees);
    // The highlight moves on without navigation: a cd in the focused pane.
    let target = worktrees[0].clone();
    h.run(&format!("cd '{}'", target.display()), "test%");
    let expected = target.clone();
    h.wait_for("the highlight follows the cd", move |v, cx| {
        v.sidebar.read(cx).selected.as_ref() == Some(&expected)
    });
    let sessions = offsets(h).0;
    click(h, "project-section-toggle");
    settle(h);
    assert_eq!(
        project_scroll(h).offset().y,
        px(0.0),
        "expanding PROJECT does not scroll to the earlier session's worktree"
    );
    let row = bounds(h, &worktree_row(&target)).unwrap();
    let viewport = project_scroll(h).bounds();
    assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom() + px(0.5));
    assert_eq!(offsets(h).0, sessions);
}

#[gpui::test]
fn collapsed_project_drops_a_reveal_when_focus_leaves_worktrees(cx: &mut TestAppContext) {
    let mut f = open(cx, "sessions-stale-home", 9, "");
    let worktrees = f.worktrees.clone();
    let h = &mut f.h;
    reveal_while_collapsed(h, &worktrees);
    // A session outside every worktree: nothing is highlighted.
    let home = h.read(|v, _| v.ws.tabs()[0].id);
    click(h, &selector(SessionKey::Tab(home)));
    settle(h);
    assert_eq!(selected(h), None);
    click(h, "project-section-toggle");
    settle(h);
    assert_eq!(
        project_scroll(h).offset().y,
        px(0.0),
        "nothing is highlighted, so nothing is revealed"
    );
    // Another session click while collapsed: only its own highlight is revealed.
    reveal_while_collapsed(h, &worktrees);
    let other = h.read(|v, _| v.ws.tabs()[2].id);
    click(h, &selector(SessionKey::Tab(other)));
    settle(h);
    let highlight = selected(h).unwrap();
    assert_eq!(highlight, worktrees[WORKTREES - 2]);
    click(h, "project-section-toggle");
    settle(h);
    let row = bounds(h, &worktree_row(&highlight)).unwrap();
    let viewport = project_scroll(h).bounds();
    assert!(
        row.top() >= viewport.top() && row.bottom() <= viewport.bottom() + px(0.5),
        "the current highlight is the one revealed"
    );
}

#[gpui::test]
fn session_rows_focus_their_pane_without_scrolling_its_terminal(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "sessions-scrollback", |_| {});
    h.wait_prompt();
    let (_, agent) = focused(&h);
    h.run("seq 1000 1300", "1299");
    h.hook(
        Some(agent.raw()),
        &h.home.home.clone(),
        HookKind::PromptSubmitted,
    );
    h.wait_for("a working agent row", move |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .any(|r| r.key == SessionKey::Pane(agent) && r.status == Some(AgentStatus::Working))
    });
    h.keys("cmd-d");
    h.wait_prompt();
    let terminal = h.read(|v, _| v.panes[&agent].0.clone());
    let offset = |h: &Harness| terminal.read_with(&h.cx, |t, _| t.frame().scrollbar.offset);
    let bottom = offset(&h);
    // Read back through the agent's output while working in the split.
    let (at, line) = h.read(|_, cx| {
        let g = terminal.read(cx).geometry.unwrap();
        (
            point(g.origin.x + px(40.0), g.origin.y + px(40.0)),
            g.line_height,
        )
    });
    let wheel = |h: &mut Harness, dy: Pixels| {
        h.cx.simulate_event(gpui::ScrollWheelEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), dy)),
            modifiers: Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
    };
    wheel(&mut h, px(600.0));
    h.wait_for("the agent's scrollback", |_, cx| {
        terminal.read(cx).frame().scrollbar.offset < bottom
    });
    let reading = offset(&h);
    click(&mut h, &selector(SessionKey::Pane(agent)));
    assert_eq!(focused(&h).1, agent, "the row focuses its pane");
    // One more line of history: the terminal handles commands in order, so
    // this lands after anything the click asked for.
    wheel(&mut h, line);
    h.wait_for("the one-line scroll", |_, cx| {
        terminal.read(cx).frame().scrollbar.offset != reading
    });
    assert_eq!(
        offset(&h),
        reading - 1,
        "a Sessions click keeps the agent's scrollback where the user left it"
    );
}

#[gpui::test]
fn restore_reveals_the_focused_worktree_only_inside_project(cx: &mut TestAppContext) {
    let mut f = open(cx, "sessions-restore", 9, "");
    let worktrees = f.worktrees.clone();
    let h = &mut f.h;
    // The newest tab, task-15, is focused; PROJECT starts collapsed next time.
    let target = worktrees[WORKTREES - 9].clone();
    assert_eq!(selected(h).as_ref(), Some(&target));
    click(h, "project-section-toggle");
    let (mut cx2, view2) = h.reopen();
    cx2.simulate_resize(size(px(960.0), px(360.0)));
    view2.update(&mut cx2, |v, cx| {
        v.sidebar
            .update(cx, |s, _| s.model.set_sort(chda_core::SortOrder::Name))
    });
    wait_until(&mut cx2, &view2, "restored worktrees and focus", |v, cx| {
        let s = v.sidebar.read(cx);
        s.model
            .repos
            .first()
            .is_some_and(|r| r.worktrees.len() == WORKTREES + 1)
            && s.selected.is_some()
    });
    let settle2 = |cx2: &mut gpui::VisualTestContext| {
        for _ in 0..3 {
            cx2.run_until_parked();
            cx2.update(|window, cx| window.simulate_next_frame(cx));
        }
        cx2.run_until_parked();
    };
    settle2(&mut cx2);
    view2.read_with(&cx2, |v, cx| {
        let s = v.sidebar.read(cx);
        assert_eq!(
            s.selected.as_ref(),
            Some(&target),
            "PROJECT highlights the restored focus"
        );
        assert!(v.config.project_collapsed, "restore never expands PROJECT");
        assert_eq!(
            s.sessions_scroll.offset().y,
            px(0.0),
            "restore never scrolls Sessions"
        );
    });
    assert!(cx2.debug_bounds("repo-0").is_none());
    let toggle = cx2.debug_bounds("project-section-toggle").unwrap().center();
    cx2.simulate_click(toggle, Modifiers::none());
    settle2(&mut cx2);
    let row = cx2
        .debug_bounds(Box::leak(worktree_row(&target).into_boxed_str()))
        .unwrap();
    let (viewport, offset) = view2.read_with(&cx2, |v, cx| {
        let s = v.sidebar.read(cx);
        (s.project_scroll.bounds(), s.project_scroll.offset().y)
    });
    assert!(
        offset < px(0.0),
        "the restored worktree was below PROJECT's top"
    );
    assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom() + px(0.5));
}
