//! Session identicons (#272): a small picture before each Claude Code and
//! Codex session's name, enlarged by a click, that keeps its place in the
//! row and never activates the session.

use chda_core::agents::HookKind;
use chda_core::{Identicon, PaneId, SessionKey};
use gpui::{Bounds, Modifiers, Pixels, TestAppContext, point, px};

use super::harness::Harness;
use super::sidebar::draw_sidebar_frame;

/// Two Claude Code conversations whose ids differ only in the last
/// character, and a Codex conversation that reuses the first one's id.
const CLAUDE_A: &str = "0199c2d4-5b1e-7f3a-9c60-2a8e4d1b7f05";
const CLAUDE_B: &str = "0199c2d4-5b1e-7f3a-9c60-2a8e4d1b7f06";

fn bounds(h: &mut Harness, selector: &str) -> Option<Bounds<Pixels>> {
    // Geometry exists only for what a frame has drawn.
    draw_sidebar_frame(h);
    h.cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

fn row(pane: PaneId) -> String {
    format!("session-pane-{}", pane.raw())
}

fn icon(pane: PaneId) -> String {
    format!("session-identicon-pane-{}", pane.raw())
}

fn focused(h: &Harness) -> PaneId {
    h.read(|v, _| v.ws.focused_pane().unwrap())
}

/// Four live agents in one tab with the same display name: Claude Code A and
/// B, Codex with A's id, and Gemini CLI, which has no picture. Gemini's row is
/// the same row without the icon, to measure the icon against.
fn four_agents(h: &mut Harness) -> [PaneId; 4] {
    let cwd = h.home.home.clone();
    let mut panes = vec![focused(h)];
    for _ in 0..3 {
        h.keys("cmd-d");
        h.wait_prompt();
        panes.push(focused(h));
    }
    let [a, b, c, d] = [panes[0], panes[1], panes[2], panes[3]];
    h.hook_session(Some(a.raw()), &cwd, HookKind::SessionStart, CLAUDE_A);
    h.hook_session(Some(b.raw()), &cwd, HookKind::SessionStart, CLAUDE_B);
    h.hook_from(
        "codex",
        Some(c.raw()),
        &cwd,
        HookKind::SessionStart,
        CLAUDE_A,
    );
    h.hook_from("gemini", Some(d.raw()), &cwd, HookKind::SessionStart, "g1");
    h.wait_for("four live agents", move |v, cx| {
        let rows = &v.sidebar.read(cx).model.sessions;
        [a, b, c, d].iter().all(|p| {
            rows.iter()
                .any(|r| r.key == SessionKey::Pane(*p) && r.agent_live)
        }) && rows.iter().filter(|r| r.identity.is_some()).count() == 3
    });
    [a, b, c, d]
}

fn identity(h: &Harness, pane: PaneId) -> Option<(String, String)> {
    h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .find(|r| r.key == SessionKey::Pane(pane))
            .and_then(|r| r.identity.clone())
            .map(|i| (i.provider, i.id))
    })
}

fn picture(h: &Harness, pane: PaneId) -> Identicon {
    let (provider, id) = identity(h, pane).expect("a conversation");
    Identicon::new(&provider, &id)
}

fn preview(h: &Harness) -> Option<(String, String)> {
    h.read(|v, _| {
        v.identicon_preview
            .as_ref()
            .map(|p| (p.identity.provider.clone(), p.identity.id.clone()))
    })
}

fn click(h: &mut Harness, selector: &str) {
    let at = bounds(h, selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center();
    h.cx.simulate_click(at, Modifiers::none());
    h.cx.run_until_parked();
}

/// Typing goes to the focused pane's shell: the command arrives whole.
fn typing_reaches_the_shell(h: &mut Harness, word: &str) {
    h.type_text(&format!("echo {}''{}", &word[..2], &word[2..]));
    h.keys("enter");
    let word = word.to_owned();
    h.wait_for(&format!("{word} printed"), move |v, cx| {
        v.focused_text(cx).lines().any(|line| line.trim() == word)
    });
}

#[gpui::test]
fn same_named_sessions_get_their_own_picture_in_rows_as_tall_as_before(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "identicon-rows", |_| {});
    h.wait_prompt();
    let [a, b, c, d] = four_agents(&mut h);

    // One name for all of them, and still four separate faces.
    let titles = h.read(|v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .filter(|r| matches!(r.key, SessionKey::Pane(_)))
            .map(|r| r.title.clone())
            .collect::<Vec<_>>()
    });
    assert!(titles.windows(2).all(|w| w[0] == w[1]), "{titles:?}");
    let (pa, pb, pc) = (picture(&h, a), picture(&h, b), picture(&h, c));
    assert!(pa != pb && pa != pc && pb != pc, "three different faces");
    assert_eq!(identity(&h, d), None, "Gemini CLI has none");

    // The picture comes before the name, inside the row, without wrapping.
    for pane in [a, b, c] {
        let (row, icon, label) = (
            bounds(&mut h, &row(pane)).unwrap(),
            bounds(&mut h, &icon(pane)).unwrap(),
            bounds(&mut h, &format!("session-label-pane-{}", pane.raw())).unwrap(),
        );
        assert_eq!((icon.size.width, icon.size.height), (px(15.0), px(15.0)));
        assert!(icon.right() <= label.left(), "{icon:?} before {label:?}");
        assert!(icon.top() >= row.top() && icon.bottom() <= row.bottom());
        assert!(label.size.width > px(48.0), "{label:?} keeps useful space");
    }

    // The icon adds no height: every row is as tall as Gemini's, which has none.
    let plain = bounds(&mut h, &row(d)).unwrap().size.height;
    for pane in [a, b, c] {
        assert_eq!(
            bounds(&mut h, &row(pane)).unwrap().size.height,
            plain,
            "a row with a picture is as tall as one without"
        );
    }
}

