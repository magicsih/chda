//! macOS update preparation and authenticated recovery app retention.
use chda_core::{handoff::UpdateHandoff, self_update::UpdateProgress};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(crate) struct UpdateJob {
    pub handoff: UpdateHandoff,
}
impl UpdateJob {
    pub fn progress(&self) -> Option<UpdateProgress> {
        let progress = (|| {
            use std::io::{Read, Seek, SeekFrom};
            let mut file =
                std::fs::File::open(self.handoff.directory.join("progress.jsonl")).ok()?;
            let length = file.metadata().ok()?.len();
            let start = length.saturating_sub(64 * 1024);
            file.seek(SeekFrom::Start(start)).ok()?;
            let mut text = String::new();
            file.read_to_string(&mut text).ok()?;
            // Ignore a partially written final line.
            let complete = text.rsplit_once('\n')?.0;
            complete
                .lines()
                .rev()
                .find_map(|s| serde_json::from_str(s).ok())
        })();
        if self.handoff.directory.join("helper-exited").exists()
            && !matches!(
                progress,
                Some(UpdateProgress::Installed | UpdateProgress::Failed { .. })
            )
        {
            return Some(UpdateProgress::Failed {
                message: "The updater stopped. Your sessions are unchanged; try again.".into(),
            });
        }
        progress
    }
    pub fn signal(&self, name: &str) -> io::Result<()> {
        std::fs::write(self.handoff.directory.join(name), b"1\n")
    }
}

/// Remove a completed attempt only after its native helper has exited. A
/// recovery GUI keeps its own bundle until a later successful update/exit.
pub(crate) fn discard(job: UpdateJob) {
    if std::env::current_exe().is_ok_and(|p| p.starts_with(&job.handoff.directory)) {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..300 {
            if job.handoff.directory.join("helper-exited").exists() {
                let _ = std::fs::remove_dir_all(&job.handoff.directory);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

pub(crate) fn start(version: String, target: Option<PathBuf>) -> Result<UpdateJob, String> {
    #[cfg(target_os = "macos")]
    {
        macos::start(&version, target).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (version, target);
        Err("In-app updates are available in the signed macOS app.".into())
    }
}

pub(crate) fn verify(exe: &Path, recovery: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::verify(exe, recovery)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (exe, recovery);
        Err(io::Error::other("macOS app verification unavailable"))
    }
}

pub(crate) fn recovery_notice() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/osascript").args([
            "-e", "display alert \"chda could not reconnect its windows\" message \"Your terminal processes are still held by the recovery process. Close any broken chda windows, then choose Retry.\" buttons {\"Retry\"} default button \"Retry\" as critical"
        ]).status();
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{
        os::unix::fs::DirBuilderExt,
        process::{Command, Stdio},
    };
    fn bundle(exe: &Path) -> io::Result<&Path> {
        let app = exe
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .filter(|p| p.extension().is_some_and(|s| s == "app"))
            .ok_or_else(|| io::Error::other("Use the signed macOS app to install updates."))?;
        Ok(app)
    }
    fn run(command: &mut Command) -> io::Result<std::process::Output> {
        let out = command.output()?;
        if !out.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            ));
        }
        Ok(out)
    }
    pub fn verify(exe: &Path, recovery: &Path) -> io::Result<()> {
        let original = run(Command::new("/usr/bin/codesign")
            .args(["-d", "-r-"])
            .arg(bundle(recovery)?))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&original.stdout),
            String::from_utf8_lossy(&original.stderr)
        );
        let requirement = text
            .lines()
            .find_map(|s| s.strip_prefix("designated => "))
            .filter(|s| s.contains("anchor apple"))
            .ok_or_else(|| io::Error::other("Updates require an Apple Developer ID signed app."))?;
        run(Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict", "-R"])
            .arg(format!("={requirement}"))
            .arg(bundle(exe)?))?;
        Ok(())
    }
    pub fn start(version: &str, target: Option<PathBuf>) -> io::Result<UpdateJob> {
        let target_exe = target.unwrap_or(std::env::current_exe()?).canonicalize()?;
        let source = bundle(&target_exe)?;
        verify(&target_exe, &target_exe)?;
        let public_key = run(Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print :SUPublicEDKey"])
            .arg(source.join("Contents/Info.plist")))?;
        if String::from_utf8_lossy(&public_key.stdout)
            .trim()
            .is_empty()
        {
            return Err(io::Error::other(
                "This app has no update signing key. Install a configured release first.",
            ));
        }
        let home = chda_core::agents::data_dir()
            .ok_or_else(|| io::Error::other("No application data directory"))?
            .join("updates");
        std::fs::create_dir_all(&home)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = home.join(format!("{}-{nonce}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let backup = directory.join("Recovery.app");
        run(Command::new("/usr/bin/ditto").arg(source).arg(&backup))?;
        let recovery_exe = backup.join("Contents/MacOS/chda");
        verify(&recovery_exe, &target_exe)?;
        std::fs::write(directory.join("progress.jsonl"), b"")?;
        let helper = backup.join("Contents/Helpers/chda-updater");
        let mut child = Command::new(&helper)
            .arg(source)
            .arg(&directory)
            .arg(version)
            .arg(std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let exit_file = directory.join("helper-exited");
        std::thread::spawn(move || {
            let result = child.wait();
            let _ = std::fs::write(exit_file, format!("{result:?}"));
        });
        Ok(UpdateJob {
            handoff: UpdateHandoff {
                directory,
                target_exe,
                recovery_exe,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_ignores_partial_writes_and_bounds_history_reads() {
        let directory = std::env::temp_dir().join(format!("chda-progress-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let job = UpdateJob {
            handoff: chda_core::handoff::UpdateHandoff {
                directory: directory.clone(),
                target_exe: PathBuf::new(),
                recovery_exe: PathBuf::new(),
            },
        };
        let mut log = "x".repeat(128 * 1024);
        log.push_str("\n{\"phase\":\"verifying\"}\n{\"phase\":\"rea");
        std::fs::write(directory.join("progress.jsonl"), log).unwrap();
        assert_eq!(job.progress(), Some(UpdateProgress::Verifying));
        std::fs::write(directory.join("progress.jsonl"), "").unwrap();
        std::fs::write(directory.join("helper-exited"), "1").unwrap();
        assert!(matches!(
            job.progress(),
            Some(UpdateProgress::Failed { .. })
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
