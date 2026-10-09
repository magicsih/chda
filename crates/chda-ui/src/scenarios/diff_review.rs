use super::harness::{Harness, git};
use crate::sidebar_view::SidebarEvent;
use gpui::{Entity, Modifiers, TestAppContext, point, px};
use std::path::PathBuf;

#[gpui::test]
fn review_is_discoverable_without_replacing_the_diff_pager(cx: &mut TestAppContext) {
    let mut repo = PathBuf::new();
    let mut wt = PathBuf::new();
    let mut h = Harness::open(cx, "diff-review", |home| {
        repo = home.repo("app");
        wt = repo.parent().unwrap().join("app.worktrees/review");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "review",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::write(wt.join("README.md"), "base\nadded\n").unwrap();
        git(&wt, &["add", "README.md"]);
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("diff summary", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .worktree_for_path(&wt)
            .is_some_and(|(_, w)| w.diff.is_some())
    });
    h.cx.update(|window, cx| {
        h.view.update(cx, |v, cx| {
            v.on_sidebar_event(
                SidebarEvent::WorktreeMenu(wt.clone(), point(px(10.0), px(10.0))),
                window,
                cx,
            )
        })
    });
    h.read(|v, _| {
        let items = &v.context_menu.as_ref().unwrap().items;
        assert!(
            items
                .iter()
                .any(|(l, _)| l.starts_with("View diff against"))
        );
        assert!(items.iter().any(|(l, _)| l == "Review diff…"));
    });
}

fn fixture(cx: &mut TestAppContext, name: &str) -> (Harness, PathBuf) {
    let mut wt = PathBuf::new();
    let mut h = Harness::open(cx, name, |home| {
        let repo = home.repo("app");
        wt = repo.parent().unwrap().join("app.worktrees/review");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "review",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::write(wt.join("notes.txt"), "first\nsecond\nthird\n").unwrap();
        git(&wt, &["add", "notes.txt"]);
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("worktree diff", |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .worktree_for_path(&wt)
            .is_some_and(|(_, w)| w.diff.is_some())
    });
    (h, wt)
}
fn open(h: &mut Harness, wt: &std::path::Path) -> Entity<crate::diff_review::DiffReviewView> {
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_review(wt, window, cx)));
    h.wait_for("native review loaded", |v, cx| {
        v.reviews.values().any(|(r, _)| {
            let r = r.read(cx);
            !r.loading && r.diff.is_some() && r.notes.is_some()
        })
    });
    h.read(|v, _| v.reviews[&v.ws.active_tab().unwrap().id].0.clone())
}
fn click(h: &mut Harness, id: &'static str) {
    h.cx.run_until_parked();
    let bounds =
        h.cx.debug_bounds(id)
            .unwrap_or_else(|| panic!("Missing review control {id}"));
    h.cx.simulate_click(bounds.center(), Modifiers::none());
    h.cx.run_until_parked();
}
fn add_note(h: &mut Harness, r: &Entity<crate::diff_review::DiffReviewView>, text: &str) {
    click(h, "review-new-line-1");
    let last = h.cx.debug_bounds("review-new-line-2").unwrap().center();
    h.cx.simulate_click(
        last,
        Modifiers {
            shift: true,
            ..Modifiers::none()
        },
    );
    click(h, "review-add-note");
    h.type_text(text);
    h.keys("enter");
    h.wait_for("saved note", |_, cx| {
        let r = r.read(cx);
        r.editor.is_none()
            && r.notes
                .as_ref()
                .is_some_and(|s| s.notebook.notes.iter().any(|n| n.text == text))
    });
}
fn batch(h: &mut Harness, r: &Entity<crate::diff_review::DiffReviewView>) {
    click(h, "review-send");
    h.wait_for("fresh review batch", |_, cx| {
        !r.read(cx).loading && r.read(cx).batch.is_some()
    });
}

#[gpui::test]
fn review_range_notes_survive_refresh_edit_and_session_restore(cx: &mut TestAppContext) {
    let (mut h, wt) = fixture(cx, "review-persist");
    let panes = h.read(|v, _| v.panes.len());
    let r = open(&mut h, &wt);
    add_note(&mut h, &r, "Keep this | 한글 🧭");
    assert_eq!(h.read(|v, _| v.panes.len()), panes);
    let note = r.read_with(&h.cx, |r, _| {
        r.notes.as_ref().unwrap().notebook.notes[0].clone()
    });
    assert_eq!((note.anchor.first, note.anchor.last), (1, 2));
    assert!(!note.resolved);
    open(&mut h, &wt);
    assert_eq!(h.read(|v, _| v.ws.tabs().len()), 2);
    std::fs::write(wt.join("notes.txt"), "changed\nsecond\nthird\n").unwrap();
    click(&mut h, "review-refresh");
    h.wait_for("changed diff", |_, cx| !r.read(cx).loading);
    assert!(r.read_with(&h.cx, |r, _| {
        note.anchor.reattach(r.diff.as_ref().unwrap()).is_none()
    }));
    h.cx.update(|window, cx| {
        r.update(cx, |v, cx| {
            v.edit_note(Some(note.clone()), false, window, cx)
        })
    });
    h.keys("cmd-a");
    h.type_text("Edited stale note | no relocation");
    click(&mut h, "review-refresh");
    h.wait_for("refresh preserves draft", |_, cx| !r.read(cx).loading);
    assert_eq!(
        r.read_with(&h.cx, |r, cx| r
            .editor
            .as_ref()
            .unwrap()
            .input
            .read(cx)
            .text()
            .to_owned()),
        "Edited stale note | no relocation"
    );
    click(&mut h, "review-note-save");
    h.wait_for("saved edit", |_, cx| r.read(cx).editor.is_none());
    let edited = r.read_with(&h.cx, |r, _| {
        r.notes.as_ref().unwrap().notebook.notes[0].clone()
    });
    assert_eq!(edited.anchor, note.anchor);
    assert_eq!(edited.text, "Edited stale note | no relocation");
    let (mut other, view) = h.reopen();
    super::harness::wait_until(&mut other, &view, "restored review", |v, cx| {
        v.reviews.values().any(|(r, _)| {
            r.read(cx)
                .notes
                .as_ref()
                .is_some_and(|s| s.notebook.notes.iter().any(|n| n == &edited))
        })
    });
    assert_eq!(view.read_with(&other, |v, _| v.panes.len()), panes);
}

