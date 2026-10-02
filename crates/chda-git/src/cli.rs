//! Write operations through the `git` binary (ADR-0003). Results are judged
//! by exit status and porcelain output; nothing here parses human text.

use std::collections::HashMap;
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

/// Folder inside the repository's git directory that deleted worktrees are
/// moved into before their files are removed.
const TRASH_DIR: &str = "chda-trash";

/// Remove a linked worktree without waiting for its files: unlock it, move
/// its folder into `<git-common-dir>/chda-trash/` (a rename, instant even for
/// a large `node_modules`), then drop git's record of it. Returns the moved
/// folder; [`purge_trash`] deletes it later. Falls back to
/// `git worktree remove -f -f` when the folder cannot be renamed there (for
/// example on another file system), returning `None`.
pub fn trash_worktree(repo: &Path, path: &Path) -> io::Result<Option<PathBuf>> {
    let is_linked = crate::read::list_worktrees(repo)?
        .iter()
        .any(|w| !w.is_main && same_path(&w.path, path));
    if !is_linked {
        return Err(io::Error::other(format!(
            "{} is not a linked worktree of {}",
            path.display(),
            repo.display()
        )));
    }
    let path_str = path.to_string_lossy();
    // Locked worktrees refuse removal and pruning; a failing unlock means it
    // was not locked.
    let _ = run(repo, &["worktree", "unlock", &path_str]);
    if !path.exists() {
        return check(repo, &["worktree", "remove", "-f", "-f", &path_str]).map(|_| None);
    }
    let trash = trash_root(repo)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "worktree".into());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let target = trash.join(format!("{name}-{stamp}"));
    let moved = std::fs::create_dir_all(&trash).and_then(|_| std::fs::rename(path, &target));
    if moved.is_err() {
        check(repo, &["worktree", "remove", "-f", "-f", &path_str])?;
        return Ok(None);
    }
    // The folder is gone from git's point of view: this drops only its record.
    check(repo, &["worktree", "remove", "-f", "-f", &path_str])?;
    Ok(Some(target))
}

/// Delete everything [`trash_worktree`] moved aside in `repo`. Slow for
/// large trees; run it in the background. Leftovers from an interrupted run
/// go too.
pub fn purge_trash(repo: &Path) -> io::Result<()> {
    let trash = trash_root(repo)?;
    match std::fs::remove_dir_all(&trash) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Equal paths, also when one of them goes through a symlink (`/tmp`,
/// `/var` on macOS). Missing folders compare by their parent.
fn same_path(a: &Path, b: &Path) -> bool {
    fn real(p: &Path) -> Option<PathBuf> {
        p.canonicalize()
            .ok()
            .or_else(|| Some(p.parent()?.canonicalize().ok()?.join(p.file_name()?)))
    }
    a == b || real(a).is_some_and(|ra| Some(ra) == real(b))
}

fn trash_root(repo: &Path) -> io::Result<PathBuf> {
    let common = PathBuf::from(output(repo, &["rev-parse", "--git-common-dir"])?);
    Ok(repo.join(common).join(TRASH_DIR))
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
/// Size of a worktree's changes against the commit it branched from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffStat {
    /// The merge base of `HEAD` and the base branch.
    pub merge_base: String,
    pub files: usize,
    pub added: usize,
    pub removed: usize,
}

/// Tracked changes in `worktree` (commits and uncommitted edits) against
/// its merge base with `base`.
pub fn diff_stat(worktree: &Path, base: &str) -> io::Result<DiffStat> {
    let merge_base = output(worktree, &["merge-base", "HEAD", base])?;
    let numstat = output(
        worktree,
        &["diff", "--numstat", "--no-renames", &merge_base],
    )?;
    let mut stat = parse_numstat(&numstat);
    stat.merge_base = merge_base;
    Ok(stat)
}

/// `git diff --numstat` lines: `added<TAB>removed<TAB>path`, with `-` for
/// binary files.
fn parse_numstat(text: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let mut cols = line.split('\t');
        let added = cols.next().and_then(|n| n.parse::<usize>().ok());
        let removed = cols.next().and_then(|n| n.parse::<usize>().ok());
        stat.files += 1;
        stat.added += added.unwrap_or(0);
        stat.removed += removed.unwrap_or(0);
    }
    stat
}

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

/// How `worktree`'s branch relates to its upstream right after a fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpstreamState {
    pub ahead: usize,
    pub behind: usize,
    /// Local commits (not on the upstream) that no remote-tracking branch
    /// has either: never pushed anywhere.
    pub local_only: usize,
}

