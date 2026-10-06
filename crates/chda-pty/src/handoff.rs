//! Handing running terminals to a successor process (live upgrade).
//!
//! The current process starts the successor with every detached PTY master
//! and one end of a socket pair inherited. Over the socket it sends a header
//! line naming those descriptors and their children, then the caller's
//! state bytes, and waits for the successor to answer `ready`. Until then
//! it still holds the PTYs and can adopt them back.
//!
//! Unix only; elsewhere every call fails with `Unsupported`.

use std::io;
use std::path::Path;
use std::time::Duration;

use crate::DetachedPty;

/// Command-line flag that starts a successor; followed by the socket's
/// descriptor number.
pub const ADOPT_FLAG: &str = "--adopt";

const VERSION: &str = "chda-handoff 1";

/// A started successor that has not confirmed yet.
pub struct Successor {
    #[cfg(unix)]
    child: std::process::Child,
    #[cfg(unix)]
    stream: std::os::unix::net::UnixStream,
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
        let (ours, theirs) = UnixStream::pair()?;
        let mut inherit: Vec<i32> = ptys.iter().map(|p| p.fd.as_raw_fd()).collect();
        inherit.push(theirs.as_raw_fd());
        let mut cmd = Command::new(exe);
        cmd.arg(ADOPT_FLAG)
            .arg(theirs.as_raw_fd().to_string())
            .stdin(Stdio::null());
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
        let mut header = String::from(VERSION);
        for p in ptys {
            header.push_str(&format!(" {}:{}", p.fd.as_raw_fd(), p.pid));
        }
        header.push('\n');
        let mut stream = ours;
        stream.write_all(header.as_bytes())?;
        stream.write_all(state)?;
        stream.shutdown(std::net::Shutdown::Write)?;
        Ok(Successor { child, stream })
    }

    impl Successor {
        /// Wait until the successor took over. An error means it exited,
        /// failed or did not answer in time; the PTYs are still ours.
        pub fn wait_ready(mut self, timeout: Duration) -> io::Result<()> {
            self.stream.set_read_timeout(Some(timeout))?;
            let mut line = String::new();
            let read = BufReader::new(&self.stream).read_line(&mut line);
            if read.is_ok() && line.trim_end() == "ready" {
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

    /// In the successor: read what the predecessor sent over `fd`.
    pub fn receive(fd: i32) -> io::Result<Inherited> {
        // SAFETY: the predecessor passed this descriptor for us to own.
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        // SAFETY: setting FD_CLOEXEC on a descriptor we own.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        let mut reader = BufReader::new(&stream);
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let mut parts = header.trim_end().split(' ');
        let version: Vec<&str> = parts.by_ref().take(2).collect();
        if version.join(" ") != VERSION {
            return Err(io::Error::other(format!(
                "unknown handoff header {header:?}"
            )));
        }
        let mut ptys = Vec::new();
        for part in parts {
            let (fd, pid) = part
                .split_once(':')
                .and_then(|(f, p)| Some((f.parse::<i32>().ok()?, p.parse::<u32>().ok()?)))
                .ok_or_else(|| io::Error::other(format!("bad handoff entry {part:?}")))?;
            // SAFETY: the predecessor kept this descriptor open for us.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            // SAFETY: setting FD_CLOEXEC on a descriptor we own.
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
            ptys.push(DetachedPty { fd, pid });
        }
        let mut state = Vec::new();
        reader.read_to_end(&mut state)?;
        Ok(Inherited {
            ptys,
            state,
            reply: Reply { stream },
        })
    }

    impl Reply {
        /// Tell the predecessor it can exit now.
        pub fn ready(mut self) -> io::Result<()> {
            self.stream.write_all(b"ready\n")
        }

        /// Tell the predecessor to keep its terminals.
        pub fn fail(mut self, reason: &str) -> io::Result<()> {
            let reason = reason.replace('\n', " ");
            self.stream.write_all(format!("fail {reason}\n").as_bytes())
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
    pub fn wait_ready(self, _: Duration) -> io::Result<()> {
        Err(crate::unsupported())
    }
}

#[cfg(not(unix))]
impl Reply {
    pub fn ready(self) -> io::Result<()> {
        Err(crate::unsupported())
    }
    pub fn fail(self, _: &str) -> io::Result<()> {
        Err(crate::unsupported())
    }
}

/// Whether this platform can hand terminals to a successor.
pub const SUPPORTED: bool = cfg!(unix);
