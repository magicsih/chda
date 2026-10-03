//! Blocking refresh helpers the UI runs on background threads.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chda_agents::{AgentAdapter, SessionCache};

use crate::sidebar::{DiffSummary, GitBadges, SessionEntry, WorktreeEntry};

fn badges(status: chda_git::GitStatus) -> GitBadges {
    GitBadges {
        changed: status.changed,
        staged: status.staged,
        untracked: status.untracked,
        conflicted: status.conflicted,
        ahead: status.ahead,
        behind: status.behind,
        operation: status.operation,
    }
}

/// `git cherry` results keyed by (repo, base id, branch id). Both ids change
/// whenever the answer could, so entries never go stale; the map stays small
/// because only current pairs are looked up and old ones are dropped.
/// (repo, base id, branch id) -> merged by patch content.
type PatchCache = HashMap<(PathBuf, String, String), bool>;

static PATCH_MERGED: Mutex<Option<PatchCache>> = Mutex::new(None);

/// Branches of `repo` merged into `base` by ancestry or by patch content
/// (squash and rebase merges).
fn merged_branches(repo: &Path, base: &str, branches: &[String]) -> Vec<String> {
    let mut merged = chda_git::branches_merged_into(repo, base).unwrap_or_default();
    let rest: Vec<&String> = branches.iter().filter(|b| !merged.contains(b)).collect();
    if rest.is_empty() {
        return merged;
    }
    let refs: Vec<&str> = std::iter::once(base)
        .chain(rest.iter().map(|b| b.as_str()))
        .collect();
    let Ok(ids) = chda_git::resolve(repo, &refs) else {
        return merged;
    };
    let Some(base_id) = ids[0].clone() else {
        return merged;
    };
    let mut cache = PATCH_MERGED.lock().unwrap_or_else(|e| e.into_inner());
    let old = cache.take().unwrap_or_default();
    let mut fresh = HashMap::new();
    for (branch, id) in rest.into_iter().zip(ids.into_iter().skip(1)) {
        let Some(id) = id else { continue };
        let key = (repo.to_path_buf(), base_id.clone(), id);
        let hit = match old.get(&key) {
            Some(v) => *v,
            None => chda_git::patch_merged(repo, base, branch).unwrap_or(false),
        };
        fresh.insert(key, hit);
        if hit {
            merged.push(branch.clone());
        }
    }
    // Keep other repositories' entries.
    fresh.extend(old.into_iter().filter(|(k, _)| k.0 != repo));
    *cache = Some(fresh);
    merged
}

/// Worktrees of `repo` with fresh git badges and merged flags. With `fetch`,
/// the default branch is fetched from `origin` first so merges made
/// elsewhere are seen.
pub fn worktrees_of(repo: &Path, fetch: bool) -> io::Result<Vec<WorktreeEntry>> {
    if fetch {
        let _ = chda_git::fetch_default_branch(repo);
    }
    let infos = chda_git::list_worktrees(repo)?;
    let branches: Vec<String> = infos
        .iter()
        .filter(|i| !i.is_main)
        .filter_map(|i| i.branch.clone())
        .collect();
    let base = chda_git::merge_target(repo).ok();
    let merged = base
        .as_deref()
        .map(|base| merged_branches(repo, base, &branches))
        .unwrap_or_default();
    let mut notes = chda_git::branch_descriptions(repo).unwrap_or_default();
    let mut out = Vec::new();
    for info in infos {
        let badges = if info.missing {
            GitBadges::default()
        } else {
            chda_git::status(&info.path).map(badges).unwrap_or_default()
        };
        let merged = info.branch.as_ref().is_some_and(|b| merged.contains(b));
        let note = info.branch.as_ref().and_then(|b| notes.remove(b));
        let diff = base
            .as_deref()
            .filter(|_| !info.is_main && !info.missing)
            .and_then(|base| {
                let stat = chda_git::diff_stat(&info.path, base).ok()?;
                Some(DiffSummary {
                    base: base.to_owned(),
                    merge_base: stat.merge_base,
                    files: stat.files,
                    added: stat.added,
                    removed: stat.removed,
                })
            });
        out.push(WorktreeEntry {
            path: info.path,
            branch: info.branch,
            is_main: info.is_main,
            badges,
            diff,
            merged,
            missing: info.missing,
            note,
            ..Default::default()
        });
    }
    Ok(out)
}

