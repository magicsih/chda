//! Incremental, read-only commit ancestry through gix. FFI/library types stay here.
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitRefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
    Head,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitRef {
    pub kind: CommitRefKind,
    pub name: String,
}

impl CommitRef {
    pub fn label(&self) -> String {
        match self.kind {
            CommitRefKind::LocalBranch => format!("branch: {}", self.name),
            CommitRefKind::RemoteBranch => format!("remote: {}", self.name),
            CommitRefKind::Tag => format!("tag: {}", self.name),
            CommitRefKind::Head => self.name.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    pub id: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub refs: Vec<CommitRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    pub commits: Vec<CommitInfo>,
    pub has_more: bool,
}

type Walk = gix::traverse::commit::Topo<gix::Repository, fn(&gix::hash::oid) -> bool>;

/// A snapshot of refs and an owning, lazy ancestry iterator. Only background
/// callers read it. Each page resumes this walk, rather than walking from HEAD again.
pub struct CommitHistory {
    repo: gix::Repository,
    walk: std::iter::Peekable<Walk>,
    refs: BTreeMap<String, Vec<CommitRef>>,
}

impl CommitHistory {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut repo = gix::open(path).map_err(io::Error::other)?;
        repo.object_cache_size(Some(8 * 1024 * 1024));
        let mut refs = BTreeMap::<String, Vec<CommitRef>>::new();
        let mut tips = Vec::new();
        let platform = repo.references().map_err(io::Error::other)?;
        let references = platform
            .all()
            .map_err(io::Error::other)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        drop(platform);
        for mut reference in references {
            let full_name = reference.name().as_bstr().to_string();
            let (kind, name) = if let Some(name) = full_name.strip_prefix("refs/heads/") {
                (CommitRefKind::LocalBranch, name)
            } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
                (CommitRefKind::RemoteBranch, name)
            } else if let Some(name) = full_name.strip_prefix("refs/tags/") {
                (CommitRefKind::Tag, name)
            } else {
                continue;
            };
            let id = reference.peel_to_id().map_err(io::Error::other)?.detach();
            // Tags may legitimately point at trees or blobs, outside commit ancestry.
            let object = repo.find_object(id).map_err(io::Error::other)?;
            if object.kind != gix::object::Kind::Commit {
                continue;
            }
            tips.push(id);
            refs.entry(id.to_string()).or_default().push(CommitRef {
                kind,
                name: name.to_owned(),
            });
        }
        let head = repo.head().map_err(io::Error::other)?;
        if let Some(id) = head.id() {
            let id = id.detach();
            tips.push(id);
            let name = head
                .referent_name()
                .map(|name| format!("HEAD → {}", name.shorten()))
                .unwrap_or_else(|| "HEAD · detached".into());
            refs.entry(id.to_string()).or_default().push(CommitRef {
                kind: CommitRefKind::Head,
                name,
            });
        }
        for labels in refs.values_mut() {
            labels.sort_by_key(|reference| match reference.kind {
                CommitRefKind::Head => 0,
                CommitRefKind::Tag => 1,
                CommitRefKind::LocalBranch => 2,
                CommitRefKind::RemoteBranch => 3,
            });
        }
        tips.sort();
        tips.dedup();
        let mut objects = repo.clone();
        objects.object_cache_size(Some(8 * 1024 * 1024));
        let walk = gix::traverse::commit::topo::Builder::new(objects)
            .with_tips(tips)
            .with_commit_graph(repo.commit_graph_if_enabled().ok().flatten())
            .sorting(gix::traverse::commit::topo::Sorting::TopoOrder)
            .build()
            .map_err(io::Error::other)?
            .peekable();
        Ok(Self { repo, walk, refs })
    }

