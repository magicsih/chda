//! `chda note` inside a worktree prints and sets its branch's note.

use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn chda(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chda"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn note_prints_and_sets_the_branch_description() {
    let root = std::env::temp_dir().join(format!("chda-note-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = root.join("app.worktrees/feat");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );

    let out = chda(&wt, &["note"]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "no note yet");

    assert!(
        chda(&wt, &["note", "Fix", "the login loop"])
            .status
            .success()
    );
    let out = chda(&wt, &["note"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Fix the login loop\n");
    assert_eq!(
        git(&repo, &["config", "branch.feat.description"]).trim_end(),
        "Fix the login loop"
    );
    assert!(git(&repo, &["config", "--get-regexp", "description"]).contains("branch.feat."));

    assert!(chda(&wt, &["note", ""]).status.success());
    assert!(chda(&wt, &["note"]).stdout.is_empty(), "removed");

    let out = chda(&root, &["note"]);
    assert!(!out.status.success(), "outside a repository");
    std::fs::remove_dir_all(&root).unwrap();
}
