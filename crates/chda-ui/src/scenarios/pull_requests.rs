//! Pull request badges per repository host, through a stand-in `gh`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chda_core::{CheckState, PrState};
use gpui::TestAppContext;

use super::harness::{Harness, Home, git};

/// A `gh` logged in to github.com only, with PR #5 for `feat` in `org/app`.
fn fake_gh(home: &Home) {
    let gh = home.bin.join("gh");
    std::fs::create_dir_all(&home.bin).unwrap();
    std::fs::write(
        &gh,
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
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
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
    let hint = chda_core::login_hint(chda_core::Forge::GitHub, "ghe.example.com");
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

fn script(home: &Home, name: &str, body: &str) {
    std::fs::create_dir_all(&home.bin).unwrap();
    let path = home.bin.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[gpui::test]
fn gitlab_and_gitea_repositories_get_badges(cx: &mut TestAppContext) {
    let (mut lab, mut tea) = (PathBuf::new(), PathBuf::new());
    let mut h = Harness::open(cx, "forges", |home| {
        // glab: logged in to gitlab.com; MR !12 for `feat`, pipeline failed.
        script(
            home,
            "glab",
            r#"case "$1 $2" in
"--version ") exit 0 ;;
"auth status") [ "$4" = "gitlab.com" ] ;;
"mr list")
  [ "$GITLAB_HOST" = "gitlab.com" ] && [ "$4" = "group/lab" ] && [ "$6" = "feat" ] || exit 1
  echo '[{"iid":12,"state":"opened","source_branch":"feat","web_url":"https://gitlab.com/group/lab/-/merge_requests/12"}]' ;;
"mr view") echo '{"iid":12,"state":"opened","head_pipeline":{"status":"failed"}}' ;;
*) exit 1 ;;
esac
"#,
        );
        // tea: a login for codeberg.org; PR 3 for `feat`, merged.
        script(
            home,
            "tea",
            r#"case "$1 $2" in
"--version ") exit 0 ;;
"logins list") echo '[{"name":"cb","url":"https://codeberg.org","user":"me","default":"true"}]' ;;
"pulls list")
  [ "$4" = "cb" ] && [ "$6" = "org/tea" ] || exit 1
  echo '[{"index":"3","state":"merged","head":"feat","url":"https://codeberg.org/org/tea/pulls/3","ci":"success"}]' ;;
*) exit 1 ;;
esac
"#,
        );
        lab = repo_with_branch(home, "lab", "https://gitlab.com/group/lab.git");
        tea = repo_with_branch(home, "tea", "https://codeberg.org/org/tea.git");
        std::fs::write(
            &home.config,
            format!("repos = [\"{}\", \"{}\"]\n", lab.display(), tea.display()),
        )
        .unwrap();
    });
    let pr_of = |repo: &Path, name: &str| {
        repo.parent()
            .unwrap()
            .join(format!("{name}.worktrees/feat"))
    };
    let (lab_feat, tea_feat) = (pr_of(&lab, "lab"), pr_of(&tea, "tea"));
    h.wait_for("MR !12 and PR 3", move |v, cx| {
        let model = &v.sidebar.read(cx).model;
        let pr = |p: &Path| {
            model
                .worktree_for_path(p)
                .and_then(|(_, w)| w.pr.clone())
                .map(|pr| (pr.number, pr.state, pr.checks))
        };
        pr(&lab_feat) == Some((12, PrState::Open, CheckState::Failure))
            && pr(&tea_feat) == Some((3, PrState::Merged, CheckState::Success))
    });
}
