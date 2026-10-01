//! File-system watching: changes under a repository's `.git` (HEAD, refs,
//! index, worktrees) trigger a refresh of that repository, and changes to
//! config files trigger a reload.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Watches repositories and reports which one changed, debounced.
pub struct RepoWatcher {
    watcher: notify::RecommendedWatcher,
    repos: Arc<Mutex<Vec<PathBuf>>>,
}

impl RepoWatcher {
    /// `changed` runs on a background thread with the repository path, at
    /// most once per debounce window per repository.
    pub fn new(changed: impl Fn(PathBuf) + Send + 'static) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    let _ = tx.send(path);
                }
            }
        })?;
        let repos: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&repos);
        std::thread::Builder::new()
            .name("chda-repo-watch".into())
            .spawn(move || debounce_loop(&rx, &shared, &changed))
            .map_err(|e| notify::Error::generic(&e.to_string()))?;
        Ok(Self { watcher, repos })
    }

    /// Start watching `repo`'s `.git` directory.
    pub fn watch(&mut self, repo: &Path) -> notify::Result<()> {
        let mut repos = self.repos.lock().unwrap_or_else(|e| e.into_inner());
        if repos.iter().any(|r| r == repo) {
            return Ok(());
        }
        self.watcher
            .watch(&repo.join(".git"), RecursiveMode::Recursive)?;
        repos.push(repo.to_path_buf());
        Ok(())
    }

    pub fn unwatch(&mut self, repo: &Path) {
        let mut repos = self.repos.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = repos.iter().position(|r| r == repo) {
            repos.remove(i);
            let _ = self.watcher.unwatch(&repo.join(".git"));
        }
    }
}

/// How long config edits settle before a reload; editors often write a file
/// in several steps.
const FILE_DEBOUNCE: Duration = Duration::from_millis(200);

/// Watches a set of files and reports, debounced, that one of them changed.
/// It watches their directories, so files that editors replace (write to a
/// temporary file, then rename) and files created later are seen too.
pub struct FileWatcher {
    watcher: notify::RecommendedWatcher,
    files: Arc<Mutex<HashSet<PathBuf>>>,
    dirs: HashSet<PathBuf>,
}

impl FileWatcher {
    /// `changed` runs on a background thread after a burst of changes.
    pub fn new(changed: impl Fn() + Send + 'static) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    let _ = tx.send(path);
                }
            }
        })?;
        let files: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
        let shared = Arc::clone(&files);
        std::thread::Builder::new()
            .name("chda-file-watch".into())
            .spawn(move || {
                let mut deadline: Option<Instant> = None;
                loop {
                    let timeout = deadline
                        .map(|d| d.saturating_duration_since(Instant::now()))
                        .unwrap_or(Duration::from_secs(3600));
                    match rx.recv_timeout(timeout) {
                        Ok(path) => {
                            let files = shared.lock().unwrap_or_else(|e| e.into_inner());
                            if files.contains(&path) {
                                deadline.get_or_insert_with(|| Instant::now() + FILE_DEBOUNCE);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if deadline.take().is_some() {
                                changed();
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .map_err(|e| notify::Error::generic(&e.to_string()))?;
        Ok(Self {
            watcher,
            files,
            dirs: HashSet::new(),
        })
    }

    /// Replace the watched files. Files whose directory does not exist are
    /// skipped.
    pub fn set_files(&mut self, files: impl IntoIterator<Item = PathBuf>) {
        let files: HashSet<PathBuf> = files.into_iter().collect();
        let dirs: HashSet<PathBuf> = files
            .iter()
            .filter_map(|f| f.parent().map(Path::to_path_buf))
            .filter(|d| d.is_dir())
            .collect();
        for gone in self.dirs.difference(&dirs) {
            let _ = self.watcher.unwatch(gone);
        }
        for new in dirs.difference(&self.dirs) {
            let _ = self.watcher.watch(new, RecursiveMode::NonRecursive);
        }
        self.dirs = dirs;
        // Events report canonical paths on macOS (/private/tmp for /tmp).
        let mut all = files.clone();
        all.extend(files.iter().filter_map(|f| {
            let dir = f.parent()?.canonicalize().ok()?;
            Some(dir.join(f.file_name()?))
        }));
        *self.files.lock().unwrap_or_else(|e| e.into_inner()) = all;
    }
}

/// Object and log writes happen constantly during git operations and never
/// change what the sidebar shows; refs, HEAD, index and worktrees do.
fn interesting(path: &Path, git_dir: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(git_dir) else {
        return false;
    };
    let mut parts = rel.components();
    let first = parts.next().and_then(|c| c.as_os_str().to_str());
    if matches!(first, Some("objects" | "logs" | "lfs")) {
        return false;
    }
    !path.extension().is_some_and(|e| e == "lock")
}

fn debounce_loop(
    rx: &mpsc::Receiver<PathBuf>,
    repos: &Mutex<Vec<PathBuf>>,
    changed: &impl Fn(PathBuf),
) {
    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut deadline: Option<Instant> = None;
    loop {
        let timeout = deadline
            .map(|d| d.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(3600));
        match rx.recv_timeout(timeout) {
            Ok(path) => {
                let repos = repos.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(repo) = repos
                    .iter()
                    .find(|r| path.starts_with(r.join(".git")))
                    .filter(|r| interesting(&path, &r.join(".git")))
                {
                    pending.insert(repo.clone());
                    deadline.get_or_insert_with(|| Instant::now() + DEBOUNCE);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                for repo in pending.drain() {
                    changed(repo);
                }
                deadline = None;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_changes_are_reported_once_per_burst() {
        let dir = std::env::temp_dir().join(format!("chda-filewatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        let (tx, rx) = mpsc::channel();
        let mut w = FileWatcher::new(move || {
            let _ = tx.send(());
        })
        .unwrap();
        w.set_files([file.clone(), dir.join("missing/other")]);
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(dir.join("unrelated"), "x").unwrap();
        std::fs::write(&file, "a = 1").unwrap();
        std::fs::write(&file, "a = 2").unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("change reported");
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
        std::fs::write(dir.join("unrelated"), "y").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(600)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ref_changes_report_the_repository_once() {
        let root = std::env::temp_dir().join(format!("chda-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "-b", "main"]);
        git(&[
            "-c",
            "user.email=t@e",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ]);
        let root = root.canonicalize().unwrap();

        let (tx, rx) = mpsc::channel();
        let mut watcher = RepoWatcher::new(move |repo| {
            let _ = tx.send(repo);
        })
        .unwrap();
        watcher.watch(&root).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        git(&["branch", "watched-branch"]);
        let repo = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(repo, root);
        assert!(
            rx.recv_timeout(Duration::from_millis(600)).is_err(),
            "should be debounced to one event"
        );
        assert!(!interesting(
            &root.join(".git/objects/ab/cd"),
            &root.join(".git")
        ));
        assert!(interesting(
            &root.join(".git/refs/heads/x"),
            &root.join(".git")
        ));
        assert!(!interesting(
            &root.join(".git/index.lock"),
            &root.join(".git")
        ));
        watcher.unwatch(&root);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
