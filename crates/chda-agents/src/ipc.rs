//! Local IPC between `chda hook` / `chda mcp` and the running app: a Unix
//! domain socket (named pipe on Windows, later). A connection carries either
//! hook events, one JSON line each, or one request (`{"request": ...}`)
//! after which the client closes its writing half and reads one reply line.

use std::path::{Path, PathBuf};

pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hook.sock")
}

#[cfg(unix)]
mod unix {
    use std::io::{self, BufRead, Read, Write};
    use std::net::Shutdown;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::mpsc::{self, Sender};
    use std::time::Duration;

    use crate::HookEvent;
    use crate::control::{Envelope, Incoming, Reply, Request};

    /// How long a request may take in the app (creating a worktree runs git).
    const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

    pub fn send(path: &Path, event: &HookEvent) -> io::Result<()> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
        let line = serde_json::to_string(event)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()
    }

    /// Send `request` to the running app and wait for its reply.
    pub fn request(path: &Path, request: &Request) -> io::Result<Reply> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        stream.set_read_timeout(Some(REPLY_TIMEOUT + Duration::from_secs(5)))?;
        let line = serde_json::to_string(&Envelope {
            request: request.clone(),
        })?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.shutdown(Shutdown::Write)?;
        let mut reply = String::new();
        io::BufReader::new(stream).read_line(&mut reply)?;
        serde_json::from_str(&reply).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Listen on `path` (replacing a stale socket) and forward every event
    /// and request to `tx`, calling `wake` after each. Connections are read
    /// one after another, so events keep their order; a request waits for
    /// its reply on a thread of its own, so it does not hold up events.
    pub fn serve(
        path: &Path,
        tx: Sender<Incoming>,
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
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    if std::io::Read::by_ref(&mut stream)
                        .take(1_048_577)
                        .read_to_string(&mut buf)
                        .is_err()
                        || buf.len() > 1_048_576
                    {
                        continue;
                    }
                    for line in buf.lines() {
                        let incoming = if let Ok(quota) =
                            serde_json::from_str::<crate::quota::QuotaSnapshot>(line)
                        {
                            Incoming::Quota(quota)
                        } else if let Ok(event) = serde_json::from_str::<HookEvent>(line) {
                            Incoming::Event(event)
                        } else if let Ok(Envelope { request }) = serde_json::from_str(line) {
                            let (reply_tx, reply_rx) = mpsc::channel();
                            let _ = std::thread::Builder::new()
                                .name("chda-ipc-reply".into())
                                .spawn(move || answer(stream, reply_rx));
                            if tx.send(Incoming::Request(request, reply_tx)).is_err() {
                                return;
                            }
                            wake();
                            break;
                        } else {
                            continue;
                        };
                        if tx.send(incoming).is_err() {
                            return;
                        }
                        wake();
                    }
                }
            })
    }

    /// Write the app's reply to a request back to its connection.
    fn answer(mut stream: UnixStream, reply: mpsc::Receiver<Reply>) {
        let reply = reply
            .recv_timeout(REPLY_TIMEOUT)
            .unwrap_or_else(|_| Reply::err("chda did not answer in time"));
        if let Ok(mut text) = serde_json::to_string(&reply) {
            text.push('\n');
            let _ = stream.write_all(text.as_bytes());
        }
    }
}

#[cfg(unix)]
pub use unix::{request, send, serve};

#[cfg(not(unix))]
pub fn send(_: &Path, _: &crate::HookEvent) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "hook IPC is not implemented on this platform",
    ))
}

#[cfg(not(unix))]
pub fn request(_: &Path, _: &crate::control::Request) -> std::io::Result<crate::control::Reply> {
    Err(std::io::Error::other(
        "hook IPC is not implemented on this platform",
    ))
}

#[cfg(not(unix))]
pub fn serve(
    _: &Path,
    _: std::sync::mpsc::Sender<crate::control::Incoming>,
    _: impl Fn() + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    Err(std::io::Error::other(
        "hook IPC is not implemented on this platform",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::control::{Incoming, Reply, Request};
    use crate::{HookEvent, HookKind};

    #[test]
    fn events_and_requests_round_trip_over_the_socket() {
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
            pane: Some(3),
        };
        send(&path, &event).unwrap();
        let Ok(Incoming::Event(got)) = rx.recv_timeout(std::time::Duration::from_secs(5)) else {
            panic!("no event");
        };
        assert_eq!(got, event);
        wake_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();

        // Events sent one after another arrive in that order, even while a
        // request waits for its reply.
        let kinds = [
            HookKind::PromptSubmitted,
            HookKind::WaitingInput,
            HookKind::Stopped,
        ];
        for i in 0..30 {
            send(
                &path,
                &HookEvent {
                    kind: kinds[i % 3],
                    timestamp: i as u64,
                    ..event.clone()
                },
            )
            .unwrap();
        }
        for i in 0..30 {
            let Ok(Incoming::Event(got)) = rx.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("no event {i}");
            };
            assert_eq!(got.timestamp, i as u64);
        }

        // A request gets the app's reply back on the same connection.
        let app = std::thread::spawn(move || {
            let Ok(Incoming::Request(req, reply)) =
                rx.recv_timeout(std::time::Duration::from_secs(5))
            else {
                panic!("no request");
            };
            assert_eq!(req, Request::ListWorktrees { cwd: "/w".into() });
            reply
                .send(Reply::ok(
                    "1 worktree",
                    serde_json::json!([{"branch": "main"}]),
                ))
                .unwrap();
        });
        let reply = request(&path, &Request::ListWorktrees { cwd: "/w".into() }).unwrap();
        app.join().unwrap();
        assert!(reply.ok);
        assert_eq!(reply.data[0]["branch"], "main");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Quota reports use the same private, bounded local transport as hooks.
#[cfg(unix)]
pub fn send_quota(path: &Path, snapshot: &crate::quota::QuotaSnapshot) -> std::io::Result<()> {
    use std::io::Write;
    let mut stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(1)))?;
    stream.write_all(serde_json::to_string(snapshot)?.as_bytes())?;
    stream.write_all(b"\n")
}
#[cfg(not(unix))]
pub fn send_quota(_: &Path, _: &crate::quota::QuotaSnapshot) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "quota IPC is unavailable",
    ))
}
