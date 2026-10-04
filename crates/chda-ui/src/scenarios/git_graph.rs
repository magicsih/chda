//! Read-only repository history alongside real terminal tabs (#107).
use super::harness::{Harness, git, wait_until};
use crate::git_graph::GitGraphView;
use crate::sidebar_view::SidebarEvent;
use crate::workspace_view::MenuAction;
use chda_core::{CommitRefKind, HistoryPage, SavedWindow};
use gpui::{App, Entity, TestAppContext};
use std::path::PathBuf;

fn active_graph(h: &Harness) -> Entity<GitGraphView> {
    h.read(|view, _| view.graphs[&view.ws.active_tab().unwrap().id].clone())
}
fn open(h: &mut Harness, repo: PathBuf) {
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |view, cx| view.open_git_graph(repo, window, cx))
    });
}
fn loaded(h: &mut Harness) {
    h.wait_for("graph page", |view, cx| {
        !view.graphs[&view.ws.active_tab().unwrap().id]
            .read(cx)
            .loading
    });
}
fn graph_has(view: &crate::workspace_view::WorkspaceView, cx: &App, subject: &str) -> bool {
    view.graphs.values().any(|graph| {
        graph
            .read(cx)
            .rows
            .iter()
            .any(|row| row.commit.subject == subject)
    })
}

#[gpui::test]
fn graph_menu_merges_refs_refresh_and_mixed_restore(cx: &mut TestAppContext) {
    let mut repo = PathBuf::new();
    let mut h = Harness::open(cx, "graph", |home| {
        repo = home.repo("app");
        git(&repo, &["checkout", "-q", "-b", "feature"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "feature"]);
        git(&repo, &["tag", "-a", "v1", "-m", "annotated"]);
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/feature", "HEAD"],
        );
        git(&repo, &["checkout", "-q", "main"]);
        git(
            &repo,
            &["commit", "-q", "--allow-empty", "-m", "main change"],
        );
        git(
            &repo,
            &["merge", "-q", "--no-ff", "feature", "-m", "merge feature"],
        );
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("registered repo", |view, cx| {
        !view.sidebar.read(cx).model.repos[0].worktrees.is_empty()
    });
    let panes = h.read(|view, _| view.panes.len());
    h.cx.update(|window, cx| {
        h.view.update(cx, |view, cx| {
            view.on_sidebar_event(
                SidebarEvent::RepoMenu(repo.clone(), gpui::point(gpui::px(10.0), gpui::px(10.0))),
                window,
                cx,
            )
        })
    });
    let action = h.read(|view, _| {
        view.context_menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|(label, _)| label == "View Git tree")
            .unwrap()
            .1
            .clone()
    });
    assert!(matches!(action, MenuAction::ViewGitTree(_)));
    h.cx.update(|window, cx| {
        h.view
            .update(cx, |view, cx| view.run_menu_action(action, window, cx))
    });
    loaded(&mut h);
    let graph = active_graph(&h);
    graph.read_with(&h.cx, |view, _| {
        assert_eq!(view.repo, repo);
        assert!(view.error.is_none(), "{:?}", view.error);
        assert_eq!(view.rows.len(), 4);
        assert_eq!(view.rows[0].commit.subject, "merge feature");
        assert_eq!(view.rows[0].commit.parents.len(), 2);
        assert!(view.rows.iter().any(|row| row.columns == 2));
        let feature = view
            .rows
            .iter()
            .find(|row| row.commit.subject == "feature")
            .unwrap();
        for kind in [
            CommitRefKind::LocalBranch,
            CommitRefKind::RemoteBranch,
            CommitRefKind::Tag,
        ] {
            assert!(
                feature
                    .commit
                    .refs
                    .iter()
                    .any(|reference| reference.kind == kind)
            );
        }
        assert!(
            view.rows[0]
                .commit
                .refs
                .iter()
                .any(|reference| reference.kind == CommitRefKind::Head)
        );
    });
    assert_eq!(h.read(|view, _| view.panes.len()), panes);
    h.keys("cmd-d cmd-shift-d cmd-alt-left cmd-] cmd-shift-enter");
    assert_eq!(h.read(|view, _| view.panes.len()), panes);
    assert!(h.read(|view, _| view.ws.focused_pane().is_none()));
    open(&mut h, repo.clone());
    assert_eq!(h.read(|view, _| view.ws.tabs().len()), 2);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "new work"]);
    let old_generation = graph.read_with(&h.cx, |view, _| view.generation);
    graph.update(&mut h.cx, |view, cx| {
        view.refresh(cx);
        view.apply_page(
            old_generation,
            Ok(HistoryPage {
                commits: Vec::new(),
                has_more: false,
            }),
            cx,
        );
        assert!(
            view.loading,
            "an obsolete page must not finish the new query"
        );
    });
    h.wait_for("refreshed history", |view, cx| {
        graph_has(view, cx, "new work")
    });
    let graph_id = h.read(|view, _| view.ws.active_tab().unwrap().id);
    h.cx.update(|window, cx| {
        h.view.update(cx, |view, cx| {
            view.ws.rename_tab(graph_id, "Project history");
            view.focus(window, cx);
        })
    });
    // A terminal, graph, terminal and second graph must retain their order.
    h.keys("cmd-t");
    h.wait_prompt();
    let other = h.home.repo("other");
    open(&mut h, other);
    h.cx.update(|window, cx| {
        h.view.update(cx, |view, cx| {
            view.ws.activate_tab_id(graph_id);
            view.focus(window, cx);
        })
    });
    let snapshot = h.read(|view, _| view.ws.snapshot());
    let (mut cx2, view2) = h.reopen();
    wait_until(&mut cx2, &view2, "restored graphs", |view, cx| {
        view.graphs.len() == 2 && graph_has(view, cx, "new work")
    });
    view2.read_with(&cx2, |view, _| {
        assert_eq!(view.ws.snapshot(), snapshot);
        assert_eq!(view.panes.len(), 2);
    });
    // Closing a graph does not terminate the original shell or another graph.
    drop(graph);
    h.keys("cmd-w");
    assert_eq!(h.read(|view, _| view.graphs.len()), 1);
    assert_eq!(h.read(|view, _| view.panes.len()), 2);
    assert_eq!(h.read(|view, _| view.ws.snapshot().tabs.len()), 3);
}

