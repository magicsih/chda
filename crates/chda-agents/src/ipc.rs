//! Local IPC between `chda hook` / `chda mcp` and the running app: a Unix
//! domain socket (named pipe on Windows, later). A connection carries either
//! hook events, one JSON line each, or one request (`{"request": ...}`)
//! after which the client reads one reply line. Newlines delimit messages;
//! acknowledgement does not depend on closing either half of the socket.

mod process;
pub use process::{capture_command, is_executable, with_process, write_private_atomic};

use std::path::{Path, PathBuf};

/// Serializes the transition from queued delivery to a durable handoff journal.
/// The same lock covers journaling and enqueueing, so freezing and draining
/// the GUI cannot miss an event already acknowledged to a hook client.
pub struct Server {
    journal: std::sync::Arc<std::sync::Mutex<Option<std::fs::File>>>,
}
impl Server {
    pub fn journal_to(&self, path: Option<&Path>) -> std::io::Result<()> {
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| std::io::Error::other("IPC journal lock failed"))?;
        *journal = path
            .map(|p| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
            })
            .transpose()?;
        Ok(())
    }
}

/// Fallback hooks are serialized across processes, including draining on startup.
/// A separate lock file avoids the append-versus-unlink race on the event file.
pub fn append_fallback(dir: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("events.lock"))?;
    lock.lock()?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("events.jsonl"))?;
    writeln!(file, "{line}")
}

pub fn take_fallback(dir: &Path) -> std::io::Result<String> {
    let path = dir.join("events.jsonl");
    if !path.exists() {
        return Ok(String::new());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("events.lock"))?;
    lock.lock()?;
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            std::fs::remove_file(path)?;
            Ok(text)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hook.sock")
}

#[cfg(unix)]
mod unix {
    use std::io::{self, BufRead, Read, Write};
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
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let line = serde_json::to_string(event)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        let mut ack = [0; 3];
        stream.read_exact(&mut ack)?;
        if ack != *b"ok\n" {
            return Err(io::Error::other("Hook delivery was not acknowledged"));
        }
        Ok(())
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
    ) -> io::Result<super::Server> {
        serve_journaled(path, tx, wake, None)
    }

    pub fn serve_journaled(
        path: &Path,
        tx: Sender<Incoming>,
        wake: impl Fn() + Send + 'static,
        journal: Option<&Path>,
    ) -> io::Result<super::Server> {
        let server = super::Server {
            journal: Default::default(),
        };
        server.journal_to(journal)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let gate = server.journal.clone();
        std::thread::Builder::new()
            .name("chda-hook-ipc".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut buf = String::new();
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    if io::BufReader::new(&mut stream)
                        .take(1_048_577)
                        .read_line(&mut buf)
                        .is_err()
                        || buf.len() > 1_048_576
                        || !buf.ends_with('\n')
                    {
                        continue;
                    }
                    for line in buf.lines() {
                        let Ok(mut journal) = gate.lock() else { return };
                        let incoming = if let Ok(quota) =
                            serde_json::from_str::<crate::quota::QuotaSnapshot>(line)
                        {
                            Incoming::Quota(quota)
                        } else if let Ok(event) = serde_json::from_str::<HookEvent>(line) {
                            Incoming::Event(event)
                        } else if let Ok(Envelope { request }) = serde_json::from_str(line) {
                            if journal.is_some() {
                                let _ = stream.write_all(
                                    serde_json::to_string(&Reply::err(
                                        "chda is reconnecting after an update; retry shortly",
                                    ))
                                    .unwrap()
                                    .as_bytes(),
                                );
                                break;
                            }
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
                        if let Some(file) = journal.as_mut() {
                            // A failed write is not acknowledged; the hook falls back.
                            if writeln!(file, "{line}").is_err() {
                                break;
                            }
                        }
                        if tx.send(incoming).is_err() {
                            return;
                        }
                        wake();
                        let _ = stream.write_all(b"ok\n");
                    }
                }
            })?;
        Ok(server)
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
pub use unix::{request, send, serve, serve_journaled};

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
) -> std::io::Result<Server> {
    Err(std::io::Error::other(
        "hook IPC is not implemented on this platform",
    ))
}

