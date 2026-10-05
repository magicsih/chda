//! Bounded local helper processes for agent discovery and telemetry.

/// Follow symlinks and reject directories, broken links and non-executable files.
pub fn is_executable(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Capture bounded stdout without letting shell startup or tools block the UI.
/// A private process group is terminated even when descendants keep stdout open.
pub fn capture_command(
    command: &mut std::process::Command,
    input: Option<&[u8]>,
    timeout: std::time::Duration,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    use std::{
        io::{Read, Write},
        process::Stdio,
        sync::mpsc,
        time::Instant,
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    struct Probe(std::process::Child);
    impl Drop for Probe {
        fn drop(&mut self) {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = Probe(command.spawn()?);
    let stdout = child.0.stdout.take().unwrap();
    let stdin = child.0.stdin.take();
    let bytes = input.map(<[u8]>::to_vec);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let (Some(mut stdin), Some(bytes)) = (stdin, bytes) {
            let _ = stdin.write_all(&bytes);
        }
        let mut output = Vec::new();
        let result = stdout.take(max_bytes as u64 + 1).read_to_end(&mut output);
        let result = result.and_then(|_| {
            if output.len() > max_bytes {
                Err(std::io::Error::other("tool output exceeded its bound"))
            } else {
                Ok(output)
            }
        });
        let _ = tx.send(result);
    });
    let deadline = Instant::now() + timeout;
    let output = rx.recv_timeout(timeout).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "tool did not return within its deadline",
        )
    })??;
    loop {
        if let Some(status) = child.0.try_wait()? {
            return if status.success() {
                Ok(output)
            } else {
                Err(std::io::Error::other("tool exited unsuccessfully"))
            };
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "tool did not exit within its deadline",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Run a streaming helper in a private process group and always reap it.
pub fn with_process<T>(
    command: &mut std::process::Command,
    exchange: impl FnOnce(&mut std::process::Child) -> std::io::Result<T>,
) -> std::io::Result<T> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    struct Guard(std::process::Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Guard(command.spawn()?);
    exchange(&mut child.0)
}
