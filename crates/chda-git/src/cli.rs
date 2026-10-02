//! Write operations through the `git` binary (ADR-0003). Results are judged
//! by exit status and porcelain output; nothing here parses human text.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Outcome of merging a branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// Nothing to do: `from` was already in `into`.
    UpToDate,
    Merged,
    /// The merge stopped on conflicts and was aborted; the tree is clean.
    Conflicted,
}

/// Locate `git`, honoring `CHDA_GIT` for tests and unusual installs.
pub fn git_binary() -> PathBuf {
    std::env::var_os("CHDA_GIT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("git"))
}

fn run(cwd: &Path, args: &[&str]) -> io::Result<Output> {
    Command::new(git_binary())
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
}

fn check(cwd: &Path, args: &[&str]) -> io::Result<()> {
    let out = run(cwd, args)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// `git worktree add`. Creates `branch` from `base` (default: HEAD) unless it
/// already exists, in which case it is checked out.
pub fn add_worktree(repo: &Path, branch: &str, path: &Path, base: Option<&str>) -> io::Result<()> {
    let exists = run(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )?
    .status
    .success();
    let path = path.to_string_lossy();
    if exists {
        check(repo, &["worktree", "add", &path, branch])
    } else {
        let mut args = vec!["worktree", "add", "-b", branch, &path];
        if let Some(base) = base {
            args.push(base);
        }
        check(repo, &args)
    }
}

/// `git worktree remove`, then `git worktree prune`.
pub fn remove_worktree(repo: &Path, path: &Path, force: bool) -> io::Result<()> {
    // A worktree whose folder is already gone cannot be "removed"; pruning
    // drops git's record of it.
    if !path.exists() {
        return check(repo, &["worktree", "prune"]);
    }
    let path = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    check(repo, &args)?;
    check(repo, &["worktree", "prune"])
}

/// `git branch -d` (or `-D` with `force`).
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> io::Result<()> {
    check(repo, &["branch", if force { "-D" } else { "-d" }, branch])
}

/// Merge `from` into `into` inside `repo`'s worktree, which must have `into`
/// checked out. Conflicts abort the merge and report [`MergeOutcome::Conflicted`].
pub fn merge_into(repo: &Path, from: &str, into: &str) -> io::Result<MergeOutcome> {
    let head = run(repo, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    if String::from_utf8_lossy(&head.stdout).trim() != into {
        return Err(io::Error::other(format!(
            "{into} is not checked out in {}",
            repo.display()
        )));
    }
    let ancestor = run(repo, &["merge-base", "--is-ancestor", from, into])?;
    if ancestor.status.success() {
        return Ok(MergeOutcome::UpToDate);
    }
    let out = run(repo, &["merge", "--no-edit", from])?;
    if out.status.success() {
        return Ok(MergeOutcome::Merged);
    }
    let _ = run(repo, &["merge", "--abort"]);
    Ok(MergeOutcome::Conflicted)
}

/// The branch worktrees should be merged into: `origin/HEAD` if set, else
/// `main` or `master`, whichever exists.
pub fn default_branch(repo: &Path) -> io::Result<String> {
    let out = run(
        repo,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    )?;
    if out.status.success() {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if let Some(branch) = name.strip_prefix("origin/") {
            return Ok(branch.to_owned());
        }
    }
    for candidate in ["main", "master"] {
        if run(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{candidate}"),
            ],
        )?
        .status
        .success()
        {
            return Ok(candidate.to_owned());
        }
    }
    Err(io::Error::other("no main or master branch"))
}

/// The ref to judge "merged" against: `origin/<default>` when the remote
/// tracking branch exists (it reflects merges made elsewhere even when the
/// local branch is stale), else the local default branch.
pub fn merge_target(repo: &Path) -> io::Result<String> {
    let base = default_branch(repo)?;
    let remote = format!("origin/{base}");
    let exists = run(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{remote}"),
        ],
    )?
    .status
    .success();
    Ok(if exists { remote } else { base })
}

/// Update `refs/remotes/origin/<default>` from the remote, quietly. Errors
/// (offline, no remote, auth) are returned but callers usually ignore them.
pub fn fetch_default_branch(repo: &Path) -> io::Result<()> {
    let base = default_branch(repo)?;
    check(repo, &["fetch", "--quiet", "--no-tags", "origin", &base])
}

/// Whether `branch` has been merged into `base` by patch content, which
/// `branches_merged_into` (ancestry) cannot see:
/// - rebase merge: every commit of `branch` has an equal patch in `base`;
/// - squash merge: the whole branch diff, squashed into one commit, has an
///   equal patch in `base`.
pub fn patch_merged(repo: &Path, base: &str, branch: &str) -> io::Result<bool> {
    if cherry_all_applied(repo, base, branch)? {
        return Ok(true);
    }
    let merge_base = output(repo, &["merge-base", base, branch])?;
    let squashed = output(
        repo,
        &[
            "-c",
            "user.name=chda",
            "-c",
            "user.email=chda@localhost",
            "commit-tree",
            &format!("{branch}^{{tree}}"),
            "-p",
            &merge_base,
            "-m",
            "squashed for merge check",
        ],
    )?;
    cherry_all_applied(repo, base, &squashed)
}

/// `git cherry`: true when no commit of `head` is missing from `base`.
fn cherry_all_applied(repo: &Path, base: &str, head: &str) -> io::Result<bool> {
    Ok(output(repo, &["cherry", base, head])?
        .lines()
        .all(|l| !l.starts_with('+')))
}

/// Trimmed stdout of a git command that must succeed.
fn output(repo: &Path, args: &[&str]) -> io::Result<String> {
    let out = run(repo, args)?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Object ids of `refs`, in order; `None` for refs that do not resolve.
pub fn resolve(repo: &Path, refs: &[&str]) -> io::Result<Vec<Option<String>>> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        let res = run(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{r}^{{commit}}"),
            ],
        )?;
        out.push(
            res.status
                .success()
                .then(|| String::from_utf8_lossy(&res.stdout).trim().to_owned()),
        );
    }
    Ok(out)
}

