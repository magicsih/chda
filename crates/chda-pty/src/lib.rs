//! PTY creation, reading, writing and resizing. Thin layer over `portable-pty`.
//!
//! Platform-specific code for the PTY lives here.

pub mod handoff;
mod shell_path;
pub use shell_path::shell_path;

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
///
/// A PTY is either spawned here or adopted from an earlier chda process
/// ([`Pty::detach`], [`Pty::adopt`]); an adopted child belongs to another
/// parent, so its exit status is unknown.
pub struct Pty {
    master: Master,
    writer: Box<dyn Write + Send>,
    child: ChildProcess,
}

enum Master {
    Spawned(Box<dyn MasterPty + Send>),
    #[cfg(unix)]
    Adopted(std::os::fd::OwnedFd),
}

enum ChildProcess {
    Spawned(Box<dyn Child + Send + Sync>),
    #[cfg(unix)]
    Adopted(u32),
}

impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pty").field("pid", &self.pid()).finish()
    }
}

/// Variables chda sets for its panes that this process inherited, e.g. when
/// chda (or a test) starts from a chda pane. A new pane must not report to
/// that other pane or use its agent settings, so they are dropped and only
/// the caller's `env` applies.
fn inherited_chda_vars(
    vars: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<std::ffi::OsString> {
    vars.map(|(key, _)| key)
        .filter(|key| key.to_string_lossy().starts_with("CHDA_"))
        .collect()
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
        for key in inherited_chda_vars(std::env::vars_os()) {
            cmd.env_remove(key);
        }
        for (key, value) in opts.env {
            cmd.env(OsStr::new(key), OsStr::new(value));
        }

        let child = slave.spawn_command(cmd).map_err(io::Error::other)?;
        // The slave end is only needed to spawn; the child holds its own copy.
        drop(slave);
        // portable-pty's Unix writer types end-of-file into the PTY when it
        // is dropped, which would end the shell of a detached PTY. A plain
        // descriptor does not; closing the PTY still hangs up the child.
        #[cfg(unix)]
        let writer: Box<dyn Write + Send> = {
            let raw = master
                .as_raw_fd()
                .ok_or_else(|| io::Error::other("the PTY has no descriptor"))?;
            // SAFETY: `raw` stays open while `master` lives.
            let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(raw) }.try_clone_to_owned()?;
            Box::new(std::fs::File::from(fd))
        };
        #[cfg(not(unix))]
        let writer = master.take_writer().map_err(io::Error::other)?;

        Ok(Self {
            master: Master::Spawned(master),
            writer,
            child: ChildProcess::Spawned(child),
        })
    }

    /// Take over a PTY another chda process detached. The child keeps
    /// running; it never sees the PTY close.
    pub fn adopt(detached: DetachedPty) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let writer = std::fs::File::from(detached.fd.try_clone()?);
            Ok(Self {
                master: Master::Adopted(detached.fd),
                writer: Box::new(writer),
                child: ChildProcess::Adopted(detached.pid),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = detached;
            Err(unsupported())
        }
    }

    /// Let go of the PTY without ending the child, so another process can
    /// [`Pty::adopt`] it. Stop the reader first: bytes it reads after this
    /// are lost to the next owner.
    pub fn detach(self) -> io::Result<DetachedPty> {
        self.detached_handle()
    }

    /// Duplicate the master without relinquishing the current owner.
    pub fn detached_handle(&self) -> io::Result<DetachedPty> {
        #[cfg(unix)]
        {
            let pid = self
                .pid()
                .ok_or_else(|| io::Error::other("the child has no process id"))?;
            let fd = match &self.master {
                Master::Spawned(master) => {
                    let raw = master
                        .as_raw_fd()
                        .ok_or_else(|| io::Error::other("the PTY has no descriptor"))?;
                    // SAFETY: `raw` stays open while `master` lives.
                    unsafe { std::os::fd::BorrowedFd::borrow_raw(raw) }.try_clone_to_owned()?
                }
                Master::Adopted(fd) => fd.try_clone()?,
            };
            Ok(DetachedPty { fd, pid })
        }
        #[cfg(not(unix))]
        {
            let _ = self;
            Err(unsupported())
        }
    }

    /// A reader for the child's output. Call once and move it to a reader
    /// thread; its [`ReaderStop`] ends the reads without losing bytes.
    pub fn reader(&self) -> io::Result<PtyReader> {
        let inner: Box<dyn Read + Send> = match &self.master {
            Master::Spawned(master) => master.try_clone_reader().map_err(io::Error::other)?,
            #[cfg(unix)]
            Master::Adopted(fd) => Box::new(std::fs::File::from(fd.try_clone()?)),
        };
        PtyReader::new(inner, self.raw_fd())
    }

    /// Send bytes to the child.
    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Tell the kernel and the child that the terminal size changed.
    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        match &self.master {
            Master::Spawned(master) => master.resize(size.into()).map_err(io::Error::other),
            #[cfg(unix)]
            Master::Adopted(fd) => {
                use std::os::fd::AsRawFd;
                let ws = libc::winsize {
                    ws_row: size.rows,
                    ws_col: size.cols,
                    ws_xpixel: size.pixel_width,
                    ws_ypixel: size.pixel_height,
                };
                // SAFETY: TIOCSWINSZ reads one winsize from the pointer.
                if unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSWINSZ, &ws) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }
        }
    }

    /// Process id of the child, if known.
    pub fn pid(&self) -> Option<u32> {
        match &self.child {
            ChildProcess::Spawned(child) => child.process_id(),
            #[cfg(unix)]
            ChildProcess::Adopted(pid) => Some(*pid),
        }
    }

    /// Non-blocking check whether the child has exited. `Some(None)`: it
    /// exited, but it was adopted, so its status is unknown.
    pub fn try_wait(&mut self) -> io::Result<Option<Option<ExitStatus>>> {
        match &mut self.child {
            ChildProcess::Spawned(child) => Ok(child.try_wait()?.map(|s| Some(s.into()))),
            #[cfg(unix)]
            ChildProcess::Adopted(pid) => Ok((!alive(*pid)).then_some(None)),
        }
    }

    /// Block until the child exits; `None` for an adopted child.
    pub fn wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.child {
            ChildProcess::Spawned(child) => Ok(Some(child.wait()?.into())),
            #[cfg(unix)]
            ChildProcess::Adopted(pid) => {
                while alive(*pid) {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Ok(None)
            }
        }
    }

    /// Terminate the child: SIGHUP, as closing a terminal does.
    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.child {
            ChildProcess::Spawned(child) => child.kill(),
            #[cfg(unix)]
            ChildProcess::Adopted(pid) => {
                // SAFETY: plain signal delivery.
                if unsafe { libc::kill(*pid as libc::pid_t, libc::SIGHUP) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }
        }
    }

    fn raw_fd(&self) -> Option<i32> {
        match &self.master {
            Master::Spawned(master) => {
                #[cfg(unix)]
                {
                    master.as_raw_fd()
                }
                #[cfg(not(unix))]
                {
                    let _ = master;
                    None
                }
            }
            #[cfg(unix)]
            Master::Adopted(fd) => {
                use std::os::fd::AsRawFd;
                Some(fd.as_raw_fd())
            }
        }
    }

    /// Working directory of the process in the foreground of this PTY, for
    /// shells that do not report it through OSC 7. macOS only for now.
    pub fn foreground_cwd(&self) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let fd = self.raw_fd()?;
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

