//! Local IPC between `chda hook` and the running app: a Unix domain socket
//! (named pipe on Windows, later) carrying one JSON event per connection.

use std::path::{Path, PathBuf};

pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hook.sock")
}

#[cfg(unix)]
mod unix {
    use std::io::{self, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::mpsc::Sender;

    use crate::HookEvent;

    pub fn send(path: &Path, event: &HookEvent) -> io::Result<()> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
        let line = serde_json::to_string(event)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()
    }

    /// Listen on `path` (replacing a stale socket) and forward every event
    /// to `tx`, calling `wake` after each. Runs until the listener fails.
    pub fn serve(
        path: &Path,
        tx: Sender<HookEvent>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<std::thread::JoinHandle<()>> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        std::thread::Builder::new()
            .name("chda-hook-ipc".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut buf = String::new();
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                    if stream.read_to_string(&mut buf).is_err() {
                        continue;
                    }
                    for line in buf.lines() {
                        if let Ok(event) = serde_json::from_str::<HookEvent>(line) {
                            if tx.send(event).is_err() {
                                return;
                            }
                            wake();
                        }
                    }
                }
            })
    }
}

#[cfg(unix)]
pub use unix::{send, serve};

#[cfg(not(unix))]
pub fn send(_: &Path, _: &HookEvent) -> io::Result<()> {
    Err(io::Error::other(
        "hook IPC is not implemented on this platform",
    ))
}

#[cfg(not(unix))]
pub fn serve(
    _: &Path,
    _: std::sync::mpsc::Sender<HookEvent>,
    _: impl Fn() + Send + 'static,
) -> io::Result<std::thread::JoinHandle<()>> {
    Err(io::Error::other(
        "hook IPC is not implemented on this platform",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{HookEvent, HookKind};

    #[test]
    fn events_round_trip_over_the_socket() {
        let dir = std::env::temp_dir().join(format!("chda-ipc-{}", std::process::id()));
        let path = socket_path(&dir);
        let (tx, rx) = std::sync::mpsc::channel();
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        let _server = serve(&path, tx, move || {
            let _ = wake_tx.send(());
        })
        .unwrap();
        let event = HookEvent {
            agent: "claude".into(),
            session_id: "s1".into(),
            cwd: "/w".into(),
            kind: HookKind::Stopped,
            timestamp: 7,
        };
        send(&path, &event).unwrap();
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            event
        );
        wake_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