#[cfg(not(unix))]
pub fn serve_journaled(
    path: &Path,
    tx: std::sync::mpsc::Sender<crate::control::Incoming>,
    wake: impl Fn() + Send + 'static,
    _: Option<&Path>,
) -> std::io::Result<Server> {
    serve(path, tx, wake)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::control::{Incoming, Reply, Request};
    use crate::{HookEvent, HookKind};

    #[test]
    fn hook_is_acknowledged_without_half_closing_the_connection() {
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join(format!("chda-ipc-open-{}", std::process::id()));
        let path = socket_path(&dir);
        let (tx, rx) = std::sync::mpsc::channel();
        let _server = serve(&path, tx, || {}).unwrap();
        let mut client = std::os::unix::net::UnixStream::connect(&path).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        client.write_all(b"{\"agent\":\"claude\",\"session_id\":\"live\",\"cwd\":\"/w\",\"kind\":\"stopped\",\"timestamp\":7}\n").unwrap();
        let mut ack = [0; 3];
        client.read_exact(&mut ack).unwrap();
        assert_eq!(&ack, b"ok\n");
        assert!(matches!(rx.recv().unwrap(), Incoming::Event(_)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn journal_barrier_keeps_acknowledged_events_and_rejects_mutations() {
        let dir = std::env::temp_dir().join(format!("chda-ipc-journal-{}", std::process::id()));
        let path = socket_path(&dir);
        let (tx, rx) = std::sync::mpsc::channel();
        let server = serve(&path, tx, || {}).unwrap();
        let event = HookEvent {
            agent: "claude".into(),
            session_id: "live".into(),
            cwd: "/w".into(),
            kind: HookKind::WaitingInput,
            timestamp: 7,
            pane: Some(3),
        };
        send(&path, &event).unwrap();
        server.journal_to(Some(&dir.join("handoff.jsonl"))).unwrap();
        send(&path, &event).unwrap();
        let journal = std::fs::read_to_string(dir.join("handoff.jsonl")).unwrap();
        assert_eq!(journal.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<HookEvent>(journal.trim()).unwrap(),
            event
        );
        assert_eq!(rx.try_iter().count(), 2);
        let reply = request(
            &path,
            &Request::OpenTab {
                path: "/w".into(),
                agent: None,
            },
        )
        .unwrap();
        assert!(!reply.ok);
        server.journal_to(None).unwrap();
        send(&path, &event).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("handoff.jsonl")).unwrap(),
            journal
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn draining_fallback_does_not_lose_concurrent_appends() {
        let dir = std::env::temp_dir().join(format!("chda-fallback-{}", std::process::id()));
        let threads: Vec<_> = (0..4)
            .map(|producer| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for n in 0..100 {
                        append_fallback(&dir, &format!("{producer}:{n}")).unwrap();
                    }
                })
            })
            .collect();
        let mut lines = Vec::new();
        while threads.iter().any(|t| !t.is_finished()) {
            lines.extend(take_fallback(&dir).unwrap().lines().map(str::to_owned));
            std::thread::yield_now();
        }
        for thread in threads {
            thread.join().unwrap();
        }
        lines.extend(take_fallback(&dir).unwrap().lines().map(str::to_owned));
        assert_eq!(lines.len(), 400);
        lines.sort();
        lines.dedup();
        assert_eq!(lines.len(), 400);
        std::fs::remove_dir_all(dir).unwrap();
    }

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
    use std::io::{Read, Write};
    let json = serde_json::to_string(snapshot)?;
    let deliver = || -> std::io::Result<()> {
        let mut stream = std::os::unix::net::UnixStream::connect(path)?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(1)))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
        writeln!(stream, "{json}")?;
        let mut ack = [0; 3];
        stream.read_exact(&mut ack)?;
        if ack != *b"ok\n" {
            return Err(std::io::Error::other("quota delivery not acknowledged"));
        }
        Ok(())
    };
    deliver().or_else(|_| {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("missing quota directory"))?;
        append_fallback(parent, &json)
    })
}
#[cfg(not(unix))]
pub fn send_quota(_: &Path, _: &crate::quota::QuotaSnapshot) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "quota IPC is unavailable",
    ))
}