#[gpui::test]
fn graph_pages_keep_connections_and_queries_can_retry(cx: &mut TestAppContext) {
    let mut repo = PathBuf::new();
    let mut h = Harness::open(cx, "graph-pages", |home| {
        repo = home.repo("history");
        for i in 0..205 {
            git(
                &repo,
                &["commit", "-q", "--allow-empty", "-m", &format!("item {i}")],
            );
        }
    });
    h.wait_prompt();
    open(&mut h, repo.clone());
    loaded(&mut h);
    let graph = active_graph(&h);
    graph.read_with(&h.cx, |view, _| {
        assert_eq!(view.rows.len(), 200);
        assert!(view.has_more);
    });
    let bounds = h.cx.debug_bounds("git-graph").expect("graph is rendered");
    h.cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-12000.0))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    h.wait_for("the next page on scroll", |view, cx| {
        view.graphs
            .values()
            .any(|graph| graph.read(cx).rows.len() == 206)
    });
    graph.read_with(&h.cx, |view, _| {
        assert!(!view.has_more);
        assert_eq!(view.rows[199].commit.parents[0], view.rows[200].commit.id);
        assert!(
            view.rows[200]
                .lines
                .iter()
                .any(|line| line.from == (0, 0.0) && line.to == (0, 0.5))
        );
    });
    let empty = h.home.home.join("empty");
    std::fs::create_dir(&empty).unwrap();
    git(&empty, &["init", "-q", "-b", "main"]);
    open(&mut h, empty.clone());
    loaded(&mut h);
    assert!(active_graph(&h).read_with(&h.cx, |view, _| view.rows.is_empty()));
    assert!(active_graph(&h).read_with(&h.cx, |view, _| view.error.is_none()));
    let missing = h.home.home.join("missing");
    open(&mut h, missing.clone());
    loaded(&mut h);
    let broken = active_graph(&h);
    assert!(broken.read_with(&h.cx, |view, _| view.error.is_some()));
    std::fs::create_dir(&missing).unwrap();
    git(&missing, &["init", "-q", "-b", "main"]);
    git(
        &missing,
        &["commit", "-q", "--allow-empty", "-m", "recovered"],
    );
    let bounds = h.cx.debug_bounds("graph-retry").expect("retry is visible");
    h.cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    h.wait_for("successful retry", |view, cx| {
        graph_has(view, cx, "recovered")
    });
    assert!(broken.read_with(&h.cx, |view, _| view.error.is_none()));
    let old = broken.read_with(&h.cx, |view, _| view.generation);
    broken.update(&mut h.cx, |view, cx| view.refresh(cx));
    h.keys("cmd-w");
    // The graph entity may still be held by this test, but is no longer in the workspace.
    broken.update(&mut h.cx, |view, cx| {
        view.apply_page(old, Err(std::io::Error::other("obsolete")), cx)
    });
    assert!(!h.read(|view, _| {
        view.graphs
            .values()
            .any(|graph| graph.entity_id() == broken.entity_id())
    }));
    assert_eq!(h.read(|view, _| view.ws.tabs().len()), 3);
    assert_eq!(SavedWindow::load(&h.home.data).unwrap().tabs.len(), 3);
}