#[gpui::test]
fn clicking_a_picture_enlarges_it_and_activates_nothing(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "identicon-preview", |_| {});
    h.wait_prompt();
    let [a, _, c, _] = four_agents(&mut h);
    let working = focused(&h);
    assert_ne!(working, a, "the clicked row is not the focused one");
    let rows_before: Vec<_> = [a, c]
        .iter()
        .map(|p| bounds(&mut h, &row(*p)).unwrap())
        .collect();
    let scroll_before = h.read(|v, cx| v.sidebar.read(cx).sessions_scroll.offset());

    // The click opens the preview of that conversation, and only that.
    click(&mut h, &icon(a));
    assert_eq!(preview(&h), Some(("claude".into(), CLAUDE_A.into())));
    assert_eq!(focused(&h), working, "the session was not activated");
    let large = bounds(&mut h, "identicon-preview-icon").expect("the enlargement");
    assert_eq!(
        (large.size.width, large.size.height),
        (px(160.0), px(160.0))
    );
    for part in ["identicon-preview-provider", "identicon-preview-id"] {
        assert!(bounds(&mut h, part).is_some(), "{part} is shown");
    }
    // Nothing under it moved.
    for (pane, before) in [a, c].iter().zip(&rows_before) {
        assert_eq!(bounds(&mut h, &row(*pane)).as_ref(), Some(before));
    }
    assert_eq!(
        h.read(|v, cx| v.sidebar.read(cx).sessions_scroll.offset()),
        scroll_before
    );

    // Escape closes it, and typing goes to the pane that had the input.
    h.keys("escape");
    assert_eq!(preview(&h), None);
    assert_eq!(focused(&h), working);
    typing_reaches_the_shell(&mut h, "AFTERESC");

    // The Close button, and a click outside that does nothing else.
    click(&mut h, &icon(c));
    assert_eq!(preview(&h), Some(("codex".into(), CLAUDE_A.into())));
    click(&mut h, "identicon-preview-close");
    assert_eq!(preview(&h), None);
    typing_reaches_the_shell(&mut h, "AFTERCLOSE");

    click(&mut h, &icon(a));
    assert!(preview(&h).is_some());
    let outside = point(px(900.0), px(440.0));
    h.cx.simulate_click(outside, Modifiers::none());
    h.cx.run_until_parked();
    assert_eq!(preview(&h), None, "a click outside closes it");
    assert_eq!(focused(&h), working, "and reaches nothing underneath");
    typing_reaches_the_shell(&mut h, "AFTEROUT");

    // The rest of the row still works as before.
    click(&mut h, &format!("session-label-pane-{}", a.raw()));
    assert_eq!(focused(&h), a);
    assert_eq!(preview(&h), None);
}

#[gpui::test]
fn the_palette_shows_the_focused_sessions_picture_from_the_keyboard(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "identicon-palette", |_| {});
    h.wait_prompt();
    let [a, _, _, d] = four_agents(&mut h);
    let offered = |h: &Harness| {
        h.read(|v, cx| {
            v.palette_items(cx)
                .iter()
                .any(|item| item.label == "Show session icon")
        })
    };

    // Gemini CLI, which has focus, has no picture to show.
    assert_eq!(focused(&h), d);
    assert!(!offered(&h));

    click(&mut h, &format!("session-label-pane-{}", a.raw()));
    assert_eq!(focused(&h), a);
    assert!(offered(&h));
    h.keys("cmd-shift-p");
    h.type_text("Show session icon");
    h.keys("enter");
    h.cx.run_until_parked();
    assert_eq!(preview(&h), Some(("claude".into(), CLAUDE_A.into())));
    assert!(bounds(&mut h, "identicon-preview-icon").is_some());

    // Escape closes it and the keyboard is back in the same pane.
    h.keys("escape");
    assert_eq!(preview(&h), None);
    assert_eq!(focused(&h), a);
    typing_reaches_the_shell(&mut h, "KEYBOARD");
}