/// Fetch the upstream of `worktree`'s branch and compare. `None` when the
/// branch has no upstream.
pub fn fetch_upstream(worktree: &Path) -> io::Result<Option<UpstreamState>> {
    let Some(branch) = current_branch(worktree)? else {
        return Ok(None);
    };
    let remote = run(worktree, &["config", &format!("branch.{branch}.remote")])?;
    let merge = run(worktree, &["config", &format!("branch.{branch}.merge")])?;
    if !remote.status.success() || !merge.status.success() {
        return Ok(None);
    }
    let remote = String::from_utf8_lossy(&remote.stdout).trim().to_owned();
    let merge = String::from_utf8_lossy(&merge.stdout).trim().to_owned();
    // "." is a local branch as upstream: nothing to fetch.
    if remote != "." {
        check(
            worktree,
            &["fetch", "--quiet", "--no-tags", &remote, &merge],
        )?;
    }
    let counts = output(
        worktree,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    )?;
    let mut counts = counts.split_whitespace().map(|n| n.parse::<usize>());
    let (Some(Ok(ahead)), Some(Ok(behind))) = (counts.next(), counts.next()) else {
        return Err(io::Error::other(format!(
            "unexpected rev-list output for {branch}"
        )));
    };
    let local_only = output(
        worktree,
        &[
            "rev-list",
            "--count",
            "@{upstream}..HEAD",
            "--not",
            "--remotes",
        ],
    )?
    .parse()
    .map_err(io::Error::other)?;
    Ok(Some(UpstreamState {
        ahead,
        behind,
        local_only,
    }))
}

/// How [`pull_upstream`] brings the upstream's commits in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullMode {
    FastForward,
    /// Replay local commits on top of the upstream.
    Rebase,
    /// Merge commit.
    Merge,
}

