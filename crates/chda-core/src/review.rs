//! Local review notes: exact diff context, private persistence and manual delivery.
pub use chda_git::{DiffFile, DiffLine, DiffLineKind, ReviewDiff};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub fn read_review_diff(worktree: &Path, base_ref: &str) -> io::Result<ReviewDiff> {
    let base_bytes = chda_agents::ipc::capture_command(
        &mut chda_git::review_base_command(worktree, base_ref)?,
        None,
        Duration::from_secs(5),
        256,
    )?;
    let base = std::str::from_utf8(&base_bytes)
        .map_err(io::Error::other)?
        .trim();
    let read = || {
        chda_agents::ipc::capture_command(
            &mut chda_git::review_diff_command(worktree, base)?,
            None,
            Duration::from_secs(10),
            32 * 1024 * 1024,
        )
    };
    let first = read()?;
    if first != read()? {
        return Err(io::Error::other(
            "Worktree changed while loading. Refresh the diff.",
        ));
    }
    chda_git::parse_review_diff(base, &first)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewSide {
    Old,
    New,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewContext {
    pub kind: i8,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewAnchor {
    pub base: String,
    pub revision: String,
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub side: ReviewSide,
    pub first: u32,
    pub last: u32,
    pub context: Vec<ReviewContext>,
    pub first_offset: usize,
    pub last_offset: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAttachment {
    pub file: usize,
    pub first_row: usize,
    pub last_row: usize,
    pub first: u32,
    pub last: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCopy {
    pub agent: String,
    pub session: String,
    pub at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewNote {
    pub id: u64,
    pub worktree: PathBuf,
    pub anchor: ReviewAnchor,
    pub text: String,
    pub resolved: bool,
    pub copied: Option<ReviewCopy>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewNotebook {
    pub version: u32,
    pub notes: Vec<ReviewNote>,
}
impl Default for ReviewNotebook {
    fn default() -> Self {
        Self {
            version: 1,
            notes: Vec::new(),
        }
    }
}
impl ReviewNotebook {
    pub fn load(data: &Path) -> io::Result<Self> {
        let path = data.join("review-notes.json");
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("Review notes must be a regular file"));
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(io::Error::other(
                "Review notes exceed the supported file size",
            ));
        }
        let notebook: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if notebook.version != 1 || notebook.notes.len() > 10_000 {
            return Err(io::Error::other("Unsupported review note file"));
        }
        let mut ids = std::collections::HashSet::new();
        if notebook.notes.iter().any(|n| {
            n.id == 0
                || !ids.insert(n.id)
                || n.anchor.context.is_empty()
                || n.anchor.first_offset > n.anchor.last_offset
                || n.anchor.last_offset >= n.anchor.context.len()
                || n.anchor.first == 0
                || n.anchor.last < n.anchor.first
        }) {
            return Err(io::Error::other("Invalid review note anchors"));
        }
        Ok(notebook)
    }
    pub fn save(&self, data: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        if bytes.len() > 8 * 1024 * 1024 || self.notes.len() > 10_000 {
            return Err(io::Error::other(
                "Review notes exceed the supported file size",
            ));
        }
        chda_agents::ipc::write_private_atomic(&data.join("review-notes.json"), &bytes)
    }
    pub fn next_id(&self) -> io::Result<u64> {
        self.notes
            .iter()
            .map(|n| n.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Review note IDs exhausted"))
    }
}
fn line_number(line: &DiffLine, side: ReviewSide) -> Option<u32> {
    match side {
        ReviewSide::Old => line.old,
        ReviewSide::New => line.new,
    }
}
fn context(line: &DiffLine) -> ReviewContext {
    ReviewContext {
        kind: match line.kind {
            DiffLineKind::Context => 0,
            DiffLineKind::Added => 1,
            DiffLineKind::Removed => -1,
            DiffLineKind::Header => 2,
        },
        text: line.text.clone(),
    }
}
impl ReviewAnchor {
    pub fn capture(
        diff: &ReviewDiff,
        file: usize,
        first: usize,
        last: usize,
        side: ReviewSide,
    ) -> io::Result<Self> {
        let file = diff
            .files
            .get(file)
            .ok_or_else(|| io::Error::other("Diff file disappeared"))?;
        let a = file
            .lines
            .get(first)
            .ok_or_else(|| io::Error::other("Select a diff line"))?;
        let b = file
            .lines
            .get(last)
            .ok_or_else(|| io::Error::other("Select a diff line"))?;
        let invalid = || io::Error::other("Choose a line or range on one side of one hunk");
        if file.binary || first > last || a.hunk != b.hunk {
            return Err(invalid());
        }
        let first_number = line_number(a, side).ok_or_else(invalid)?;
        let last_number = line_number(b, side).ok_or_else(invalid)?;
        let lines: Vec<_> = file
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.hunk == a.hunk && l.kind != DiffLineKind::Header)
            .collect();
        let i = lines
            .iter()
            .position(|(n, _)| *n == first)
            .ok_or_else(invalid)?;
        let j = lines
            .iter()
            .position(|(n, _)| *n == last)
            .ok_or_else(invalid)?;
        let start = i.saturating_sub(3);
        let end = (j + 4).min(lines.len());
        Ok(Self {
            base: diff.base.clone(),
            revision: diff.revision.clone(),
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            side,
            first: first_number,
            last: last_number,
            context: lines[start..end].iter().map(|(_, l)| context(l)).collect(),
            first_offset: i - start,
            last_offset: j - start,
        })
    }
    pub fn reattach(&self, diff: &ReviewDiff) -> Option<ReviewAttachment> {
        if self.base != diff.base
            || self.context.is_empty()
            || self.first_offset > self.last_offset
            || self.last_offset >= self.context.len()
        {
            return None;
        }
        let mut attached = None;
        for (file_index, file) in diff.files.iter().enumerate().filter(|(_, f)| {
            !f.binary && (f.old_path == self.old_path || f.new_path == self.new_path)
        }) {
            let rows: Vec<_> = file
                .lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.kind != DiffLineKind::Header)
                .collect();
            for window in rows.windows(self.context.len()) {
                if window.first()?.1.hunk != window.last()?.1.hunk
                    || !window
                        .iter()
                        .zip(&self.context)
                        .all(|((_, line), expected)| context(line) == *expected)
                {
                    continue;
                }
                let a = window[self.first_offset];
                let b = window[self.last_offset];
                let first = line_number(a.1, self.side)?;
                let last = line_number(b.1, self.side)?;
                if attached.is_some() {
                    return None;
                }
                attached = Some(ReviewAttachment {
                    file: file_index,
                    first_row: a.0,
                    last_row: b.0,
                    first,
                    last,
                });
            }
        }
        attached
    }
}

/// Shared app repository. All calls perform file work on a background thread.
pub struct ReviewRepository {
    data: Option<PathBuf>,
    state: std::sync::Mutex<Option<(ReviewNotebook, u64)>>,
    generation: std::sync::atomic::AtomicU64,
}
#[derive(Clone, Debug)]
pub struct ReviewSnapshot {
    pub notebook: ReviewNotebook,
    pub generation: u64,
}
impl ReviewRepository {
    pub fn new(data: Option<PathBuf>) -> Self {
        Self {
            data,
            state: std::sync::Mutex::new(None),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }
    fn with_state<T>(
        &self,
        f: impl FnOnce(&mut ReviewNotebook, &mut u64) -> io::Result<T>,
    ) -> io::Result<T> {
        let data = self.data.as_ref().ok_or_else(|| {
            io::Error::other("A local data directory is required to save review notes")
        })?;
        let mut guard = self
            .state
            .lock()
            .map_err(|_| io::Error::other("Review notes are unavailable"))?;
        if guard.is_none() {
            *guard = Some((ReviewNotebook::load(data)?, 0));
        }
        let (notebook, generation) = guard.as_mut().unwrap();
        let result = f(notebook, generation);
        self.generation
            .store(*generation, std::sync::atomic::Ordering::Release);
        result
    }
    /// A non-blocking guard for a background-validated clipboard preview.
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn snapshot(&self) -> io::Result<ReviewSnapshot> {
        self.with_state(|notebook, generation| {
            Ok(ReviewSnapshot {
                notebook: notebook.clone(),
                generation: *generation,
            })
        })
    }
    pub fn save_note(
        &self,
        expected: Option<&ReviewNote>,
        worktree: &Path,
        anchor: ReviewAnchor,
        text: String,
    ) -> io::Result<ReviewSnapshot> {
        self.with_state(|notebook,generation|{
            let mut next=notebook.clone();
            if let Some(expected)=expected {
                let current=next.notes.iter_mut().find(|n|n.id==expected.id&&n.worktree==worktree).ok_or_else(||io::Error::other("Review note disappeared"))?;
                if current!=expected {return Err(io::Error::other("This note changed in another window. Copy the draft and review the latest note."));}
                current.anchor=anchor;current.text=text;current.copied=None;
            }else {
                next.notes.push(ReviewNote{id:next.next_id()?,worktree:worktree.into(),anchor,text,resolved:false,copied:None});
            }
            next.save(self.data.as_ref().unwrap())?;*notebook=next;*generation+=1;
            Ok(ReviewSnapshot{notebook:notebook.clone(),generation:*generation})
        })
    }
    pub fn resolve_note(
        &self,
        expected: &ReviewNote,
        resolved: bool,
    ) -> io::Result<ReviewSnapshot> {
        self.with_state(|notebook, generation| {
            let mut next = notebook.clone();
            let current = next
                .notes
                .iter_mut()
                .find(|n| n.id == expected.id && n.worktree == expected.worktree)
                .ok_or_else(|| io::Error::other("Review note disappeared"))?;
            if current != expected {
                return Err(io::Error::other(
                    "This note changed in another window. Review its latest text.",
                ));
            }
            current.resolved = resolved;
            next.save(self.data.as_ref().unwrap())?;
            *notebook = next;
            *generation += 1;
            Ok(ReviewSnapshot {
                notebook: notebook.clone(),
                generation: *generation,
            })
        })
    }
    pub fn mark_copied(
        &self,
        expected_generation: u64,
        ids: &[u64],
        worktree: &Path,
        copy: ReviewCopy,
    ) -> io::Result<ReviewSnapshot> {
        self.with_state(|notebook, generation| {
            if *generation != expected_generation {
                return Err(io::Error::other(
                    "Review notes changed. Prepare the batch again.",
                ));
            }
            let mut next = notebook.clone();
            for id in ids {
                let note = next
                    .notes
                    .iter_mut()
                    .find(|n| n.id == *id && n.worktree == worktree)
                    .ok_or_else(|| io::Error::other("Review note disappeared"))?;
                note.copied = Some(copy.clone());
            }
            next.save(self.data.as_ref().unwrap())?;
            *notebook = next;
            *generation += 1;
            Ok(ReviewSnapshot {
                notebook: notebook.clone(),
                generation: *generation,
            })
        })
    }
}

/// Plain text for explicit manual delivery; stale locations are never presented as current.
pub fn review_batch(
    worktree: &Path,
    diff: &ReviewDiff,
    notes: &[ReviewNote],
) -> io::Result<String> {
    let mut body = format!(
        "Review notes for {:?}\nBase {} · diff revision {}\n",
        worktree, diff.base, diff.revision
    );
    for note in notes {
        if note.worktree != worktree {
            return Err(io::Error::other(
                "Review notes belong to a different worktree",
            ));
        }
        let (path, first, last, state) = match note.anchor.reattach(diff) {
            Some(at) => {
                let file = &diff.files[at.file];
                (
                    match note.anchor.side {
                        ReviewSide::Old => &file.old_path,
                        ReviewSide::New => &file.new_path,
                    },
                    at.first,
                    at.last,
                    "Current diff".to_owned(),
                )
            }
            None => (
                match note.anchor.side {
                    ReviewSide::Old => &note.anchor.old_path,
                    ReviewSide::New => &note.anchor.new_path,
                },
                note.anchor.first,
                note.anchor.last,
                format!(
                    "Stale anchor — verify against original revision {}",
                    note.anchor.revision
                ),
            ),
        };
        body.push_str(&format!(
            "\nNote {} · {:?} · {:?} lines {}–{} · {}\n{}\n",
            note.id, path, note.anchor.side, first, last, state, note.text
        ));
        if body.len() > 1024 * 1024 {
            return Err(io::Error::other("Select fewer notes for one copy batch"));
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn diff(offset: u32) -> ReviewDiff {
        ReviewDiff {
            base: "base".into(),
            revision: format!("r{offset}"),
            files: vec![DiffFile {
                old_path: "before.txt".into(),
                new_path: "after.txt".into(),
                status: "R".into(),
                binary: false,
                lines: (0..7)
                    .map(|n| DiffLine {
                        kind: DiffLineKind::Added,
                        text: format!("line{n}"),
                        old: None,
                        new: Some(offset + n + 1),
                        hunk: 1,
                    })
                    .collect(),
            }],
        }
    }
    #[test]
    fn notes_follow_unique_context_after_line_movement_without_resolving() {
        let original = diff(0);
        let anchor = ReviewAnchor::capture(&original, 0, 2, 3, ReviewSide::New).unwrap();
        let moved = diff(10);
        let attached = anchor.reattach(&moved).unwrap();
        assert_eq!((attached.first, attached.last), (13, 14));
        assert_eq!(
            anchor.revision, "r0",
            "original revision stays attributable"
        );
    }
    #[test]
    fn changed_repeated_or_foreign_base_context_is_stale() {
        let original = diff(0);
        let anchor = ReviewAnchor::capture(&original, 0, 2, 3, ReviewSide::New).unwrap();
        let mut changed = diff(10);
        changed.files[0].lines[2].text = "changed".into();
        assert!(anchor.reattach(&changed).is_none());
        let mut repeated = diff(10);
        let copy = repeated.files[0].lines.clone();
        repeated.files[0].lines.extend(copy);
        assert!(anchor.reattach(&repeated).is_none());
        let mut foreign = diff(10);
        foreign.base = "other".into();
        assert!(anchor.reattach(&foreign).is_none());
    }

    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "chda-notes-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn private_notes_round_trip_and_stale_batches_keep_the_original_location() {
        let root = Root::new();
        let repository = ReviewRepository::new(Some(root.0.clone()));
        let original = diff(0);
        let anchor = ReviewAnchor::capture(&original, 0, 2, 3, ReviewSide::New).unwrap();
        let saved = repository
            .save_note(
                None,
                Path::new("/worktree"),
                anchor,
                "Keep 안녕 🧭 and | pipes".into(),
            )
            .unwrap();
        let loaded = ReviewNotebook::load(&root.0).unwrap();
        assert_eq!(loaded.notes.len(), 1);
        assert_eq!(loaded.notes[0], saved.notebook.notes[0]);
        let mut changed = diff(10);
        changed.files[0].lines[2].text = "replaced".into();
        let batch = review_batch(Path::new("/worktree"), &changed, &loaded.notes).unwrap();
        assert!(batch.contains("Stale anchor"));
        assert!(batch.contains("lines 3–4"));
        assert!(batch.contains("Keep 안녕 🧭 and | pipes"));
        assert!(review_batch(Path::new("/other-worktree"), &changed, &loaded.notes).is_err());
    }
    #[test]
    fn concurrent_edits_cannot_overwrite_a_changed_note_and_copying_never_resolves() {
        let root = Root::new();
        let repository = ReviewRepository::new(Some(root.0.clone()));
        let original = diff(0);
        let anchor = ReviewAnchor::capture(&original, 0, 2, 3, ReviewSide::New).unwrap();
        let saved = repository
            .save_note(None, Path::new("/worktree"), anchor.clone(), "first".into())
            .unwrap();
        let old = saved.notebook.notes[0].clone();
        let newer = repository
            .save_note(
                Some(&old),
                Path::new("/worktree"),
                anchor.clone(),
                "new text".into(),
            )
            .unwrap();
        assert!(
            repository
                .save_note(
                    Some(&old),
                    Path::new("/worktree"),
                    anchor,
                    "late text".into()
                )
                .is_err()
        );
        assert!(
            repository
                .mark_copied(
                    saved.generation,
                    &[old.id],
                    Path::new("/worktree"),
                    ReviewCopy {
                        agent: "codex".into(),
                        session: "exact".into(),
                        at: 1
                    }
                )
                .is_err()
        );
        let copied = repository
            .mark_copied(
                newer.generation,
                &[old.id],
                Path::new("/worktree"),
                ReviewCopy {
                    agent: "codex".into(),
                    session: "exact".into(),
                    at: 1,
                },
            )
            .unwrap();
        assert_eq!(copied.notebook.notes[0].text, "new text");
        assert!(!copied.notebook.notes[0].resolved);
        assert_eq!(
            copied.notebook.notes[0].copied.as_ref().unwrap().session,
            "exact"
        );
    }
    #[test]
    fn unsupported_note_files_are_preserved_without_creating_an_empty_replacement() {
        let root = Root::new();
        let path = root.0.join("review-notes.json");
        let bytes = b"{\"version\":99,\"notes\":[]}";
        std::fs::write(&path, bytes).unwrap();
        let repository = ReviewRepository::new(Some(root.0.clone()));
        assert!(repository.snapshot().is_err());
        let original = diff(0);
        let anchor = ReviewAnchor::capture(&original, 0, 2, 3, ReviewSide::New).unwrap();
        assert!(
            repository
                .save_note(None, Path::new("/worktree"), anchor, "draft".into())
                .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn real_git_diff_keeps_rename_paths_binary_state_and_never_runs_external_helpers() {
        let root = Root::new();
        let git = |args: &[&str]| {
            let result = std::process::Command::new("git")
                .current_dir(&root.0)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&result.stderr)
            );
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Fixture"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        let old = "old name\n안녕.txt";
        let new = "new name\n안녕.txt";
        std::fs::write(
            root.0.join(old),
            "one\ntwo\nthree\nfour\nfive\nsix\nseven\n",
        )
        .unwrap();
        std::fs::write(root.0.join("binary"), [0, 1, 2]).unwrap();
        std::fs::write(root.0.join(".gitattributes"), "* diff=fixture\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "fixture"]);
        git(&["mv", old, new]);
        std::fs::write(
            root.0.join(new),
            "one\ntwo\nchanged\nfour\nfive\nsix\nseven\n",
        )
        .unwrap();
        std::fs::write(root.0.join("binary"), [0, 1, 4]).unwrap();
        std::fs::write(root.0.join("untracked"), "not included").unwrap();
        git(&["config", "diff.external", "touch external-was-run"]);
        git(&["config", "diff.fixture.textconv", "touch textconv-was-run"]);
        git(&["config", "diff.suppressBlankEmpty", "true"]);
        let loaded = read_review_diff(&root.0, "main").unwrap();
        let renamed = loaded
            .files
            .iter()
            .find(|f| f.new_path == Path::new(new))
            .unwrap();
        assert_eq!(renamed.old_path, Path::new(old));
        assert!(renamed.status.starts_with('R'));
        assert!(
            renamed
                .lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Added && l.text == "changed" && l.new == Some(3))
        );
        assert!(
            loaded
                .files
                .iter()
                .find(|f| f.new_path == Path::new("binary"))
                .unwrap()
                .binary
        );
        assert!(
            !loaded
                .files
                .iter()
                .any(|f| f.new_path == Path::new("untracked"))
        );
        assert!(!root.0.join("external-was-run").exists());
        assert!(!root.0.join("textconv-was-run").exists());
        assert!(read_review_diff(&root.0, "--output=unsafe").is_err());
    }
    #[test]
    fn review_tabs_reuse_worktrees_restore_without_ptys_and_keep_history_separate() {
        let mut workspace = crate::Workspace::new();
        let first = workspace.open_review("/worktree".into(), "/repository".into(), "main".into());
        assert_eq!(
            first,
            workspace.open_review("/worktree".into(), "/repository".into(), "main".into())
        );
        workspace.open_graph("/repository".into());
        assert_eq!(workspace.tabs().len(), 2);
        workspace.activate_tab_id(first);
        assert!(workspace.focused_pane().is_none());
        assert!(workspace.active_tab().unwrap().panes().is_empty());
        let saved = workspace.snapshot();
        let loaded: crate::SavedWindow =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        let mut restored = crate::Workspace::new();
        let (panes, _) = restored.restore(&loaded);
        assert!(panes.is_empty());
        assert_eq!(restored.snapshot(), saved);
        assert_eq!(workspace.handoff_snapshot(), saved);
    }
    #[test]
    fn target_roots_distinguish_linked_worktrees_and_nested_repositories() {
        let root = Root::new();
        let run = |cwd: &Path, args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .current_dir(cwd)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        run(&root.0, &["init", "-q", "-b", "main"]);
        run(
            &root.0,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "base",
            ],
        );
        let linked = root.0.join("linked");
        run(
            &root.0,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "linked",
                linked.to_str().unwrap(),
            ],
        );
        let child = linked.join("subfolder");
        std::fs::create_dir(&child).unwrap();
        assert_eq!(
            crate::worktree_root(&child).unwrap(),
            linked.canonicalize().unwrap()
        );
        let nested = child.join("nested");
        std::fs::create_dir(&nested).unwrap();
        run(&nested, &["init", "-q"]);
        assert_eq!(
            crate::worktree_root(&nested).unwrap(),
            nested.canonicalize().unwrap()
        );
        assert_ne!(
            crate::worktree_root(&nested).unwrap(),
            linked.canonicalize().unwrap()
        );
    }
}