    pub fn next_page(&mut self, limit: usize) -> io::Result<HistoryPage> {
        let mut commits = Vec::new();
        for _ in 0..limit {
            let Some(info) = self.walk.next() else {
                break;
            };
            let info = info.map_err(io::Error::other)?;
            let commit = self.repo.find_commit(info.id).map_err(io::Error::other)?;
            let message = commit.message().map_err(io::Error::other)?;
            let subject = String::from_utf8_lossy(message.title).into_owned();
            let id = info.id.to_string();
            commits.push(CommitInfo {
                refs: self.refs.get(&id).cloned().unwrap_or_default(),
                id,
                parents: info.parent_ids.iter().map(ToString::to_string).collect(),
                subject,
            });
        }
        Ok(HistoryPage {
            commits,
            has_more: self.walk.peek().is_some(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "chda-history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            let repo = Self(path);
            repo.git(&["init", "-q", "-b", "main"]);
            repo
        }
        fn git(&self, args: &[&str]) -> String {
            let output = Command::new("git")
                .args(["-c", "user.name=test", "-c", "user.email=test@example.test"])
                .args(args)
                .current_dir(&self.0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().into()
        }
        fn commit(&self, subject: &str) -> String {
            self.git(&["commit", "-q", "--allow-empty", "-m", subject]);
            self.git(&["rev-parse", "HEAD"])
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn history_reads_divergence_merge_refs_detached_and_empty() {
        let repo = Repo::new();
        assert!(
            CommitHistory::open(&repo.0)
                .unwrap()
                .next_page(200)
                .unwrap()
                .commits
                .is_empty()
        );
        let base = repo.commit("base");
        repo.git(&["checkout", "-q", "-b", "feature"]);
        let feature = repo.commit("feature change");
        repo.git(&["tag", "-a", "v1", "-m", "annotated tag"]);
        repo.git(&["update-ref", "refs/remotes/origin/feature", &feature]);
        repo.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/feature",
        ]);
        repo.git(&["checkout", "-q", "main"]);
        let main = repo.commit("main change");
        repo.git(&["merge", "-q", "--no-ff", "feature", "-m", "merge feature"]);
        let merge = repo.git(&["rev-parse", "HEAD"]);
        let mut history = CommitHistory::open(&repo.0).unwrap();
        let page = history.next_page(200).unwrap();
        assert!(!page.has_more);
        assert_eq!(page.commits.len(), 4);
        assert_eq!(page.commits[0].id, merge);
        assert_eq!(page.commits[0].parents, vec![main, feature.clone()]);
        assert_eq!(page.commits.last().unwrap().id, base);
        let feature_commit = page
            .commits
            .iter()
            .find(|commit| commit.id == feature)
            .unwrap();
        for (kind, name) in [
            (CommitRefKind::Tag, "v1"),
            (CommitRefKind::LocalBranch, "feature"),
            (CommitRefKind::RemoteBranch, "origin/feature"),
            (CommitRefKind::RemoteBranch, "origin/HEAD"),
        ] {
            assert!(
                feature_commit.refs.contains(&CommitRef {
                    kind: kind.clone(),
                    name: name.into()
                }),
                "missing {kind:?} {name}: {:?}",
                feature_commit.refs
            );
        }
        assert!(
            page.commits[0]
                .refs
                .iter()
                .any(|reference| reference.kind == CommitRefKind::Head)
        );
        repo.git(&["checkout", "-q", "--detach", &feature]);
        let detached = CommitHistory::open(&repo.0)
            .unwrap()
            .next_page(200)
            .unwrap();
        assert!(
            detached
                .commits
                .iter()
                .find(|commit| commit.id == feature)
                .unwrap()
                .refs
                .iter()
                .any(|reference| reference.name == "HEAD · detached")
        );
    }

    #[test]
    fn history_pages_resume_without_duplicates_or_skipping() {
        let repo = Repo::new();
        // commit-tree creates a large linear history quickly without touching the worktree.
        let tree = repo.git(&["mktree"]);
        let mut parent = String::new();
        for i in 0..405 {
            let mut args = vec!["commit-tree", &tree, "-m"];
            let title = format!("commit {i}");
            args.push(&title);
            if !parent.is_empty() {
                args.extend(["-p", &parent]);
            }
            parent = repo.git(&args);
        }
        repo.git(&["update-ref", "refs/heads/main", &parent]);
        let mut history = CommitHistory::open(&repo.0).unwrap();
        let first = history.next_page(200).unwrap();
        let second = history.next_page(200).unwrap();
        let last = history.next_page(200).unwrap();
        assert_eq!(
            (
                first.commits.len(),
                second.commits.len(),
                last.commits.len()
            ),
            (200, 200, 5)
        );
        assert!(first.has_more && second.has_more && !last.has_more);
        assert_eq!(
            first.commits.last().unwrap().parents[0],
            second.commits[0].id
        );
        let ids: std::collections::HashSet<_> = first
            .commits
            .iter()
            .chain(&second.commits)
            .chain(&last.commits)
            .map(|commit| &commit.id)
            .collect();
        assert_eq!(ids.len(), 405);
    }
}
