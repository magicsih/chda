//! Handing running terminals to a successor process (live upgrade).
//!
//! The current process starts the successor with every detached PTY master
//! and one end of a socket pair inherited. Over the socket it sends a header
//! line naming those descriptors and their children, then the caller's
//! state bytes, and waits for the successor to answer `prepared`. Until commit,
//! it still holds the PTYs and can adopt them back. Preparing successors must
//! neither consume terminal output nor signal the shell on cleanup.
//!
//! Unix only; elsewhere every call fails with `Unsupported`.

use std::io;
use std::path::Path;
use std::time::Duration;

use crate::DetachedPty;

/// Command-line flag that starts a successor; followed by the socket's
/// descriptor number.
pub const ADOPT_FLAG: &str = "--adopt";

/// A started successor that has not confirmed yet.
pub struct Successor {
    #[cfg(unix)]
    child: std::process::Child,
    #[cfg(unix)]
    stream: std::os::unix::net::UnixStream,
    #[cfg(unix)]
    committed: bool,
    #[cfg(unix)]
    prepared: bool,
}

/// What a successor receives.
pub struct Inherited {
    pub ptys: Vec<DetachedPty>,
    pub state: Vec<u8>,
    pub reply: Reply,
}

/// The successor's answer channel to its predecessor.
pub struct Reply {
    #[cfg(unix)]
    stream: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
mod unix {
    use super::*;
    const VERSION: &str = "chda-handoff 2";
    const MAX_STATE: usize = 512 * 1024 * 1024;
    const MAX_HEADER: u64 = 1024 * 1024;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    /// Start `exe` as the successor and send it `ptys` and `state`. The
    /// PTYs stay open here as well until this process exits.
    pub fn spawn_successor(
        exe: &Path,
        ptys: &[DetachedPty],
        state: &[u8],
    ) -> io::Result<Successor> {
        spawn_command(Command::new(exe), ptys, state, |cmd, fd| {
            cmd.arg(ADOPT_FLAG).arg(fd.to_string());
        })
    }