/// Bring the (already fetched) upstream into `worktree`. A rebase or merge
/// that stops on conflicts is aborted, leaving the branch as it was, and
/// reported as [`MergeOutcome::Conflicted`].
pub fn pull_upstream(worktree: &Path, mode: PullMode) -> io::Result<MergeOutcome> {
    let (args, abort): (&[&str], &[&str]) = match mode {
        PullMode::FastForward => (&["merge", "--ff-only", "@{upstream}"], &[]),
        PullMode::Rebase => (&["rebase", "@{upstream}"], &["rebase", "--abort"]),
        PullMode::Merge => (
            &["merge", "--no-edit", "@{upstream}"],
            &["merge", "--abort"],
        ),
    };
    let out = run(worktree, args)?;
    if out.status.success() {
        return Ok(MergeOutcome::Merged);
    }
    if abort.is_empty() {
        return Err(io::Error::other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let _ = run(worktree, abort);
    Ok(MergeOutcome::Conflicted)
}

/// Branch descriptions (`branch.<name>.description`, what
/// `git branch --edit-description` writes), keyed by branch name.
pub fn branch_descriptions(repo: &Path) -> io::Result<HashMap<String, String>> {
    let out = run(
        repo,
        &["config", "-z", "--get-regexp", r"^branch\..*\.description$"],
    )?;
    // Exit status 1: no description is set.
    if !out.status.success() && out.status.code() != Some(1) {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter_map(|entry| {
            let (key, value) = entry.split_once('\n')?;
            let branch = key.strip_prefix("branch.")?.strip_suffix(".description")?;
            let value = value.trim_end();
            (!value.is_empty()).then(|| (branch.to_owned(), value.to_owned()))
        })
        .collect())
}

/// Set `branch`'s description; blank text removes it.
pub fn set_branch_description(repo: &Path, branch: &str, text: &str) -> io::Result<()> {
    let key = format!("branch.{branch}.description");
    let text = text.trim();
    if text.is_empty() {
        let out = run(repo, &["config", "--unset", &key])?;
        // Exit status 5: it was not set.
        if out.status.success() || out.status.code() == Some(5) {
            return Ok(());
        }
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    check(repo, &["config", &key, &format!("{text}\n")])
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

    /// A clone of `repo` with `branch` tracking its origin.
    fn clone_tracking(repo: &TempRepo, name: &str) -> PathBuf {
        let clone = repo.root.join(name);
        TempRepo::git(&repo.root, &["clone", "-q", "repo", name]);
        TempRepo::git(&clone, &["config", "user.email", "t@example.com"]);
        TempRepo::git(&clone, &["config", "user.name", "t"]);
        TempRepo::git(&clone, &["config", "commit.gpgsign", "false"]);
        clone
    }

    #[test]
    fn upstream_fast_forward_rebase_and_merge() {
        let repo = TempRepo::new("pull");
        let clone = clone_tracking(&repo, "clone");
        assert_eq!(fetch_upstream(&repo.path).unwrap(), None, "no upstream");

        // Behind only: fast-forward.
        repo.commit_file(&repo.path, "b.txt", "b\n");
        let st = fetch_upstream(&clone).unwrap().unwrap();
        assert_eq!((st.ahead, st.behind, st.local_only), (0, 1, 0));
        assert_eq!(
            pull_upstream(&clone, PullMode::FastForward).unwrap(),
            MergeOutcome::Merged
        );
        assert!(clone.join("b.txt").is_file());

        // Diverged with a local, never pushed commit: ff refuses, rebase works.
        repo.commit_file(&repo.path, "c.txt", "c\n");
        repo.commit_file(&clone, "local.txt", "l\n");
        let st = fetch_upstream(&clone).unwrap().unwrap();
        assert_eq!((st.ahead, st.behind, st.local_only), (1, 1, 1));
        assert!(pull_upstream(&clone, PullMode::FastForward).is_err());
        assert_eq!(
            pull_upstream(&clone, PullMode::Rebase).unwrap(),
            MergeOutcome::Merged
        );
        let st = fetch_upstream(&clone).unwrap().unwrap();
        assert_eq!((st.ahead, st.behind), (1, 0));

        // A local commit that is on another remote branch counts as pushed.
        TempRepo::git(&clone, &["push", "-q", "origin", "HEAD:refs/heads/shared"]);
        TempRepo::git(&clone, &["fetch", "-q", "origin"]);
        repo.commit_file(&repo.path, "d.txt", "d\n");
        let st = fetch_upstream(&clone).unwrap().unwrap();
        assert_eq!((st.ahead, st.behind, st.local_only), (1, 1, 0));
        assert_eq!(
            pull_upstream(&clone, PullMode::Merge).unwrap(),
            MergeOutcome::Merged
        );
        assert!(clone.join("d.txt").is_file());

        // Conflicts are aborted and leave the branch as it was.
        repo.commit_file(&repo.path, "a.txt", "upstream\n");
        repo.commit_file(&clone, "a.txt", "local\n");
        fetch_upstream(&clone).unwrap();
        let head = output(&clone, &["rev-parse", "HEAD"]).unwrap();
        for mode in [PullMode::Rebase, PullMode::Merge] {
            assert_eq!(
                pull_upstream(&clone, mode).unwrap(),
                MergeOutcome::Conflicted
            );
            assert_eq!(output(&clone, &["rev-parse", "HEAD"]).unwrap(), head);
            assert!(crate::status(&clone).unwrap().operation.is_none());
        }
        // A merge left half way is reported.
        assert!(
            !run(&clone, &["merge", "@{upstream}"])
                .unwrap()
                .status
                .success()
        );
        assert_eq!(crate::status(&clone).unwrap().operation, Some("merge"));
        TempRepo::git(&clone, &["merge", "--abort"]);
    }

    #[test]
    fn trash_removes_locked_and_large_worktrees_at_once() {
        let repo = TempRepo::new("trash");
        let wt = repo.root.join("repo.worktrees").join("big");
        add_worktree(&repo.path, "big", &wt, None).unwrap();
        // `git worktree remove --force` refuses a locked worktree.
        TempRepo::git(&repo.path, &["worktree", "lock", wt.to_str().unwrap()]);
        assert!(remove_worktree(&repo.path, &wt, true).is_err());
        let deps = wt.join("node_modules/pkg");
        std::fs::create_dir_all(&deps).unwrap();
        for i in 0..2000 {
            std::fs::write(deps.join(format!("f{i}.js")), "x").unwrap();
        }

        let moved = trash_worktree(&repo.path, &wt).unwrap().unwrap();
        assert!(!wt.exists());
        assert!(moved.join("node_modules/pkg/f0.js").is_file());
        let list = TempRepo::git(&repo.path, &["worktree", "list", "--porcelain"]);
        assert!(!list.contains("big"), "record dropped: {list}");
        delete_branch(&repo.path, "big", true).unwrap();

        purge_trash(&repo.path).unwrap();
        assert!(!moved.exists());
        purge_trash(&repo.path).unwrap();

        // Only linked worktrees: never the main one or a stray folder.
        assert!(trash_worktree(&repo.path, &repo.path).is_err());
        assert!(trash_worktree(&repo.path, &repo.root.join("elsewhere")).is_err());
        assert!(repo.path.join("a.txt").is_file());
    }

    #[test]
    fn trash_keeps_other_missing_worktree_records() {
        let repo = TempRepo::new("trash-missing");
        let a = repo.root.join("repo.worktrees").join("a");
        let b = repo.root.join("repo.worktrees").join("b");
        add_worktree(&repo.path, "a", &a, None).unwrap();
        add_worktree(&repo.path, "b", &b, None).unwrap();
        std::fs::remove_dir_all(&a).unwrap();
        std::fs::remove_dir_all(&b).unwrap();
        assert_eq!(trash_worktree(&repo.path, &a).unwrap(), None);
        let list = TempRepo::git(&repo.path, &["worktree", "list", "--porcelain"]);
        assert!(!list.contains("worktrees/a\n"), "{list}");
        assert!(
            list.contains("worktrees/b"),
            "the other record stays: {list}"
        );
    }

    #[test]
    fn branch_descriptions_round_trip() {
        let repo = TempRepo::new("desc");
        assert!(branch_descriptions(&repo.path).unwrap().is_empty());
        TempRepo::git(&repo.path, &["branch", "feat/a.b"]);
        set_branch_description(
            &repo.path,
            "feat/a.b",
            "Fix login loop\n\nSee #123 = details\n",
        )
        .unwrap();
        set_branch_description(&repo.path, "main", "trunk").unwrap();
        let notes = branch_descriptions(&repo.path).unwrap();
        assert_eq!(notes["feat/a.b"], "Fix login loop\n\nSee #123 = details");
        assert_eq!(notes["main"], "trunk");
        // Stored like `git branch --edit-description` does: with a newline.
        assert_eq!(
            TempRepo::git(&repo.path, &["config", "branch.main.description"]),
            "trunk\n\n"
        );
        set_branch_description(&repo.path, "main", "  ").unwrap();
        set_branch_description(&repo.path, "main", "").unwrap();
        assert!(
            !branch_descriptions(&repo.path)
                .unwrap()
                .contains_key("main")
        );
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

    #[test]
    fn diff_stat_counts_commits_and_edits_since_the_merge_base() {
        let repo = TempRepo::new("diffstat");
        let wt = repo.root.join("wt");
        crate::add_worktree(&repo.path, "feat", &wt, None).unwrap();
        assert_eq!(diff_stat(&wt, "main").unwrap().files, 0);
        repo.commit_file(&wt, "new.txt", "one\ntwo\n");
        // main moves on; that must not count.
        repo.commit_file(&repo.path, "other.txt", "x\n");
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        let stat = diff_stat(&wt, "main").unwrap();
        assert_eq!((stat.files, stat.added), (2, 3));
        assert_eq!(stat.merge_base.len(), 40);
        assert_eq!(
            parse_numstat("-\t-\tlogo.png\n4\t1\tsrc/x.rs\n"),
            DiffStat {
                merge_base: String::new(),
                files: 2,
                added: 4,
                removed: 1
            }
        );
    }
}
