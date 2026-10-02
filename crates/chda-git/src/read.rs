//! Read operations through gix (ADR-0003).

use std::io;
use std::path::{Path, PathBuf};

/// One worktree of a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub is_main: bool,
    /// Branch name, or `None` when HEAD is detached.
    pub branch: Option<String>,
    /// Abbreviated HEAD commit; `None` on an unborn branch.
    pub head: Option<String>,
    pub locked: bool,
    /// The worktree's folder is gone (deleted outside git); git calls such
    /// worktrees prunable.
    pub missing: bool,
}

/// Counts shown as badges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// Tracked files changed in the worktree but not staged.
    pub changed: usize,
    pub staged: usize,
    pub untracked: usize,
    pub conflicted: usize,
    /// Commits ahead of / behind the upstream, when there is one.
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
}

impl GitStatus {
    pub fn is_clean(&self) -> bool {
        self.changed + self.staged + self.untracked + self.conflicted == 0
    }
}

fn err(e: gix::Error) -> io::Error {
    io::Error::other(e)
}

/// The main worktree path of the repository containing `path`, or an error
/// when `path` is not inside a git repository.
pub fn main_worktree(path: &Path) -> io::Result<PathBuf> {
    let repo = gix::discover(path).map_err(err)?;
    let main = repo.main_repo().map_err(err)?;
    main.workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("bare repository"))
}

fn head_info(repo: &gix::Repository) -> io::Result<(Option<String>, Option<String>)> {
    let head = repo.head().map_err(err)?;
    let branch = head.referent_name().map(|n| n.shorten().to_string());
    let id = head.id().map(|id| id.shorten_or_id().to_string());
    Ok((branch, id))
}

/// All worktrees of the repository at `repo`, main first.
pub fn list_worktrees(repo: &Path) -> io::Result<Vec<WorktreeInfo>> {
    let main = gix::discover(repo).map_err(err)?.main_repo().map_err(err)?;
    let mut out = Vec::new();
    if let Some(wt) = main.worktree() {
        let (branch, head) = head_info(&main)?;
        out.push(WorktreeInfo {
            path: wt.base().to_path_buf(),
            is_main: true,
            branch,
            head,
            locked: wt.is_locked(),
            missing: false,
        });
    }
    for proxy in main.worktrees()? {
        let Ok(path) = proxy.base() else {
            continue;
        };
        let locked = proxy.is_locked();
        let Ok(linked) = proxy.into_repo_with_possibly_inaccessible_worktree() else {
            continue;
        };
        let (branch, head) = head_info(&linked)?;
        let missing = !path.exists();
        out.push(WorktreeInfo {
            path,
            is_main: false,
            branch,
            head,
            locked,
            missing,
        });
    }
    Ok(out)
}

/// Working-tree status of the worktree at `worktree`.
pub fn status(worktree: &Path) -> io::Result<GitStatus> {
    use gix::status::index_worktree::Item as IwItem;
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
    use gix::status::{Item, UntrackedFiles};

    let repo = gix::discover(worktree).map_err(err)?;
    let mut s = GitStatus::default();
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(err)?
        .untracked_files(UntrackedFiles::Files)
        .index_worktree_submodules(None)
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled)
        .into_iter(Vec::new())
        .map_err(err)?;
    for item in iter {
        match item.map_err(err)? {
            Item::TreeIndex(_) => s.staged += 1,
            Item::IndexWorktree(IwItem::Modification { status, .. }) => match status {
                EntryStatus::Conflict { .. } => s.conflicted += 1,
                EntryStatus::Change(
                    Change::Removed
                    | Change::Type { .. }
                    | Change::Modification { .. }
                    | Change::SubmoduleModification(_),
                ) => s.changed += 1,
                EntryStatus::NeedsUpdate(_) | EntryStatus::IntentToAdd => {}
            },
            Item::IndexWorktree(IwItem::DirectoryContents { entry, .. }) => {
                if matches!(entry.status, gix::dir::entry::Status::Untracked) {
                    s.untracked += 1;
                }
            }
            Item::IndexWorktree(IwItem::Rewrite { .. }) => s.changed += 1,
        }
    }
    if let Some((ahead, behind)) = ahead_behind(&repo)? {
        s.ahead = Some(ahead);
        s.behind = Some(behind);
    }
    Ok(s)
}