/// Local branches whose commits are all contained in `base`.
pub fn branches_merged_into(repo: &Path, base: &str) -> io::Result<Vec<String>> {
    let out = run(
        repo,
        &["branch", "--format=%(refname:short)", "--merged", base],
    )?;
    if !out.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty() && l != base)
        .collect())
}

/// All local branch names.
pub fn local_branches(repo: &Path) -> io::Result<Vec<String>> {
    let out = run(repo, &["branch", "--format=%(refname:short)"])?;
    if !out.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect())
}

/// The branch checked out in `worktree`, if not detached.
pub fn current_branch(worktree: &Path) -> io::Result<Option<String>> {
    let out = run(worktree, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    Ok(out
        .status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A throwaway repository with one commit on `main`.
    pub struct TempRepo {
        pub root: PathBuf,
        pub path: PathBuf,
    }

    impl TempRepo {
        pub fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("chda-git-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let path = root.join("repo");
            std::fs::create_dir_all(&path).unwrap();
            Self::git(&path, &["init", "-q", "-b", "main"]);
            Self::git(&path, &["config", "user.email", "t@example.com"]);
            Self::git(&path, &["config", "user.name", "t"]);
            Self::git(&path, &["config", "commit.gpgsign", "false"]);
            std::fs::write(path.join("a.txt"), "a\n").unwrap();
            Self::git(&path, &["add", "."]);
            Self::git(&path, &["commit", "-q", "-m", "init"]);
            Self { root, path }
        }

        pub fn git(cwd: &Path, args: &[&str]) -> String {
            let out = run(cwd, args).unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        }

        pub fn commit_file(&self, cwd: &Path, name: &str, text: &str) {
            std::fs::write(cwd.join(name), text).unwrap();
            Self::git(cwd, &["add", name]);
            Self::git(cwd, &["commit", "-q", "-m", name]);
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TempRepo;
    use super::*;

    #[test]
    fn worktree_add_merge_remove_and_branch_delete() {
        let repo = TempRepo::new("cli");
        let wt = repo.root.join("repo.worktrees").join("feat");
        add_worktree(&repo.path, "feat", &wt, None).unwrap();
        assert!(wt.join("a.txt").is_file());
        repo.commit_file(&wt, "b.txt", "b\n");

        assert_eq!(
            merge_into(&repo.path, "feat", "main").unwrap(),
            MergeOutcome::Merged
        );
        assert!(repo.path.join("b.txt").is_file());
        assert_eq!(
            merge_into(&repo.path, "feat", "main").unwrap(),
            MergeOutcome::UpToDate
        );

        // Conflict: both sides change a.txt.
        repo.commit_file(&wt, "a.txt", "from feat\n");
        repo.commit_file(&repo.path, "a.txt", "from main\n");
        assert_eq!(
            merge_into(&repo.path, "feat", "main").unwrap(),
            MergeOutcome::Conflicted
        );
        assert_eq!(
            std::fs::read_to_string(repo.path.join("a.txt")).unwrap(),
            "from main\n"
        );

        remove_worktree(&repo.path, &wt, false).unwrap();
        assert!(!wt.exists());
        assert!(delete_branch(&repo.path, "feat", false).is_err());
        delete_branch(&repo.path, "feat", true).unwrap();
        assert!(merge_into(&repo.path, "feat", "develop").is_err());

        assert_eq!(default_branch(&repo.path).unwrap(), "main");
        assert_eq!(current_branch(&repo.path).unwrap().as_deref(), Some("main"));
        TempRepo::git(&repo.path, &["branch", "done"]);
        TempRepo::git(&repo.path, &["branch", "ahead"]);
        let wt2 = repo.root.join("repo.worktrees").join("ahead");
        add_worktree(&repo.path, "ahead", &wt2, None).unwrap();
        repo.commit_file(&wt2, "z.txt", "z\n");
        assert!(
            local_branches(&repo.path)
                .unwrap()
                .contains(&"done".to_string())
        );
        let merged = branches_merged_into(&repo.path, "main").unwrap();
        assert!(merged.contains(&"done".to_string()));
        assert!(!merged.contains(&"ahead".to_string()));
    }

    #[test]
    fn squash_merged_branch_counts_as_patch_merged() {
        let repo = TempRepo::new("squash");
        let wt = repo.root.join("repo.worktrees").join("feat");
        add_worktree(&repo.path, "feat", &wt, None).unwrap();
        repo.commit_file(&wt, "b.txt", "b\n");
        repo.commit_file(&wt, "c.txt", "c\n");
        assert!(!patch_merged(&repo.path, "main", "feat").unwrap());

        TempRepo::git(&repo.path, &["merge", "--squash", "feat"]);
        TempRepo::git(&repo.path, &["commit", "-q", "-m", "feat (squashed)"]);
        assert!(
            !branches_merged_into(&repo.path, "main")
                .unwrap()
                .contains(&"feat".to_string())
        );
        assert!(patch_merged(&repo.path, "main", "feat").unwrap());

        // A later commit on the branch makes it unmerged again.
        repo.commit_file(&wt, "d.txt", "d\n");
        assert!(!patch_merged(&repo.path, "main", "feat").unwrap());

        // Rebase merge: the commits land on main one by one with new ids.
        let wt2 = repo.root.join("repo.worktrees").join("other");
        add_worktree(&repo.path, "other", &wt2, None).unwrap();
        repo.commit_file(&wt2, "e.txt", "e\n");
        repo.commit_file(&wt2, "f.txt", "f\n");
        TempRepo::git(&repo.path, &["cherry-pick", "main..other"]);
        assert!(patch_merged(&repo.path, "main", "other").unwrap());

        let ids = resolve(&repo.path, &["main", "feat", "nope"]).unwrap();
        assert!(ids[0].is_some() && ids[1].is_some() && ids[2].is_none());
    }

    #[test]
    fn merge_target_prefers_the_remote_tracking_branch() {
        let repo = TempRepo::new("target");
        assert_eq!(merge_target(&repo.path).unwrap(), "main");
        assert!(fetch_default_branch(&repo.path).is_err());

        let remote = repo.root.join("remote.git");
        TempRepo::git(&repo.root, &["clone", "-q", "--bare", "repo", "remote.git"]);
        TempRepo::git(
            &repo.path,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        fetch_default_branch(&repo.path).unwrap();
        assert_eq!(merge_target(&repo.path).unwrap(), "origin/main");
    }
}
