//! Private local transport and detached process startup for the session service.
use std::{io, path::Path, process::Command, time::Duration};

#[cfg(unix)]
mod implementation {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        io::{Read, Write},
        os::unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
        path::PathBuf,
    };

    pub struct Listener {
        listener: UnixListener,
        _lock: File,
        path: PathBuf,
    }
    pub struct Stream(UnixStream);

    pub fn private_directory(path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)?;
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Session directory is not owned by this user",
            ));
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    }

    pub fn token() -> io::Result<String> {
        let mut bytes = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn socket_root() -> PathBuf {
        PathBuf::from(format!("/tmp/chda-session-{}", unsafe { libc::geteuid() }))
    }

    impl Listener {
        /// Acquire ownership before removing a stale socket. The lock is never
        /// unlinked: a waiting starter must lock the same inode as its successor.
        pub fn bind(path: &Path) -> io::Result<Self> {
            private_directory(
                path.parent()
                    .ok_or_else(|| io::Error::other("Missing socket directory"))?,
            )?;
            let lock = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path.with_extension("lock"))?;
            lock.try_lock().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "A session server already owns this profile",
                )
            })?;
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            let listener = UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            Ok(Self {
                listener,
                _lock: lock,
                path: path.to_owned(),
            })
        }
        pub fn accept(&self) -> io::Result<Stream> {
            let (stream, _) = self.listener.accept()?;
            // BSD sockets inherit O_NONBLOCK from their listening socket.
            stream.set_nonblocking(false)?;
            verify_peer(&stream)?;
            Ok(Stream(stream))
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
    fn verify_peer(stream: &UnixStream) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        #[cfg(target_os = "macos")]
        let uid = {
            let (mut uid, mut gid) = (0, 0);
            if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
                return Err(io::Error::last_os_error());
            }
            uid
        };
        #[cfg(target_os = "linux")]
        let uid = {
            let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
            let mut size = std::mem::size_of_val(&cred) as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut cred as *mut libc::ucred).cast(),
                    &mut size,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            cred.uid
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let uid = {
            let _ = stream;
            unsafe { libc::geteuid() }
        };
        if uid != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Session peer is another user",
            ));
        }
        Ok(())
    }
    impl Stream {
        pub fn connect(path: &Path) -> io::Result<Self> {
            let stream = UnixStream::connect(path)?;
            verify_peer(&stream)?;
            Ok(Self(stream))
        }
        pub fn timeout(&self, timeout: Duration) -> io::Result<()> {
            self.0.set_read_timeout(Some(timeout))?;
            self.0.set_write_timeout(Some(timeout))
        }
    }
    impl Read for Stream {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for Stream {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    pub fn spawn_detached(command: &mut Command) -> io::Result<std::process::Child> {
        // No inherited tty or standard handles tie the server to the GUI.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }
}

#[cfg(not(unix))]
mod implementation {
    use super::*;
    use std::io::{Read, Write};
    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "The session service requires Unix local sockets",
        ))
    }
    pub struct Listener;
    pub struct Stream;
    impl Listener {
        pub fn bind(_: &Path) -> io::Result<Self> {
            unsupported()
        }
        pub fn accept(&self) -> io::Result<Stream> {
            unsupported()
        }
    }
    impl Stream {
        pub fn connect(_: &Path) -> io::Result<Self> {
            unsupported()
        }
        pub fn timeout(&self, _: Duration) -> io::Result<()> {
            unsupported()
        }
    }
    impl Read for Stream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            unsupported()
        }
    }
    impl Write for Stream {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            unsupported()
        }
        fn flush(&mut self) -> io::Result<()> {
            unsupported()
        }
    }
    pub fn private_directory(_: &Path) -> io::Result<()> {
        unsupported()
    }
    pub fn token() -> io::Result<String> {
        unsupported()
    }
    pub fn socket_root() -> std::path::PathBuf {
        std::env::temp_dir().join("chda-session")
    }
    pub fn spawn_detached(_: &mut Command) -> io::Result<std::process::Child> {
        unsupported()
    }
}
pub use implementation::*;
