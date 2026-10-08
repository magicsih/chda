//! Explicit worktree preparation and app-owned consent, separate from imported configuration.
use chda_config::PreparationPlan;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparationIdentity {
    pub primary: PathBuf,
    pub repository: PathBuf,
    pub created_seconds: u64,
    pub created_nanos: u32,
}
#[derive(Clone, Debug)]
pub struct PreparationPreview {
    pub identity: PreparationIdentity,
    pub destination: PathBuf,
    pub plan: PreparationPlan,
    pub retry: bool,
    pub files: Vec<PreparationFile>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparationFile {
    pub relative: PathBuf,
    pub keep_existing: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparationProgress {
    Copying,
    Command { index: usize, total: usize },
    Complete,
}
/// Only explicit consent or an exact match in private app data can construct this token.
pub struct AuthorizedPreparation {
    preview: PreparationPreview,
}
impl PreparationPreview {
    pub fn inspect(
        primary: &Path,
        destination: &Path,
        plan: PreparationPlan,
        retry: bool,
    ) -> io::Result<Self> {
        let repository = chda_git::repository_directory(primary)?;
        let created = std::fs::metadata(&repository)?
            .created()?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?;
        let mut preview = Self {
            identity: PreparationIdentity {
                primary: primary.canonicalize()?,
                repository,
                created_seconds: created.as_secs(),
                created_nanos: created.subsec_nanos(),
            },
            destination: destination.canonicalize()?,
            plan,
            retry,
            files: Vec::new(),
        };
        preview.validate()?;
        preview.files = preview.inspect_files()?;
        Ok(preview)
    }
    fn validate(&self) -> io::Result<()> {
        if self.plan.commands.len() > 128
            || self.plan.files.len() > 1024
            || serde_json::to_vec(&self.plan)
                .map_err(io::Error::other)?
                .len()
                > 1024 * 1024
        {
            return Err(io::Error::other(
                "Preparation plan exceeds the supported size",
            ));
        }
        if chda_git::main_worktree(&self.destination)?.canonicalize()? != self.identity.primary
            || self.destination == self.identity.primary
        {
            return Err(io::Error::other(
                "Preparation requires a new worktree of the selected repository",
            ));
        }
        if self
            .plan
            .commands
            .iter()
            .any(|c| c.trim().is_empty() || c.contains('\0'))
        {
            return Err(io::Error::other(
                "Setup commands must be nonempty and contain no NUL",
            ));
        }
        let mut files = std::collections::HashSet::new();
        let mut input = Vec::new();
        for path in &self.plan.files {
            if path.as_os_str().is_empty()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
                || !files.insert(path.clone())
            {
                return Err(io::Error::other(
                    "Choose unique relative file paths without traversal",
                ));
            }
            let path = path
                .to_str()
                .ok_or_else(|| io::Error::other("Preparation paths must be UTF-8"))?;
            if path.contains('\0') {
                return Err(io::Error::other("File paths must contain no NUL"));
            }
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
        if !input.is_empty() {
            let output = chda_agents::ipc::capture_command(
                &mut chda_git::preparation_ignored_command(&self.identity.primary),
                Some(&input),
                std::time::Duration::from_secs(5),
                1024 * 1024,
            )?;
            if output != input {
                return Err(io::Error::other(
                    "Copy sources must all be untracked and gitignored",
                ));
            }
        }
        Ok(())
    }
    fn inspect_files(&self) -> io::Result<Vec<PreparationFile>> {
        let source = open_root(&self.identity.primary)?;
        let destination = open_root(&self.destination)?;
        self.plan
            .files
            .iter()
            .map(|path| {
                let file = open_source(&source, path)?;
                if !file.metadata()?.is_file() {
                    return Err(io::Error::other("Only regular local files can be copied"));
                }
                let keep_existing = match parent_dir(&destination, path, false) {
                    Ok(parent) => match parent.symlink_metadata(path.file_name().unwrap()) {
                        Ok(m) if self.retry && m.is_file() => true,
                        Ok(_) => {
                            return Err(io::Error::new(
                                io::ErrorKind::AlreadyExists,
                                "Destination exists; use Retry to keep regular files",
                            ));
                        }
                        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                        Err(e) => return Err(e),
                    },
                    Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                    Err(e) => return Err(e),
                };
                Ok(PreparationFile {
                    relative: path.clone(),
                    keep_existing,
                })
            })
            .collect()
    }
    fn fresh(&self) -> io::Result<Self> {
        let fresh = Self::inspect(
            &self.identity.primary,
            &self.destination,
            self.plan.clone(),
            self.retry,
        )?;
        if fresh.identity != self.identity {
            return Err(io::Error::other(
                "Repository changed since preparation review",
            ));
        }
        if fresh.files != self.files {
            return Err(io::Error::other(
                "Destination file state changed; review preparation again",
            ));
        }
        Ok(fresh)
    }
    pub fn authorize(&self, data: &Path) -> io::Result<AuthorizedPreparation> {
        // The production caller must be the exact preview's explicit confirmation action.
        self.fresh()?;
        let _lock = APPROVAL_LOCK
            .lock()
            .map_err(|_| io::Error::other("Preparation approval lock unavailable"))?;
        let mut approvals = Approvals::load(data)?;
        approvals
            .entries
            .retain(|(id, _)| id.primary != self.identity.primary);
        approvals
            .entries
            .push((self.identity.clone(), self.plan.clone()));
        approvals.save(data)?;
        Ok(AuthorizedPreparation {
            preview: self.clone(),
        })
    }
    pub fn remembered(&self, data: &Path) -> io::Result<Option<AuthorizedPreparation>> {
        self.fresh()?;
        let _lock = APPROVAL_LOCK
            .lock()
            .map_err(|_| io::Error::other("Preparation approval lock unavailable"))?;
        Ok(Approvals::load(data)?
            .entries
            .iter()
            .any(|(id, plan)| id == &self.identity && plan == &self.plan)
            .then(|| AuthorizedPreparation {
                preview: self.clone(),
            }))
    }
}
static APPROVAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[derive(Serialize, Deserialize)]
struct Approvals {
    version: u32,
    entries: Vec<(PreparationIdentity, PreparationPlan)>,
}
impl Approvals {
    fn load(data: &Path) -> io::Result<Self> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
        use std::io::Read;
        let read = || -> io::Result<Vec<u8>> {
            let dir = open_root(data)?;
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No).nonblock(true);
            let file = dir
                .open_with("worktree-preparation-approvals.json", &options)?
                .into_std();
            if !file.metadata()?.is_file() {
                return Err(io::Error::other(
                    "Preparation approval data must be a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        let bytes = match read() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    version: 1,
                    entries: Vec::new(),
                });
            }
            Err(e) => return Err(e),
        };
        if bytes.len() > 1024 * 1024 {
            return Err(io::Error::other(
                "Preparation approvals exceed the supported file size",
            ));
        }
        let approvals: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if approvals.version != 1 {
            return Err(io::Error::other("Unsupported preparation approval data"));
        }
        Ok(approvals)
    }
    fn save(&self, data: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        if bytes.len() > 1024 * 1024 {
            return Err(io::Error::other(
                "Preparation approvals exceed the supported file size",
            ));
        }
        chda_agents::ipc::write_private_atomic(
            &data.join("worktree-preparation-approvals.json"),
            &bytes,
        )
    }
}
impl AuthorizedPreparation {
    pub fn run(
        &self,
        shell: &Path,
        environment: &[(String, String)],
        cancelled: &std::sync::atomic::AtomicBool,
        mut progress: impl FnMut(PreparationProgress),
    ) -> io::Result<()> {
        use std::process::{Command, Stdio};
        check_cancelled(cancelled)?;
        progress(PreparationProgress::Copying);
        self.copy_files(cancelled)?;
        for (index, command) in self.preview.plan.commands.iter().enumerate() {
            check_cancelled(cancelled)?;
            progress(PreparationProgress::Command {
                index,
                total: self.preview.plan.commands.len(),
            });
            let mut child = Command::new(shell);
            child
                .arg("-c")
                .arg(command)
                .current_dir(&self.preview.destination)
                .envs(environment.iter().map(|(key, value)| (key, value)))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let status = chda_agents::ipc::with_process(&mut child, |child| {
                loop {
                    check_cancelled(cancelled)?;
                    if let Some(status) = child.try_wait()? {
                        break Ok(status);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
            })?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "Setup command {} failed — exit {}",
                    index + 1,
                    status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "terminated".into())
                )));
            }
        }
        check_cancelled(cancelled)?;
        progress(PreparationProgress::Complete);
        Ok(())
    }
    pub fn copy_files(&self, cancelled: &std::sync::atomic::AtomicBool) -> io::Result<()> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        use std::io::{Read, Write};
        let fresh = self.preview.fresh()?;
        let source = open_root(&fresh.identity.primary)?;
        let destination = open_root(&fresh.destination)?;
        for path in &fresh.plan.files {
            check_cancelled(cancelled)?;
            let leaf = path
                .file_name()
                .ok_or_else(|| io::Error::other("Choose a file"))?;
            let mut input = open_source(&source, path)?;
            let metadata = input.metadata()?;
            if !metadata.is_file() {
                return Err(io::Error::other("Only regular local files can be copied"));
            }
            let destination_parent = parent_dir(&destination, path, true)?;
            match destination_parent.symlink_metadata(leaf) {
                Ok(m) if fresh.retry && m.is_file() => continue,
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Destination exists; review retry or keep the existing file",
                    ));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            let mut write_options = cap_std::fs::OpenOptions::new();
            write_options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            let mut output = destination_parent
                .open_with(leaf, &write_options)?
                .into_std();
            chda_agents::ipc::protect_file(&output)?;
            let mut buffer = [0; 64 * 1024];
            loop {
                check_cancelled(cancelled)?;
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                output.write_all(&buffer[..n])?;
            }
            output.sync_all()?;
            output.set_permissions(metadata.permissions())?;
        }
        Ok(())
    }
}
fn open_source(root: &cap_std::fs::Dir, path: &Path) -> io::Result<std::fs::File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
    let parent = parent_dir(root, path, false)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    parent
        .open_with(path.file_name().unwrap(), &options)
        .map(|f| f.into_std())
}
fn check_cancelled(cancelled: &std::sync::atomic::AtomicBool) -> io::Result<()> {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Preparation cancelled; worktree and existing files kept",
        ))
    } else {
        Ok(())
    }
}
/// Bootstrap from the filesystem root and open every directory component without symlinks.
fn open_root(path: &Path) -> io::Result<cap_std::fs::Dir> {
    use cap_fs_ext::DirExt;
    let mut anchor = PathBuf::new();
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(p) => anchor.push(p.as_os_str()),
            std::path::Component::RootDir => anchor.push(component.as_os_str()),
            std::path::Component::Normal(n) => names.push(n),
            _ => {
                return Err(io::Error::other(
                    "Root path must be absolute without traversal",
                ));
            }
        }
    }
    if !anchor.is_absolute() {
        return Err(io::Error::other("Preparation root must be absolute"));
    }
    let mut dir = cap_std::fs::Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?;
    for name in names {
        dir = dir.open_dir_nofollow(name)?;
    }
    Ok(dir)
}
fn parent_dir(root: &cap_std::fs::Dir, path: &Path, create: bool) -> io::Result<cap_std::fs::Dir> {
    use cap_fs_ext::DirExt;
    let mut dir = root.try_clone()?;
    for component in path.parent().unwrap_or(Path::new("")).components() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::other(
                "Relative file path must have no traversal",
            ));
        };
        dir = match dir.open_dir_nofollow(name) {
            Ok(next) => next,
            Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                match dir.create_dir(name) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
                dir.open_dir_nofollow(name)?
            }
            Err(e) => return Err(e),
        };
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    struct Fixture {
        root: PathBuf,
        primary: PathBuf,
        worktree: PathBuf,
        data: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "chda-preparation-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            let root = root.canonicalize().unwrap();
            let primary = root.join("primary");
            let worktree = root.join("worktree");
            let data = root.join("data");
            std::fs::create_dir(&primary).unwrap();
            let git = |cwd: &Path, args: &[&str]| {
                let result = std::process::Command::new("git")
                    .current_dir(cwd)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
            };
            git(&primary, &["init", "-q", "-b", "main"]);
            std::fs::write(primary.join(".gitignore"), "*.env\nlocal/\n").unwrap();
            std::fs::write(primary.join("tracked.txt"), "tracked\n").unwrap();
            git(&primary, &["add", "."]);
            git(
                &primary,
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "commit",
                    "-q",
                    "-m",
                    "base",
                ],
            );
            git(
                &primary,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "new",
                    worktree.to_str().unwrap(),
                ],
            );
            Self {
                root,
                primary,
                worktree,
                data,
            }
        }
        fn plan(&self) -> PreparationPlan {
            PreparationPlan {
                commands: vec![],
                files: vec!["local/한글 space.env".into()],
            }
        }
        fn source(&self) {
            std::fs::create_dir(self.primary.join("local")).unwrap();
            std::fs::write(
                self.primary.join("local/한글 space.env"),
                "synthetic fixture only",
            )
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn explicit_ignored_copy_preserves_primary_and_existing_retry_edits() {
        let f = Fixture::new();
        f.source();
        let p = PreparationPreview::inspect(&f.primary, &f.worktree, f.plan(), false).unwrap();
        assert!(p.remembered(&f.data).unwrap().is_none());
        let approved = p.authorize(&f.data).unwrap();
        approved.copy_files(&AtomicBool::new(false)).unwrap();
        assert_eq!(
            std::fs::read(f.worktree.join("local/한글 space.env")).unwrap(),
            std::fs::read(f.primary.join("local/한글 space.env")).unwrap()
        );
        std::fs::write(f.worktree.join("local/한글 space.env"), "user edit").unwrap();
        assert!(approved.copy_files(&AtomicBool::new(false)).is_err());
        let retry = PreparationPreview::inspect(&f.primary, &f.worktree, f.plan(), true)
            .unwrap()
            .remembered(&f.data)
            .unwrap()
            .unwrap();
        retry.copy_files(&AtomicBool::new(false)).unwrap();
        assert_eq!(
            std::fs::read_to_string(f.worktree.join("local/한글 space.env")).unwrap(),
            "user edit"
        );
    }
    #[test]
    fn imported_and_changed_plans_never_inherit_consent() {
        let f = Fixture::new();
        f.source();
        let p = PreparationPreview::inspect(&f.primary, &f.worktree, f.plan(), false).unwrap();
        p.authorize(&f.data).unwrap();
        let mut plan = f.plan();
        plan.commands.push("true".into());
        let changed = PreparationPreview::inspect(&f.primary, &f.worktree, plan, false).unwrap();
        assert!(changed.remembered(&f.data).unwrap().is_none());
    }
    #[test]
    fn preview_rejects_tracked_nonignored_missing_directories_and_escape_paths() {
        let f = Fixture::new();
        f.source();
        std::fs::write(f.primary.join("untracked.txt"), "fixture").unwrap();
        for path in [
            "tracked.txt",
            "untracked.txt",
            "missing.env",
            "local",
            "../outside.env",
            "/outside.env",
            "local/../tracked.txt",
        ] {
            let plan = PreparationPlan {
                commands: vec![],
                files: vec![path.into()],
            };
            assert!(
                PreparationPreview::inspect(&f.primary, &f.worktree, plan, false).is_err(),
                "{path}"
            );
        }
        std::fs::write(f.primary.join("forced.env"), "fixture").unwrap();
        assert!(
            std::process::Command::new("git")
                .current_dir(&f.primary)
                .args(["add", "-f", "forced.env"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            PreparationPreview::inspect(
                &f.primary,
                &f.worktree,
                PreparationPlan {
                    commands: vec![],
                    files: vec!["forced.env".into()]
                },
                false
            )
            .is_err()
        );
        assert!(!f.worktree.join("local").exists());
    }
    #[test]
    fn symlinked_source_and_destination_components_are_rejected() {
        use cap_fs_ext::DirExt;
        let f = Fixture::new();
        f.source();
        let source = open_root(&f.primary).unwrap();
        source
            .symlink_file("local/한글 space.env", "link.env")
            .unwrap();
        assert!(
            PreparationPreview::inspect(
                &f.primary,
                &f.worktree,
                PreparationPlan {
                    commands: vec![],
                    files: vec!["link.env".into()]
                },
                false
            )
            .is_err()
        );
        source.symlink_dir("local", "alias").unwrap();
        std::fs::write(f.primary.join(".gitignore"), "*.env\nlocal/\nalias/\n").unwrap();
        assert!(
            PreparationPreview::inspect(
                &f.primary,
                &f.worktree,
                PreparationPlan {
                    commands: vec![],
                    files: vec!["alias/한글 space.env".into()]
                },
                false
            )
            .is_err()
        );
        let p = PreparationPreview::inspect(&f.primary, &f.worktree, f.plan(), false).unwrap();
        let approved = p.authorize(&f.data).unwrap();
        let outside = f.root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        open_root(&f.worktree)
            .unwrap()
            .symlink_dir("../outside", "local")
            .unwrap();
        assert!(approved.copy_files(&AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
    }
    #[test]
    fn corrupt_and_oversized_approval_data_is_preserved() {
        let f = Fixture::new();
        f.source();
        let p = PreparationPreview::inspect(&f.primary, &f.worktree, f.plan(), false).unwrap();
        std::fs::create_dir(&f.data).unwrap();
        let path = f.data.join("worktree-preparation-approvals.json");
        for bytes in [
            b"{broken".to_vec(),
            b"{\"version\":2,\"entries\":[]}".to_vec(),
            vec![b'x'; 1024 * 1024 + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(p.remembered(&f.data).is_err());
            assert!(p.authorize(&f.data).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn setup_runs_after_copy_in_order_and_failure_stops_later_commands() {
        let f = Fixture::new();
        f.source();
        let mut plan = f.plan();
        plan.commands = vec![
            "test -f 'local/한글 space.env' && printf first > order".into(),
            "printf second >> order; printf private-output; printf private-error >&2; exit 7"
                .into(),
            "printf must-not-run > later".into(),
        ];
        let p = PreparationPreview::inspect(&f.primary, &f.worktree, plan, false).unwrap();
        let approved = p.authorize(&f.data).unwrap();
        let mut progress = Vec::new();
        let e = approved
            .run(Path::new("/bin/sh"), &[], &AtomicBool::new(false), |p| {
                progress.push(p)
            })
            .unwrap_err();
        assert_eq!(
            std::fs::read_to_string(f.worktree.join("order")).unwrap(),
            "firstsecond"
        );
        assert!(!f.worktree.join("later").exists());
        assert!(e.to_string().contains("exit 7"));
        assert!(!e.to_string().contains("private"));
        assert_eq!(
            progress,
            vec![
                PreparationProgress::Copying,
                PreparationProgress::Command { index: 0, total: 3 },
                PreparationProgress::Command { index: 1, total: 3 }
            ]
        );
        assert!(!f.primary.join("order").exists());
    }
    #[test]
    fn cancel_reaps_setup_and_prevents_success_or_later_steps() {
        let f = Fixture::new();
        let plan = PreparationPlan {
            files: vec![],
            commands: vec!["sleep 30".into(), "touch later".into()],
        };
        let p = PreparationPreview::inspect(&f.primary, &f.worktree, plan, false).unwrap();
        let approved = p.authorize(&f.data).unwrap();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let notify = cancel.clone();
        let start = std::time::Instant::now();
        let mut complete = false;
        let result = approved.run(Path::new("/bin/sh"), &[], &cancel, |p| {
            if matches!(p, PreparationProgress::Command { index: 0, .. }) {
                let notify = notify.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(75));
                    notify.store(true, std::sync::atomic::Ordering::Release);
                });
            }
            complete |= p == PreparationProgress::Complete;
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        assert!(!complete && !f.worktree.join("later").exists());
    }
    #[test]
    fn special_file_is_rejected_without_waiting_for_a_writer() {
        let f = Fixture::new();
        let mut command = std::process::Command::new("mkfifo");
        command.arg(f.primary.join("special.env"));
        chda_agents::ipc::capture_command(
            &mut command,
            None,
            std::time::Duration::from_secs(3),
            1024,
        )
        .unwrap();
        let start = std::time::Instant::now();
        assert!(
            PreparationPreview::inspect(
                &f.primary,
                &f.worktree,
                PreparationPlan {
                    commands: vec![],
                    files: vec!["special.env".into()]
                },
                false
            )
            .is_err()
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
    #[test]
    fn a_replaced_repository_at_the_same_path_requires_new_consent() {
        let f = Fixture::new();
        f.source();
        let p = PreparationPreview::inspect(&f.primary, &f.worktree, f.plan(), false).unwrap();
        p.authorize(&f.data).unwrap();
        std::fs::rename(f.primary.join(".git"), f.root.join("previous-git")).unwrap();
        std::fs::remove_dir_all(&f.worktree).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "replacement",
            ],
            vec![
                "worktree",
                "add",
                "-q",
                "-b",
                "new",
                f.worktree.to_str().unwrap(),
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .current_dir(&f.primary)
                    .args(args)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let replacement =
            PreparationPreview::inspect(&f.primary, &f.worktree, f.plan(), false).unwrap();
        assert_ne!(p.identity, replacement.identity);
        assert!(replacement.remembered(&f.data).unwrap().is_none());
        assert!(p.authorize(&f.data).is_err());
    }
}
