//! Pull request badges per repository host, through a stand-in `gh`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use gpui::TestAppContext;

use super::harness::{Harness, Home, git};

/// A `gh` logged in to github.com only, with PR #5 for `feat` in `org/app`.
fn fake_gh(home: &Home) {
    std::fs::write(
        &home.gh,
        r#"#!/bin/sh
case "$1 $2" in
"--version ") exit 0 ;;
"auth status") [ "$4" = "github.com" ] ;;
"pr list")
  if [ "$4" = "github.com/org/app" ] && [ "$6" = "feat" ]; then
    echo '[{"number":5,"state":"OPEN","url":"https://github.com/org/app/pull/5","statusCheckRollup":[]}]'
  else
    echo '[]'
  fi ;;
*) exit 1 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&home.gh, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn repo_with_branch(home: &Home, name: &str, remote: &str) -> PathBuf {
    let repo = home.repo(name);
    git(&repo, &["remote", "add", "origin", remote]);
    let wt = repo
        .parent()
        .unwrap()
        .join(format!("{name}.worktrees/feat"));
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );
    repo
}

#[gpui::test]
fn badges_per_host_and_a_login_hint_for_the_other(cx: &mut TestAppContext) {
    let (mut app, mut ent) = (PathBuf::new(), PathBuf::new());
    let mut h = Harness::open(cx, "prs", |home| {
        fake_gh(home);
        app = repo_with_branch(home, "app", "https://github.com/org/app.git");
        // An SSH alias the config maps to an Enterprise host gh is not
        // logged in to.
        ent = repo_with_branch(home, "ent", "git@github-work:org/ent.git");
        std::fs::write(
            &home.config,
            format!(
                "repos = [\"{}\", \"{}\"]\n[repo-hosts]\n\"{}\" = \"ghe.example.com\"\n",
                app.display(),
                ent.display(),
                ent.display()
            ),
        )
        .unwrap();
    });
    let feat = app.parent().unwrap().join("app.worktrees/feat");
    h.wait_for("PR #5 on app's feat worktree", move |v, cx| {
        v.sidebar
            .read(cx)
            .model
            .worktree_for_path(&feat)
            .and_then(|(_, w)| w.pr.as_ref())
            .is_some_and(|pr| pr.number == 5 && pr.url.starts_with("https://github.com/"))
    });
    let hint = chda_core::login_hint("ghe.example.com");
    h.wait_for("the login hint on the Enterprise repository", {
        let ent = ent.clone();
        move |v, cx| {
            let model = &v.sidebar.read(cx).model;
            let repo = |p: &Path| model.repos.iter().find(|r| r.path == p).unwrap();
            repo(&ent).pr_hint.as_deref() == Some(hint.as_str())
        }
    });
    h.read(|v, cx| {
        let model = &v.sidebar.read(cx).model;
        let app_repo = model.repos.iter().find(|r| r.path == app).unwrap();
        assert_eq!(app_repo.pr_hint, None);
        let ent_repo = model.repos.iter().find(|r| r.path == ent).unwrap();
        assert!(ent_repo.worktrees.iter().all(|w| w.pr.is_none()));
    });
}
