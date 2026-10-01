//! PTY creation, reading, writing and resizing. Thin layer over `portable-pty`.
//!
//! Platform-specific code for the PTY lives here.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use portable_pty::{Child, CommandBuilder, MasterPty, PtyPair, native_pty_system};

/// Size of the terminal attached to the PTY, in cells and pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl From<PtySize> for portable_pty::PtySize {
    fn from(s: PtySize) -> Self {
        Self {
            rows: s.rows,
            cols: s.cols,
            pixel_width: s.pixel_width,
            pixel_height: s.pixel_height,
        }
    }
}

/// How to start the child process.
#[derive(Clone, Debug, Default)]
pub struct SpawnOptions<'a> {
    /// Program and arguments. `None` runs the user's login shell.
    pub command: Option<&'a [&'a str]>,
    /// Working directory. `None` inherits the parent's.
    pub cwd: Option<&'a Path>,
    /// Extra environment variables layered over the inherited environment.
    pub env: &'a [(&'a str, &'a str)],
}

/// Exit status of the child process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: u32,
    pub signal: Option<String>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == 0 && self.signal.is_none()
    }
}

impl From<portable_pty::ExitStatus> for ExitStatus {
    fn from(s: portable_pty::ExitStatus) -> Self {
        Self {
            code: s.exit_code(),
            signal: s.signal().map(str::to_owned),
        }
    }
}

/// A PTY with a child process attached.
///
/// Reads happen on a reader obtained from [`Pty::reader`], usually on a
/// dedicated thread. Writes and resizes go through this handle. Keep reading
/// for as long as the child lives: the tty holds the child's exit until its
/// output is drained.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pty").field("pid", &self.pid()).finish()
    }
}

impl Pty {
    /// Open a PTY of the given size and spawn the child in it.
    pub fn spawn(size: PtySize, opts: SpawnOptions<'_>) -> io::Result<Self> {
        let PtyPair { master, slave } = native_pty_system()
            .openpty(size.into())
            .map_err(io::Error::other)?;

        let mut cmd = match opts.command {
            Some([program, args @ ..]) => {
                let mut cmd = CommandBuilder::new(program);
                cmd.args(args);
                cmd
            }
            Some([]) | None => CommandBuilder::new_default_prog(),
        };
        if let Some(cwd) = opts.cwd {
            cmd.cwd(cwd);
        }
        for (key, value) in opts.env {
            cmd.env(OsStr::new(key), OsStr::new(value));
        }

        let child = slave.spawn_command(cmd).map_err(io::Error::other)?;
        // The slave end is only needed to spawn; the child holds its own copy.
        drop(slave);
        let writer = master.take_writer().map_err(io::Error::other)?;

        Ok(Self {
            master,
            writer,
            child,
        })
    }

    /// A reader for the child's output. Call once and move it to a reader thread.
    pub fn reader(&self) -> io::Result<Box<dyn Read + Send>> {
        self.master.try_clone_reader().map_err(io::Error::other)
    }

    /// Send bytes to the child.
    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Tell the kernel and the child that the terminal size changed.
    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        self.master.resize(size.into()).map_err(io::Error::other)
    }

    /// Process id of the child, if known.
    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Non-blocking check whether the child has exited.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?.map(Into::into))
    }

    /// Block until the child exits.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        Ok(self.child.wait()?.into())
    }

    /// Terminate the child.
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    /// Working directory of the process in the foreground of this PTY, for
    /// shells that do not report it through OSC 7. macOS only for now.
    pub fn foreground_cwd(&self) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let fd = self.master.as_raw_fd()?;
            let pgrp = unsafe { libc::tcgetpgrp(fd) };
            if pgrp <= 0 {
                return None;
            }
            macos::process_cwd(pgrp)
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CStr;
    use std::path::PathBuf;

    // From <libproc.h>: PROC_PIDVNODEPATHINFO fills a proc_vnodepathinfo whose
    // first member, pvi_cdir, holds the current directory path at this
    // offset. Sizes checked against the SDK headers with clang.
    const PROC_PIDVNODEPATHINFO: libc::c_int = 9;
    const VNODEPATHINFO_SIZE: usize = 2352;
    const CDIR_PATH_OFFSET: usize = 152;

    unsafe extern "C" {
        fn proc_pidinfo(
            pid: libc::c_int,
            flavor: libc::c_int,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }

    pub fn process_cwd(pid: libc::pid_t) -> Option<PathBuf> {
        let mut buf = [0u8; VNODEPATHINFO_SIZE];
        let n = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDVNODEPATHINFO,
                0,
                buf.as_mut_ptr().cast(),
                VNODEPATHINFO_SIZE as libc::c_int,
            )
        };
        if n as usize != VNODEPATHINFO_SIZE {
            return None;
        }
        let path = CStr::from_bytes_until_nul(&buf[CDIR_PATH_OFFSET..]).ok()?;
        let path = path.to_str().ok()?;
        (!path.is_empty()).then(|| PathBuf::from(path))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn size() -> PtySize {
        PtySize {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    #[test]
    fn spawn_echo_and_read_output() {
        let mut pty = Pty::spawn(
            size(),
            SpawnOptions {
                command: Some(&["/bin/sh", "-c", "echo hello-pty; exit 3"]),
                ..Default::default()
            },
        )
        .unwrap();
        let mut reader = pty.reader().unwrap();
        let mut out = Vec::new();
        // The read ends with an error on some platforms once the child
        // exits and the slave side closes; the bytes before that are valid.
        let _ = reader.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("hello-pty"), "output was {text:?}");
        let status = pty.wait().unwrap();
        assert_eq!(status.code, 3);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn foreground_cwd_follows_the_child() {
        let mut pty = Pty::spawn(
            size(),
            SpawnOptions {
                command: Some(&["/bin/sh", "-c", "cd /private/tmp && read x"]),
                ..Default::default()
            },
        )
        .unwrap();
        // The tty blocks the child's exit until its output is drained, so a
        // PTY must always have a reader, even in tests.
        let mut reader = pty.reader().unwrap();
        let drain = std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = reader.read_to_end(&mut sink);
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut cwd = None;
        while std::time::Instant::now() < deadline {
            cwd = pty.foreground_cwd();
            if cwd.as_deref() == Some(Path::new("/private/tmp")) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(cwd.as_deref(), Some(Path::new("/private/tmp")));
        pty.write_all(b"\n").unwrap();
        assert!(pty.wait().unwrap().success());
        drain.join().unwrap();
    }

    #[test]
    fn write_reaches_child_and_resize_is_reported() {
        let mut pty = Pty::spawn(
            size(),
            SpawnOptions {
                command: Some(&["/bin/sh", "-c", "read line; echo got:$line; stty size"]),
                env: &[("CHDA_TEST", "1")],
                ..Default::default()
            },
        )
        .unwrap();
        pty.resize(PtySize {
            cols: 100,
            rows: 30,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
        let mut reader = pty.reader().unwrap();
        pty.write_all(b"ping\n").unwrap();
        let mut out = Vec::new();
        let _ = reader.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("got:ping"), "output was {text:?}");
        assert!(text.contains("30 100"), "output was {text:?}");
        assert!(pty.wait().unwrap().success());
    }
}