#[gpui::test]
fn review_copy_checks_diff_and_conversation_and_preserves_unsent_input(cx: &mut TestAppContext) {
    use chda_core::agents::HookKind;
    let (mut h, wt) = fixture(cx, "review-delivery");
    h.run(&format!("cd '{}'", wt.display()), "test%");
    h.wait_for("terminal entered the actual worktree", |v, _| {
        v.ws.focused_pane()
            .and_then(|p| v.ws.pane(p))
            .is_some_and(|p| p.cwd.as_ref() == Some(&wt))
    });
    let pane = h.read(|v, _| v.ws.focused_pane().unwrap());
    h.type_text("echo unsent-review-input");
    h.hook_session(Some(pane.raw()), &wt, HookKind::SessionStart, "exact-one");
    h.hook_session(
        Some(pane.raw()),
        &wt,
        HookKind::PromptSubmitted,
        "exact-one",
    );
    h.wait_for("live conversation", |v, _| {
        v.ws.pane(pane).is_some_and(|p| {
            p.agent_live
                && p.agent_session
                    .as_ref()
                    .is_some_and(|s| s.session == "exact-one")
        })
    });
    let r = open(&mut h, &wt);
    add_note(&mut h, &r, "Do not overwrite the agent prompt");
    batch(&mut h, &r);
    assert_eq!(
        r.read_with(&h.cx, |r, _| r.batch.as_ref().unwrap().targets.len()),
        1
    );
    click(&mut h, "review-target-0");
    h.cx.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("keep clipboard".into()))
    });
    std::fs::write(wt.join("notes.txt"), "first\nsecond\nthird\nfourth\n").unwrap();
    click(&mut h, "review-copy-agent");
    h.wait_for("changed diff rejected", |_, cx| {
        r.read(cx)
            .error
            .as_ref()
            .is_some_and(|s| s.contains("changed"))
            && !r.read(cx).loading
    });
    assert_eq!(
        h.read(|_, cx| cx.read_from_clipboard().unwrap().text())
            .as_deref(),
        Some("keep clipboard")
    );
    batch(&mut h, &r);
    click(&mut h, "review-target-0");
    h.hook_session(Some(pane.raw()), &wt, HookKind::SessionStart, "exact-two");
    h.wait_for("replacement conversation", |v, _| {
        v.ws.pane(pane)
            .unwrap()
            .agent_session
            .as_ref()
            .is_some_and(|s| s.session == "exact-two")
    });
    click(&mut h, "review-copy-agent");
    h.wait_for("changed conversation rejected", |_, cx| {
        r.read(cx)
            .error
            .as_ref()
            .is_some_and(|s| s.contains("target conversation changed"))
    });
    assert_eq!(
        h.read(|_, cx| cx.read_from_clipboard().unwrap().text())
            .as_deref(),
        Some("keep clipboard")
    );
    batch(&mut h, &r);
    click(&mut h, "review-target-0");
    click(&mut h, "review-copy-agent");
    h.wait_for("exact pane focused after copy", |v, _| {
        v.ws.focused_pane() == Some(pane)
    });
    h.wait_for("copy marker saved", |_, cx| {
        r.read(cx).notes.as_ref().unwrap().notebook.notes[0]
            .copied
            .as_ref()
            .is_some_and(|c| c.session == "exact-two")
    });
    assert!(
        h.read(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("Do not overwrite the agent prompt")
    );
    assert!(!r.read_with(&h.cx, |r, _| {
        r.notes.as_ref().unwrap().notebook.notes[0].resolved
    }));
    h.type_text("-preserved");
    h.keys("enter");
    h.wait_for("original input preserved", |v, cx| {
        v.focused_text(cx).contains("unsent-review-input-preserved")
    });
}

#[gpui::test]
fn review_keeps_a_failed_save_draft_and_can_copy_it(cx: &mut TestAppContext) {
    let (mut h, wt) = fixture(cx, "review-save-error");
    let r = open(&mut h, &wt);
    std::fs::create_dir(h.home.data.join("review-notes.json")).unwrap();
    click(&mut h, "review-new-line-1");
    click(&mut h, "review-add-note");
    h.type_text("A draft | kept after save failure");
    h.keys("enter");
    h.wait_for("save failed with draft intact", |_, cx| {
        r.read(cx)
            .error
            .as_ref()
            .is_some_and(|e| e.starts_with("Save failed"))
    });
    assert!(r.read_with(&h.cx, |r, _| r.pending()));
    click(&mut h, "review-note-copy-draft");
    assert_eq!(
        h.read(|_, cx| cx.read_from_clipboard().unwrap().text())
            .as_deref(),
        Some("A draft | kept after save failure")
    );
    assert!(h.home.data.join("review-notes.json").is_dir());
    assert!(r.read_with(&h.cx, |r, _| {
        r.notes.as_ref().unwrap().notebook.notes.is_empty()
    }));
}