/// The command a diff tab runs: the change summary and the patch against
/// `merge_base`, paged by `less` that stays open even when the diff fits
/// on one screen (`-+F` undoes an `F` in the user's `LESS`).
pub fn diff_command(merge_base: &str) -> Vec<String> {
    [
        "git",
        "-c",
        "core.pager=less -+F -R",
        "diff",
        "--stat",
        "--patch",
        merge_base,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Local branches of `repo` that no worktree has checked out.
pub fn unchecked_branches(repo: &Path, worktrees: &[WorktreeEntry]) -> io::Result<Vec<String>> {
    let mut branches = chda_git::local_branches(repo)?;
    branches.retain(|b| !worktrees.iter().any(|w| w.branch.as_deref() == Some(b)));
    Ok(branches)
}

/// The main worktree of the repository containing `path`.
pub fn repo_of(path: &Path) -> io::Result<PathBuf> {
    chda_git::main_worktree(path)
}

/// Create a worktree for `branch` at `path`. Without `base` the branch is
/// created from HEAD, or checked out when it exists already. With `base` it
/// must be a new branch starting at `base`.
pub fn create_worktree(
    repo: &Path,
    branch: &str,
    path: &Path,
    base: Option<&str>,
) -> io::Result<()> {
    if path.exists() {
        return Err(io::Error::other(format!(
            "{} already exists",
            path.display()
        )));
    }
    if let Some(base) = base
        && chda_git::local_branches(repo)?.iter().any(|b| b == branch)
    {
        return Err(io::Error::other(format!(
            "branch {branch} already exists; pick another name to start from {base}"
        )));
    }
    chda_git::add_worktree(repo, branch, path, base)
}

/// A random readable branch name (`brisk-otter`) that is neither a local
/// branch of `repo` nor would land on an existing folder via `path_of`.
pub fn new_branch_name(repo: &Path, path_of: impl Fn(&str) -> PathBuf) -> String {
    let branches = chda_git::local_branches(repo).unwrap_or_default();
    crate::random_branch_name(|name| branches.iter().any(|b| b == name) || path_of(name).exists())
}

/// Remove a worktree and prune.
pub fn delete_worktree(repo: &Path, path: &Path, force: bool) -> io::Result<()> {
    chda_git::remove_worktree(repo, path, force)
}

/// Delete a local branch; `force` deletes unmerged branches too.
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> io::Result<()> {
    chda_git::delete_branch(repo, branch, force)
}

/// Set the note of `branch` (its git branch description); blank removes it.
pub fn set_note(repo: &Path, branch: &str, text: &str) -> io::Result<()> {
    chda_git::set_branch_description(repo, branch, text)
}

/// `chda note [<text>...]`, run inside a worktree: print the note of the
/// checked-out branch, or set it (an empty text removes it). Returns the
/// process exit code.
pub fn note_main(args: &[String]) -> i32 {
    let run = || -> io::Result<Option<String>> {
        let cwd = std::env::current_dir()?;
        let branch = chda_git::current_branch(&cwd)?.ok_or_else(|| {
            io::Error::other("not on a branch (detached HEAD or not a git repository)")
        })?;
        if args.is_empty() {
            return Ok(chda_git::branch_descriptions(&cwd)?.remove(&branch));
        }
        set_note(&cwd, &branch, &args.join(" "))?;
        Ok(None)
    };
    match run() {
        Ok(Some(note)) => {
            println!("{note}");
            0
        }
        Ok(None) => 0,
        Err(e) => {
            eprintln!("chda note: {e}");
            1
        }
    }
}

/// Fresh badges for one worktree.
pub fn badges_of(worktree: &Path) -> io::Result<GitBadges> {
    chda_git::status(worktree).map(badges)
}

/// Index the agent sessions that ran inside `worktrees`, newest first,
/// keyed by their cwd, and save the index cache in `data_dir` for the next
/// launch.
pub fn index_sessions(
    adapters: &[Box<dyn AgentAdapter>],
    cache: &Arc<Mutex<SessionCache>>,
    worktrees: &[PathBuf],
    data_dir: Option<&Path>,
) -> Vec<(PathBuf, SessionEntry)> {
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    let sessions = chda_agents::index_sessions(adapters, &mut cache, worktrees);
    if let Some(dir) = data_dir {
        let _ = cache.save(dir);
    }
    sessions
        .into_iter()
        .map(|s| {
            (
                s.cwd,
                SessionEntry {
                    agent: s.agent.as_str().to_owned(),
                    id: s.id.0,
                    started_at: s.started_at,
                    last_active_at: s.last_active_at,
                    snippet: s.snippet,
                    message_count: s.message_count,
                    usage: s.usage,
                },
            )
        })
        .collect()
}
