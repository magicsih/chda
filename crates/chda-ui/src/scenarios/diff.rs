//! A worktree's change size in the sidebar and its read-only diff tab.

use std::path::PathBuf;

use gpui::TestAppContext;

use super::harness::{Harness, git};

#[gpui::test]
fn diff_summary_and_diff_tab(cx: &mut TestAppContext) {
    let mut wt = PathBuf::new();
    let mut h = Harness::open(cx, "diff", |home| {
        let repo = home.repo("app");
        wt = repo.parent().unwrap().join("app.worktrees/feat");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
        );
        std::fs::write(wt.join("notes.txt"), "first line\nsecond line\n").unwrap();
        git(&wt, &["add", "notes.txt"]);
        git(&wt, &["commit", "-q", "-m", "notes"]);
        // An uncommitted edit counts too.
        std::fs::write(wt.join("draft.txt"), "draft\n").unwrap();
        git(&wt, &["add", "draft.txt"]);
        std::fs::write(&home.config, format!("repos = [\"{}\"]\n", repo.display())).unwrap();
    });
    h.wait_prompt();
    h.wait_for("+3 on the worktree", {
        let wt = wt.clone();
        move |v, cx| {
            v.sidebar
                .read(cx)
                .model
                .worktree_for_path(&wt)
                .and_then(|(_, w)| w.diff.clone())
                .is_some_and(|d| {
                    (d.base.as_str(), d.files, d.added, d.removed) == ("main", 2, 3, 0)
                })
        }
    });

    let tabs = h.read(|v, _| v.ws.tabs().len());
    h.cx.update(|window, cx| h.view.update(cx, |v, cx| v.open_diff(&wt, window, cx)));
    h.wait_for("the paged diff in a new tab", move |v, cx| {
        let text = v.focused_text(cx);
        v.ws.tabs().len() == tabs + 1
            && text.contains("2 files changed, 3 insertions(+)")
            && text.contains("+second line")
    });
}