#[gpui::test]
fn a_conversation_keeps_its_picture_and_a_narrow_sidebar_its_label(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "identicon-stable", |home| {
        std::fs::write(&home.config, "sidebar-width = 180\n").unwrap();
    });
    h.wait_prompt();
    let [a, b, c, d] = four_agents(&mut h);
    let cwd = h.home.home.clone();
    let before: Vec<_> = [a, b, c]
        .iter()
        .map(|p| (identity(&h, *p), picture(&h, *p)))
        .collect();

    // Working, finished and resumed under the same id; another focus.
    h.hook_session(Some(a.raw()), &cwd, HookKind::PromptSubmitted, CLAUDE_A);
    h.wait_for("a working", move |v, cx| {
        v.sidebar.read(cx).model.sessions.iter().any(|r| {
            r.key == SessionKey::Pane(a) && r.status == Some(chda_core::AgentStatus::Working)
        })
    });
    h.hook_session(Some(a.raw()), &cwd, HookKind::Stopped, CLAUDE_A);
    h.hook_session(Some(b.raw()), &cwd, HookKind::SessionEnd, CLAUDE_B);
    h.hook_session(Some(b.raw()), &cwd, HookKind::SessionStart, CLAUDE_B);
    h.keys("cmd-alt-left");
    h.cx.run_until_parked();
    let after: Vec<_> = [a, b, c]
        .iter()
        .map(|p| (identity(&h, *p), picture(&h, *p)))
        .collect();
    assert_eq!(
        before, after,
        "status, focus and resuming keep each picture"
    );

    // 180px, the narrowest sidebar: no pictures, and rows exactly as before.
    let plain = bounds(&mut h, &row(d)).unwrap();
    let plain_label = bounds(&mut h, &format!("session-label-pane-{}", d.raw())).unwrap();
    for pane in [a, b, c] {
        assert_eq!(bounds(&mut h, &icon(pane)), None, "no picture at 180px");
        let row = bounds(&mut h, &row(pane)).unwrap();
        let label = bounds(&mut h, &format!("session-label-pane-{}", pane.raw())).unwrap();
        assert_eq!(row.size.height, plain.size.height);
        assert!(
            (label.size.width - plain_label.size.width).abs() < px(1.0),
            "the name keeps all its room: {label:?} against {plain_label:?}"
        );
    }

    // 200px: the pictures are back, the rows are as tall as before, and a
    // label gives up only the picture's own room and truncates, not wraps.
    h.view.update(&mut h.cx, |v, cx| {
        v.config.sidebar_width = 200;
        cx.notify();
    });
    for _ in 0..3 {
        draw_sidebar_frame(&mut h);
    }
    let plain = bounds(&mut h, &row(d)).unwrap();
    let plain_label = bounds(&mut h, &format!("session-label-pane-{}", d.raw())).unwrap();
    for pane in [a, b, c] {
        let (row, icon, label) = (
            bounds(&mut h, &row(pane)).unwrap(),
            bounds(&mut h, &icon(pane)).expect("a picture at 200px"),
            bounds(&mut h, &format!("session-label-pane-{}", pane.raw())).unwrap(),
        );
        assert_eq!(row.size.height, plain.size.height);
        assert!(icon.left() >= row.left() && icon.right() <= label.left());
        assert!(label.size.height < row.size.height, "the label is one line");
        // The picture (15px) and the gap after it (4px).
        let taken = plain_label.size.width - label.size.width;
        assert!(
            taken <= px(19.5),
            "the picture took {taken:?}: {label:?} against {plain_label:?}"
        );
    }
}

#[gpui::test]
fn an_unconfirmed_identity_is_neutral_until_the_provider_reports_its_id(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, "identity-pending", |home| home.fake_claude());
    h.wait_prompt();
    h.cx.dispatch_action(crate::workspace_view::RunClaude);
    h.confirm_agent_launch();
    h.wait_for("managed agent started", |v, cx| {
        v.focused_text(cx).contains("fake-claude")
    });
    let pane = focused(&h);
    let cwd = h.home.home.clone();
    h.wait_for("agent without an id", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .sessions
            .iter()
            .any(|r| r.key == SessionKey::Pane(pane) && r.identity.is_none())
    });
    h.keys("cmd-t");
    h.wait_prompt();
    let before = focused(&h);
    let pending = format!("session-identity-pending-pane-{}", pane.raw());
    let initial = bounds(&mut h, &pending).expect("neutral icon");
    let row_before = bounds(&mut h, &row(pane)).unwrap();
    click(&mut h, &pending);
    assert_eq!(focused(&h), before, "neutral icon never navigates");
    assert!(preview(&h).is_none());
    h.hook_session(Some(pane.raw()), &cwd, HookKind::SessionStart, CLAUDE_A);
    h.wait_for("real conversation id", |v, cx| {
        v.sidebar.read(cx).model.sessions.iter().any(|r| {
            r.key == SessionKey::Pane(pane) && r.identity.as_ref().is_some_and(|i| i.id == CLAUDE_A)
        })
    });
    assert_eq!(bounds(&mut h, &icon(pane)).unwrap(), initial);
    assert_eq!(bounds(&mut h, &row(pane)).unwrap(), row_before);
    assert!(bounds(&mut h, &pending).is_none());
    h.view.update(&mut h.cx, |v, cx| {
        v.config.sidebar_width = 199;
        cx.notify();
    });
    assert!(bounds(&mut h, &icon(pane)).is_none());
}