/// Whether a process that is not our child still runs. Reaps it when it is
/// our child after all (a PTY detached and adopted in the same process).
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let pid = pid as libc::pid_t;
    let mut status = 0;
    // SAFETY: WNOHANG never blocks; ECHILD just means another parent.
    if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == pid {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(not(unix))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "handing terminals to another process needs Unix PTYs",
    )
}

/// A PTY let go by [`Pty::detach`]: its master descriptor and the child's
/// process id, ready for [`Pty::adopt`] here or in a successor process.
#[derive(Debug)]
pub struct DetachedPty {
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
    pid: u32,
}

impl DetachedPty {
    /// Keep a recovery descriptor until the successor is fully prepared.
    pub fn try_clone(&self) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                fd: self.fd.try_clone()?,
                pid: self.pid,
            })
        }
        #[cfg(not(unix))]
        {
            Err(unsupported())
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
}

/// Output of a [`Pty`]. On Unix, [`ReaderStop::stop`] makes the next read
/// return end of file without taking bytes from the PTY.
pub struct PtyReader {
    inner: Box<dyn Read + Send>,
    #[cfg(unix)]
    poll: Option<(i32, std::os::fd::OwnedFd)>,
    stop: ReaderStop,
}

/// Ends a [`PtyReader`]'s reads from another thread.
#[derive(Clone, Debug)]
pub struct ReaderStop {
    #[cfg(unix)]
    pipe: Option<std::sync::Arc<std::os::fd::OwnedFd>>,
}

impl ReaderStop {
    /// Wake the reader and make it report end of file. Unix only; elsewhere
    /// the reader runs until the child exits.
    pub fn stop(&self) {
        #[cfg(unix)]
        if let Some(pipe) = &self.pipe {
            use std::os::fd::AsRawFd;
            // SAFETY: one byte from a valid buffer.
            unsafe { libc::write(pipe.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
        }
    }
}

impl PtyReader {
    fn new(inner: Box<dyn Read + Send>, fd: Option<i32>) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::fd::{FromRawFd, OwnedFd};
            let Some(fd) = fd else {
                return Ok(Self {
                    inner,
                    poll: None,
                    stop: ReaderStop { pipe: None },
                });
            };
            let mut ends = [0; 2];
            // SAFETY: pipe fills two descriptors we then own.
            if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let (read, write) =
                unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
            for end in ends {
                // SAFETY: setting FD_CLOEXEC on descriptors we own.
                unsafe { libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC) };
            }
            Ok(Self {
                inner,
                poll: Some((fd, read)),
                stop: ReaderStop {
                    pipe: Some(std::sync::Arc::new(write)),
                },
            })
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            Ok(Self {
                inner,
                stop: ReaderStop {},
            })
        }
    }

    pub fn stopper(&self) -> ReaderStop {
        self.stop.clone()
    }
}

