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
    pub struct FileLease {
        _file: File,
    }
    pub fn file_lease(path: &Path, wait: bool) -> io::Result<Option<FileLease>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        if wait {
            file.lock()?;
        } else if file.try_lock().is_err() {
            return Ok(None);
        }
        Ok(Some(FileLease { _file: file }))
    }

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
        /// Transfer one duplicate PTY master. The sender keeps its recovery
        /// descriptor until the service has prepared and the broker commits.
        pub fn send_pty(&mut self, pty: &crate::DetachedPty) -> io::Result<()> {
            use std::os::fd::AsRawFd;
            self.send_fd(pty.fd.as_raw_fd())
        }
        pub fn send_reply(&mut self, reply: &crate::handoff::Reply) -> io::Result<()> {
            use std::os::fd::AsRawFd;
            self.send_fd(reply.stream.as_raw_fd())
        }
        fn send_fd(&mut self, fd: i32) -> io::Result<()> {
            use std::os::fd::AsRawFd;
            let mut marker = [0xa5u8];
            let mut iov = libc::iovec {
                iov_base: marker.as_mut_ptr().cast(),
                iov_len: 1,
            };
            let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as _) } as usize;
            let mut control = vec![0usize; space.div_ceil(std::mem::size_of::<usize>())];
            let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = space as _;
            // Aligned storage covers one header and exactly one descriptor.
            unsafe {
                let cmsg = libc::CMSG_FIRSTHDR(&msg);
                (*cmsg).cmsg_level = libc::SOL_SOCKET;
                (*cmsg).cmsg_type = libc::SCM_RIGHTS;
                (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<i32>() as _) as _;
                std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<i32>(), fd);
                if libc::sendmsg(self.0.as_raw_fd(), &msg, 0) != 1 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
        pub fn receive_pty(&mut self, pid: u32) -> io::Result<crate::DetachedPty> {
            use std::os::fd::AsRawFd;
            let fd = self.receive_fd()?;
            if pid == 0 || pid > i32::MAX as u32 || unsafe { libc::isatty(fd.as_raw_fd()) } != 1 {
                return Err(io::Error::other("Invalid transferred PTY"));
            }
            Ok(crate::DetachedPty { fd, pid })
        }
        pub fn receive_reply(&mut self) -> io::Result<crate::handoff::Reply> {
            let stream = UnixStream::from(self.receive_fd()?);
            verify_peer(&stream)?;
            Ok(crate::handoff::Reply { stream })
        }
        fn receive_fd(&mut self) -> io::Result<std::os::fd::OwnedFd> {
            use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
            let mut marker = [0u8];
            let mut iov = libc::iovec {
                iov_base: marker.as_mut_ptr().cast(),
                iov_len: 1,
            };
            let space =
                unsafe { libc::CMSG_SPACE(16 * std::mem::size_of::<i32>() as u32) } as usize;
            let mut control = vec![0usize; space.div_ceil(std::mem::size_of::<usize>())];
            let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = space as _;
            let n = unsafe { libc::recvmsg(self.0.as_raw_fd(), &mut msg, 0) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut fds = Vec::new();
            unsafe {
                let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
                while !cmsg.is_null() {
                    if (*cmsg).cmsg_level == libc::SOL_SOCKET
                        && (*cmsg).cmsg_type == libc::SCM_RIGHTS
                    {
                        let len = ((*cmsg).cmsg_len as usize)
                            .saturating_sub(libc::CMSG_LEN(0) as usize)
                            / std::mem::size_of::<i32>();
                        for i in 0..len {
                            let fd = std::ptr::read_unaligned(
                                libc::CMSG_DATA(cmsg).cast::<i32>().add(i),
                            );
                            if fd >= 0 {
                                let owned = OwnedFd::from_raw_fd(fd);
                                if libc::fcntl(owned.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC)
                                    < 0
                                {
                                    return Err(io::Error::last_os_error());
                                }
                                fds.push(owned);
                            }
                        }
                    }
                    cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
                }
            }
            if n != 1 || marker != [0xa5] || msg.msg_flags & libc::MSG_CTRUNC != 0 || fds.len() != 1
            {
                return Err(io::Error::other("Invalid session descriptor transfer"));
            }
            Ok(fds.remove(0))
        }
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::fd::AsRawFd;
        #[test]
        fn descriptor_transport_rejects_non_ttys_and_bad_markers() {
            let (a, b) = UnixStream::pair().unwrap();
            let (mut a, mut b) = (Stream(a), Stream(b));
            let file = File::open("/dev/null").unwrap();
            a.send_fd(file.as_raw_fd()).unwrap();
            assert!(b.receive_pty(123).is_err());
            a.0.write_all(&[0]).unwrap();
            assert!(b.receive_fd().is_err());
        }
        #[test]
        fn executable_lease_prevents_cleanup_until_its_holder_exits() {
            let directory = std::env::temp_dir().join(format!("chda-lease-{}", token().unwrap()));
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("server.lock");
            let held = file_lease(&path, false).unwrap().unwrap();
            assert!(file_lease(&path, false).unwrap().is_none());
            drop(held);
            assert!(file_lease(&path, false).unwrap().is_some());
            std::fs::remove_file(path).unwrap();
            std::fs::remove_dir(directory).unwrap();
        }
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
    pub struct FileLease;
    pub fn file_lease(_: &Path, _: bool) -> io::Result<Option<FileLease>> {
        unsupported()
    }
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
        pub fn send_pty(&mut self, _: &crate::DetachedPty) -> io::Result<()> {
            unsupported()
        }
        pub fn receive_pty(&mut self, _: u32) -> io::Result<crate::DetachedPty> {
            unsupported()
        }
        pub fn send_reply(&mut self, _: &crate::handoff::Reply) -> io::Result<()> {
            unsupported()
        }
        pub fn receive_reply(&mut self) -> io::Result<crate::handoff::Reply> {
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
