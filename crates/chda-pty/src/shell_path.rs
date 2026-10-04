//! Read the PATH configured by a login shell without changing process globals.

use std::io;
use std::path::Path;
use std::time::Duration;

/// Capture PATH after the user's login and interactive startup files run.
/// Startup output is ignored. A deadline bounds broken or blocking dotfiles.
pub fn shell_path(
    shell: &Path,
    home: &Path,
    env: &[(&str, &str)],
    timeout: Duration,
) -> io::Result<String> {
    #[cfg(unix)]
    {
        capture(shell, home, env, timeout)
    }
    #[cfg(not(unix))]
    {
        let _ = (shell, home, env, timeout);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "shell PATH capture requires Unix",
        ))
    }
}

#[cfg(unix)]
fn capture(
    shell: &Path,
    home: &Path,
    env: &[(&str, &str)],
    timeout: Duration,
) -> io::Result<String> {
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::os::unix::{net::UnixStream, process::CommandExt};
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    // Keep the child unreaped until its private process group is terminated,
    // so the PID cannot be reused before cleanup, even on successful capture.
    struct Probe(Child);
    impl Drop for Probe {
        fn drop(&mut self) {
            // SAFETY: process_group(0) makes this child's PID its group ID.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }
    let (mut reader, writer) = UnixStream::pair()?;
    let mut command = Command::new(shell);
    command
        .args([
            "-l",
            "-i",
            "-c",
            "exec /usr/bin/printf '\\000CHDA_PATH\\000%s\\000' \"$PATH\"",
        ])
        .current_dir(home)
        .env("TERM", "xterm-256color")
        .env("HOME", home)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .process_group(0);
    let _probe = Probe(command.spawn()?);
    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    let mut bytes = [0; 4096];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "shell PATH capture timed out")
            })?;
        reader.set_read_timeout(Some(remaining))?;
        let count = reader.read(&mut bytes).map_err(|e| {
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) {
                io::Error::new(io::ErrorKind::TimedOut, "shell PATH capture timed out")
            } else {
                e
            }
        })?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "shell did not report PATH",
            ));
        }
        output.extend_from_slice(&bytes[..count]);
        if let Some(path) = parse_path(&output) {
            return path;
        }
        if output.len() > 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too much shell startup output",
            ));
        }
    }
}

#[cfg(unix)]
fn parse_path(output: &[u8]) -> Option<io::Result<String>> {
    const MARKER: &[u8] = b"\0CHDA_PATH\0";
    let start = output.windows(MARKER.len()).position(|w| w == MARKER)? + MARKER.len();
    let tail = &output[start..];
    let end = tail.iter().position(|b| *b == 0)?;
    Some(
        String::from_utf8(tail[..end].to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "shell PATH is not UTF-8"))
            .and_then(|path| {
                if path.is_empty() {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "shell PATH is empty",
                    ))
                } else {
                    Ok(path)
                }
            }),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn path_capture_ignores_startup_noise_and_preserves_spaces() {
        assert!(parse_path(b"hello").is_none());
        assert!(parse_path(b"hello\0CHDA_PATH\0/bin").is_none());
        assert_eq!(
            parse_path(b"hello\0CHDA_PATH\0/a b:/bin\0bye")
                .unwrap()
                .unwrap(),
            "/a b:/bin"
        );
        assert!(parse_path(b"\0CHDA_PATH\0\0").unwrap().is_err());
    }

    #[test]
    fn blocked_startup_is_terminated_within_the_deadline() {
        let home = std::env::temp_dir().join(format!("chda-path-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let shell = home.join("shell");
        std::fs::write(&shell, "#!/bin/sh\nexec /bin/sleep 10\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = std::time::Instant::now();
        let err = shell_path(&shell, &home, &[], Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn real_shells_read_login_and_interactive_path_configuration() {
        let home = std::env::temp_dir().join(format!("chda-shell-path-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".config/fish")).unwrap();
        let extra = home.join("tools with spaces");
        let profile = format!(
            "export PATH='{}':$PATH\nprintf 'startup noise\\n'\n",
            extra.display()
        );
        std::fs::write(home.join(".zprofile"), &profile).unwrap();
        std::fs::write(home.join(".zshrc"), "export PATH=$PATH:/interactive-only\n").unwrap();
        std::fs::write(
            home.join(".bash_profile"),
            format!("{profile}. \"$HOME/.bashrc\"\n"),
        )
        .unwrap();
        std::fs::write(
            home.join(".bashrc"),
            "export PATH=$PATH:/interactive-only\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".config/fish/config.fish"),
            format!(
                "set -gx PATH '{}' $PATH /interactive-only\nprintf 'startup noise\\n'\n",
                extra.display()
            ),
        )
        .unwrap();
        let home_str = home.to_str().unwrap();
        let config = home.join(".config");
        for shell in [
            "/bin/zsh",
            "/bin/bash",
            "/opt/homebrew/bin/fish",
            "/usr/bin/fish",
        ] {
            if !Path::new(shell).is_file() {
                continue;
            }
            let path = shell_path(
                Path::new(shell),
                &home,
                &[
                    ("PATH", "/usr/bin:/bin"),
                    ("ZDOTDIR", home_str),
                    ("XDG_CONFIG_HOME", config.to_str().unwrap()),
                ],
                Duration::from_secs(2),
            )
            .unwrap();
            let dirs: Vec<_> = std::env::split_paths(&path).collect();
            assert_eq!(dirs.first(), Some(&extra), "{shell}");
            assert_eq!(
                dirs.last(),
                Some(&std::path::PathBuf::from("/interactive-only")),
                "{shell}"
            );
            assert!(!path.contains("startup noise"));
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}