    fn spawn_command(
        mut cmd: Command,
        ptys: &[DetachedPty],
        state: &[u8],
        arguments: impl FnOnce(&mut Command, i32),
    ) -> io::Result<Successor> {
        if state.len() > MAX_STATE {
            return Err(io::Error::other("handoff state exceeds size limit"));
        }
        let (ours, theirs) = UnixStream::pair()?;
        ours.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut inherit: Vec<i32> = ptys.iter().map(|p| p.fd.as_raw_fd()).collect();
        inherit.push(theirs.as_raw_fd());
        arguments(&mut cmd, theirs.as_raw_fd());
        cmd.stdin(Stdio::null());
        // SAFETY: only fcntl, which is async-signal-safe, runs after fork.
        unsafe {
            cmd.pre_exec(move || {
                for &fd in &inherit {
                    if libc::fcntl(fd, libc::F_SETFD, 0) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        drop(theirs);
        let mut header = format!("{VERSION} {}", state.len());
        for p in ptys {
            header.push_str(&format!(" {}:{}", p.fd.as_raw_fd(), p.pid));
        }
        header.push('\n');
        let mut successor = Successor {
            child,
            stream: ours,
            committed: false,
            prepared: false,
        };
        successor.stream.write_all(header.as_bytes())?;
        successor.stream.write_all(state)?;
        Ok(successor)
    }

    impl Successor {
        /// Wait for preparation without transferring ownership. An error means
        /// it exited, failed or timed out; the PTYs are still ours.
        pub fn wait_prepared(&mut self, timeout: Duration) -> io::Result<()> {
            self.stream.set_read_timeout(Some(timeout))?;
            let mut line = String::new();
            let read = BufReader::new(&self.stream)
                .take(MAX_HEADER)
                .read_line(&mut line);
            if read.is_ok() && line.trim_end() == "prepared" {
                self.prepared = true;
                return Ok(());
            }
            let reason = match read {
                Err(e) => e.to_string(),
                Ok(0) => "it exited before taking over".to_owned(),
                Ok(_) => line.trim().trim_start_matches("fail ").to_owned(),
            };
            // A successor that failed must not linger as a second app.
            let _ = self.child.kill();
            let _ = self.child.wait();
            Err(io::Error::other(format!(
                "the new version did not start: {reason}"
            )))
        }
    }

    impl Successor {
        /// Grant ownership only after every window and terminal is prepared.
        /// After a successful write the caller must not resume reading its PTYs.
        pub fn commit(mut self) -> io::Result<()> {
            if !self.prepared {
                return Err(io::Error::other("successor has not prepared"));
            }
            self.stream.write_all(b"commit\n")?;
            self.committed = true;
            Ok(())
        }
    }

    impl Drop for Successor {
        fn drop(&mut self) {
            if !self.committed {
                // SIGKILL does not run a failed GUI's terminal destructors.
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    /// In the successor: read what the predecessor sent over `fd`.
    pub fn receive(fd: i32) -> io::Result<Inherited> {
        if fd < 3 || unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
            return Err(io::Error::other("invalid handoff socket descriptor"));
        }
        // SAFETY: the predecessor passed this descriptor for us to own.
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        // SAFETY: setting FD_CLOEXEC on a descriptor we own.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let mut reader = BufReader::new(&stream);
        let mut header = String::new();
        (&mut reader).take(MAX_HEADER).read_line(&mut header)?;
        if !header.ends_with('\n') {
            return Err(io::Error::other("handoff header too long"));
        }
        let mut parts = header.trim_end().split(' ');
        let version: Vec<&str> = parts.by_ref().take(2).collect();
        if version.join(" ") != VERSION {
            return Err(io::Error::other(format!(
                "unknown handoff header {header:?}"
            )));
        }
        let state_len = parts
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n <= MAX_STATE)
            .ok_or_else(|| io::Error::other("invalid handoff state length"))?;
        let mut descriptors = std::collections::HashSet::new();
        let mut ptys = Vec::new();
        for part in parts {
            let (fd, pid) = part
                .split_once(':')
                .and_then(|(f, p)| Some((f.parse::<i32>().ok()?, p.parse::<u32>().ok()?)))
                .ok_or_else(|| io::Error::other(format!("bad handoff entry {part:?}")))?;
            if fd < 3
                || fd == stream.as_raw_fd()
                || pid == 0
                || pid > i32::MAX as u32
                || !descriptors.insert(fd)
            {
                return Err(io::Error::other("invalid or repeated handoff descriptor"));
            }
            // SAFETY: validate the descriptor before taking ownership.
            if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 || unsafe { libc::isatty(fd) } != 1 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the predecessor kept this descriptor open for us.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            // SAFETY: setting FD_CLOEXEC on a descriptor we own.
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
            ptys.push(DetachedPty { fd, pid });
        }
        let mut state = vec![0; state_len];
        reader.read_exact(&mut state)?;
        Ok(Inherited {
            ptys,
            state,
            reply: Reply { stream },
        })
    }

    impl Reply {
        /// Report that every resource is prepared, then wait for ownership.
        /// On error, release provisional resources without signalling shells.
        pub fn prepared(mut self, timeout: Duration) -> io::Result<()> {
            self.stream.set_read_timeout(Some(timeout))?;
            self.stream.write_all(b"prepared\n")?;
            let mut command = [0; 7];
            self.stream.read_exact(&mut command)?;
            if command != *b"commit\n" {
                return Err(io::Error::other("predecessor did not commit ownership"));
            }
            Ok(())
        }

        /// Tell the predecessor to keep its terminals.
        pub fn fail(mut self, reason: &str) -> io::Result<()> {
            let reason = reason.replace('\n', " ");
            self.stream.write_all(format!("fail {reason}\n").as_bytes())
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn successor_fixture() {
            let Ok(fd) = std::env::var("CHDA_HANDOFF_TEST_FD") else {
                return;
            };
            let inherited = receive(fd.parse().unwrap()).unwrap();
            assert_eq!(inherited.state, b"screen and layout");
            if std::env::var_os("CHDA_HANDOFF_TEST_FAIL").is_some() {
                inherited.reply.fail("window creation failed").unwrap();
                return;
            }
            inherited.reply.prepared(Duration::from_secs(5)).unwrap();
            let mut pty = crate::Pty::adopt(inherited.ptys.into_iter().next().unwrap()).unwrap();
            pty.write_all(b"successor\n").unwrap();
            let mut reader = pty.reader().unwrap();
            let mut output = String::new();
            reader.read_to_string(&mut output).unwrap_or_default();
            assert!(output.contains("received:successor"), "{output}");
        }

        fn fixture(pty: &DetachedPty, fail: bool) -> Successor {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args([
                "--exact",
                "handoff::unix::tests::successor_fixture",
                "--nocapture",
            ]);
            if fail {
                command.env("CHDA_HANDOFF_TEST_FAIL", "1");
            }
            spawn_command(
                command,
                std::slice::from_ref(pty),
                b"screen and layout",
                |cmd, fd| {
                    cmd.env("CHDA_HANDOFF_TEST_FD", fd.to_string());
                },
            )
            .unwrap()
        }

        #[test]
        fn failed_process_preserves_shell_then_a_second_process_commits() {
            let pty = crate::Pty::spawn(
                crate::PtySize {
                    cols: 80,
                    rows: 24,
                    pixel_width: 0,
                    pixel_height: 0,
                },
                crate::SpawnOptions {
                    command: Some(&["/bin/sh", "-c", "read x; echo received:$x"]),
                    ..Default::default()
                },
            )
            .unwrap()
            .detach()
            .unwrap();
            let mut failed = fixture(&pty, true);
            let error = failed.wait_prepared(Duration::from_secs(5)).unwrap_err();
            assert!(error.to_string().contains("window creation failed"));
            drop(failed);
            let mut successor = fixture(&pty, false);
            successor.wait_prepared(Duration::from_secs(5)).unwrap();
            // Merely preparing must not consume output or terminate the child.
            let mut recovery = crate::Pty::adopt(pty.try_clone().unwrap()).unwrap();
            assert!(recovery.try_wait().unwrap().is_none());
            let pid = successor.child.id() as libc::pid_t;
            successor.commit().unwrap();
            let mut status = 0;
            // SAFETY: wait for the test's own successor after the handle is dropped.
            assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
            assert!(libc::WIFEXITED(status));
            assert_eq!(libc::WEXITSTATUS(status), 0);
        }
    }
}

#[cfg(unix)]
pub use unix::{receive, spawn_successor};

#[cfg(not(unix))]
pub fn spawn_successor(_: &Path, _: &[DetachedPty], _: &[u8]) -> io::Result<Successor> {
    Err(crate::unsupported())
}

#[cfg(not(unix))]
pub fn receive(_: i32) -> io::Result<Inherited> {
    Err(crate::unsupported())
}

#[cfg(not(unix))]
impl Successor {
    pub fn commit(self) -> io::Result<()> {
        Err(crate::unsupported())
    }
    pub fn wait_prepared(&mut self, _: Duration) -> io::Result<()> {
        Err(crate::unsupported())
    }
}

#[cfg(not(unix))]
impl Reply {
    pub fn prepared(self, _: Duration) -> io::Result<()> {
        Err(crate::unsupported())
    }
    pub fn fail(self, _: &str) -> io::Result<()> {
        Err(crate::unsupported())
    }
}

/// Whether this platform can hand terminals to a successor.
pub const SUPPORTED: bool = cfg!(unix);