fn ahead_behind(repo: &gix::Repository) -> io::Result<Option<(usize, usize)>> {
    let Some(local_ref) = repo.head_ref().map_err(err)? else {
        return Ok(None);
    };
    let Some(upstream_name) = local_ref
        .remote_tracking_ref_name(gix::remote::Direction::Fetch)
        .transpose()
        .map_err(err)?
    else {
        return Ok(None);
    };
    let Some(mut upstream_ref) = repo
        .try_find_reference(upstream_name.as_ref())
        .map_err(err)?
    else {
        return Ok(None);
    };
    let local_id = local_ref.into_fully_peeled_id().map_err(err)?.detach();
    let upstream_id = upstream_ref.peel_to_id().map_err(err)?.detach();
    let ahead = repo
        .rev_walk([local_id])
        .with_hidden([upstream_id])
        .all()
        .map_err(err)?
        .count();
    let behind = repo
        .rev_walk([upstream_id])
        .with_hidden([local_id])
        .all()
        .map_err(err)?
        .count();
    Ok(Some((ahead, behind)))
}

/// A configured remote, as `gh` would consider it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInfo {
    pub name: String,
    /// Fetch URL after `url.<base>.insteadOf` rewrites.
    pub url: String,
    /// `remote.<name>.gh-resolved`, set by `gh repo set-default`.
    pub gh_resolved: Option<String>,
}

/// Every remote with a fetch URL of the repository containing `repo`.
pub fn remotes(repo: &Path) -> io::Result<Vec<RemoteInfo>> {
    let repo = gix::discover(repo).map_err(err)?;
    let config = repo.config_snapshot();
    let mut out = Vec::new();
    for name in repo.remote_names() {
        let Ok(remote) = repo.find_remote(&**name) else {
            continue;
        };
        let Some(url) = remote.url(gix::remote::Direction::Fetch) else {
            continue;
        };
        let name = name.to_string();
        let gh_resolved = config
            .string(format!("remote.{name}.gh-resolved").as_str())
            .map(|v| v.to_string());
        out.push(RemoteInfo {
            url: url.to_bstring().to_string(),
            name,
            gh_resolved,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testing::TempRepo;

    #[test]
    fn lists_remotes_with_rewrites_and_gh_default() {
        let repo = TempRepo::new("remotes");
        assert!(remotes(&repo.path).unwrap().is_empty());
        TempRepo::git(
            &repo.path,
            &["remote", "add", "origin", "git@github-work:org/repo.git"],
        );
        TempRepo::git(&repo.path, &["remote", "add", "upstream", "gh:up/repo"]);
        TempRepo::git(
            &repo.path,
            &["config", "url.https://github.com/.insteadOf", "gh:"],
        );
        TempRepo::git(
            &repo.path,
            &["config", "remote.upstream.gh-resolved", "base"],
        );
        let mut list = remotes(&repo.path).unwrap();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            list,
            vec![
                RemoteInfo {
                    name: "origin".into(),
                    url: "git@github-work:org/repo.git".into(),
                    gh_resolved: None,
                },
                RemoteInfo {
                    name: "upstream".into(),
                    url: "https://github.com/up/repo".into(),
                    gh_resolved: Some("base".into()),
                },
            ]
        );
    }

    #[test]
    fn lists_worktrees_and_counts_status() {
        let repo = TempRepo::new("read");
        let wt = repo.root.join("repo.worktrees").join("feat");
        crate::add_worktree(&repo.path, "feat", &wt, None).unwrap();

        let list = list_worktrees(&repo.path).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].is_main);
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[1].branch.as_deref(), Some("feat"));
        assert_eq!(
            list[1].path.canonicalize().unwrap(),
            wt.canonicalize().unwrap()
        );
        assert!(list[0].head.is_some());
        assert_eq!(
            main_worktree(&wt).unwrap().canonicalize().unwrap(),
            repo.path.canonicalize().unwrap()
        );

        assert!(status(&wt).unwrap().is_clean());
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();
        std::fs::write(wt.join("staged.txt"), "s\n").unwrap();
        TempRepo::git(&wt, &["add", "staged.txt"]);
        let s = status(&wt).unwrap();
        assert_eq!(
            (s.changed, s.staged, s.untracked, s.conflicted),
            (1, 1, 1, 0)
        );
        assert_eq!(s.ahead, None);

        // Upstream: clone the repo as a remote and diverge.
        let remote = repo.root.join("remote.git");
        TempRepo::git(
            &repo.root,
            &[
                "clone",
                "-q",
                "--bare",
                repo.path.to_str().unwrap(),
                remote.to_str().unwrap(),
            ],
        );
        TempRepo::git(
            &repo.path,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        TempRepo::git(&repo.path, &["fetch", "-q", "origin"]);
        TempRepo::git(
            &repo.path,
            &["branch", "--set-upstream-to=origin/main", "main"],
        );
        repo.commit_file(&repo.path, "c1.txt", "1\n");
        repo.commit_file(&repo.path, "c2.txt", "2\n");
        let s = status(&repo.path).unwrap();
        assert_eq!((s.ahead, s.behind), (Some(2), Some(0)));
    }
}