impl Read for PtyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        if let Some((fd, stop)) = &self.poll {
            use std::os::fd::AsRawFd;
            let mut fds = [
                libc::pollfd {
                    fd: *fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: stop.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            loop {
                // SAFETY: two valid pollfd entries.
                let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
                if n < 0 {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                break;
            }
            if fds[1].revents != 0 {
                return Ok(0);
            }
        }
        self.inner.read(buf)
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
        let status = pty.wait().unwrap().unwrap();
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
        assert!(pty.wait().unwrap().unwrap().success());
        drain.join().unwrap();
    }

    #[test]
    fn inherited_chda_variables_are_dropped() {
        let vars = [
            ("CHDA_PANE_ID", "31"),
            ("CHDA_CODEX_NOTIFY_CONFIG", "notify=[...]"),
            ("PATH", "/bin"),
            ("MY_CHDA_NOTE", "kept"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()));
        assert_eq!(
            inherited_chda_vars(vars),
            ["CHDA_PANE_ID", "CHDA_CODEX_NOTIFY_CONFIG"]
        );
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
        assert!(pty.wait().unwrap().unwrap().success());
    }

    /// A detached PTY adopted again keeps its child running, and a stopped
    /// reader leaves unread output for the next owner.
    #[test]
    #[cfg(unix)]
    fn detach_and_adopt_keep_the_child_and_its_output() {
        let pty = Pty::spawn(
            size(),
            SpawnOptions {
                command: Some(&[
                    "/bin/sh",
                    "-c",
                    "read a; echo first:$a; read b; echo second:$b",
                ]),
                ..Default::default()
            },
        )
        .unwrap();
        let reader = pty.reader().unwrap();
        let stop = reader.stopper();
        let thread = std::thread::spawn(move || {
            let mut reader = reader;
            let mut out = Vec::new();
            let _ = reader.read_to_end(&mut out);
            out
        });
        stop.stop();
        assert!(
            thread.join().unwrap().is_empty(),
            "the stopped reader read nothing"
        );
        let pid = pty.pid().unwrap();

        let mut pty = Pty::adopt(pty.detach().unwrap()).unwrap();
        assert_eq!(pty.pid(), Some(pid));
        pty.resize(PtySize {
            cols: 90,
            rows: 20,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
        let mut reader = pty.reader().unwrap();
        pty.write_all(b"one\ntwo\n").unwrap();
        let mut out = Vec::new();
        let _ = reader.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("first:one") && text.contains("second:two"),
            "{text:?}"
        );
        assert_eq!(
            pty.wait().unwrap(),
            None,
            "an adopted child's status is unknown"
        );
    }

    #[test]
    #[cfg(unix)]
    fn handoff_receive_reads_descriptors_and_state() {
        use std::io::Write;
        use std::os::fd::{AsRawFd, IntoRawFd};
        use std::os::unix::net::UnixStream;
        let pty = Pty::spawn(
            size(),
            SpawnOptions {
                command: Some(&["/bin/sh", "-c", "read x; echo adopted:$x"]),
                ..Default::default()
            },
        )
        .unwrap();
        let detached = pty.detach().unwrap();
        let pid = detached.pid();
        let (mut ours, theirs) = UnixStream::pair().unwrap();
        let header = format!("chda-handoff 2 11 {}:{pid}\n", detached.fd.as_raw_fd());
        let _ = detached.fd.into_raw_fd();
        ours.write_all(header.as_bytes()).unwrap();
        ours.write_all(b"{\"state\":1}").unwrap();

        let inherited = handoff::receive(theirs.into_raw_fd()).unwrap();
        assert_eq!(inherited.state, b"{\"state\":1}");
        assert_eq!(inherited.ptys.len(), 1);
        let reply =
            std::thread::spawn(move || inherited.reply.prepared(std::time::Duration::from_secs(2)));
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(&ours), &mut line).unwrap();
        assert_eq!(line, "prepared\n");
        ours.write_all(b"commit\n").unwrap();
        reply.join().unwrap().unwrap();

        let mut adopted = Pty::adopt(inherited.ptys.into_iter().next().unwrap()).unwrap();
        let mut reader = adopted.reader().unwrap();
        adopted.write_all(b"yes\n").unwrap();
        let mut out = Vec::new();
        let _ = reader.read_to_end(&mut out);
        assert!(String::from_utf8_lossy(&out).contains("adopted:yes"));
    }
}
