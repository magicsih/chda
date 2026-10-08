//! The CLI proxy forwards raw WebSocket bytes, rather than JSON lines.
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct Cli {
    root: PathBuf,
    bin: PathBuf,
    requests: Arc<Mutex<Vec<Value>>>,
    server: Option<std::thread::JoinHandle<()>>,
}
impl Cli {
    fn new(response: Option<Value>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "chda-child-probe-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let server = std::thread::spawn(move || {
            // The helper can fail before connecting. Test cleanup must not
            // wait forever in accept when that happens.
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let Ok(mut socket) = tungstenite::accept(stream) else {
                return;
            };
            while let Ok(message) = socket.read() {
                let tungstenite::Message::Text(text) = message else {
                    continue;
                };
                let Ok(request) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                recorded.lock().unwrap().push(request.clone());
                let reply = match request["method"].as_str() {
                    Some("initialize") => Some(json!({"result":{}})),
                    Some("thread/list") => response.clone(),
                    _ => None,
                };
                if let Some(mut reply) = reply {
                    reply["id"] = request["id"].clone();
                    if socket
                        .send(tungstenite::Message::Text(reply.to_string().into()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });
        // Standard-library byte forwarding keeps this fixture faithful to
        // codex-stdio-to-uds without implementing a second WebSocket protocol.
        let proxy = root.join("proxy.py");
        std::fs::write(
            &proxy,
            r#"import os, selectors, socket, sys
s=socket.create_connection(('127.0.0.1',int(sys.argv[1])))
poll=selectors.DefaultSelector()
poll.register(s,selectors.EVENT_READ)
poll.register(sys.stdin.buffer,selectors.EVENT_READ)
while True:
 for key,_ in poll.select():
  if key.fileobj is s:
   data=s.recv(65536)
   if not data:sys.exit(0)
   sys.stdout.buffer.write(data);sys.stdout.buffer.flush()
  else:
   data=os.read(0,65536)
   if not data:sys.exit(0)
   s.sendall(data)
"#,
        )
        .unwrap();
        let bin = root.join("codex");
        std::fs::write(&bin,format!("#!/bin/sh\n[ \"$*\" = 'app-server proxy' ] || exit 42\necho $$ > \"$HOME/helper-pid\"\nexec python3 -u {} {port}\n",crate::shell_quote(&proxy))).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            root,
            bin,
            requests,
            server: Some(server),
        }
    }
    fn env(&self) -> Vec<(String, String)> {
        vec![("HOME".into(), self.root.to_string_lossy().into_owned())]
    }
    fn assert_reaped(&self) {
        let pid = std::fs::read_to_string(self.root.join("helper-pid"))
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "only our proxy is reaped"
        );
    }
}
impl Drop for Cli {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn children_query_existing_runtime_metadata_without_resuming_or_reading_turns() {
    let cli = Cli::new(Some(
        json!({"result":{"data":[{"id":"child","parentThreadId":"parent","status":{"type":"active","activeFlags":["waitingOnUserInput"]},"agentNickname":null,"agentRole":"reviewer","preview":"private answer"},{"id":"unrelated","parentThreadId":"other","status":{"type":"active","activeFlags":[]}}],"nextCursor":null}}),
    ));
    let result = crate::read_codex_children(&cli.bin, &cli.env(), &["parent".into()], 100).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].child.state, crate::ChildState::WaitingInput);
    assert_eq!(result[0].child.label.as_deref(), Some("reviewer"));
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("private answer")
    );
    let requests = cli.requests.lock().unwrap();
    let methods: Vec<_> = requests
        .iter()
        .filter_map(|r| r["method"].as_str())
        .collect();
    assert_eq!(methods, vec!["initialize", "initialized", "thread/list"]);
    assert_eq!(requests[2]["params"]["parentThreadId"], "parent");
    assert_eq!(requests[2]["params"]["useStateDbOnly"], true);
    cli.assert_reaped();
}

#[test]
fn rejected_queries_do_not_relay_server_content() {
    let cli = Cli::new(Some(
        json!({"error":{"code":-32601,"message":"private prompt"}}),
    ));
    let error =
        crate::read_codex_children(&cli.bin, &cli.env(), &["parent".into()], 100).unwrap_err();
    assert!(!error.to_string().contains("private prompt"));
    cli.assert_reaped();
}

#[test]
fn an_unresponsive_local_transport_is_bounded_and_reaped() {
    let cli = Cli::new(None);
    let mut command = std::process::Command::new(&cli.bin);
    command.args(["app-server", "proxy"]).envs(cli.env());
    let start = std::time::Instant::now();
    let error = super::with_local_rpc(&mut command, std::time::Duration::from_secs(5), |rpc| {
        rpc.request("thread/list", json!({}))
    })
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(start.elapsed() < std::time::Duration::from_secs(7));
    cli.assert_reaped();
}

#[test]
#[ignore = "requires an explicitly selected installed CLI and its existing local daemon"]
fn installed_daemon_metadata_capability() {
    let executable =
        std::env::var_os("CHDA_TEST_CODEX_EXECUTABLE").expect("select a local CLI explicitly");
    let events = crate::read_codex_children(
        std::path::Path::new(&executable),
        &[],
        &["01900000-0000-7000-8000-000000000001".into()],
        100,
    )
    .expect("installed daemon must accept an exact-parent metadata-only query");
    assert!(events.is_empty());
    eprintln!(
        "Existing local daemon accepted exact-parent metadata query; zero model calls. Synthetic parent has zero children; actual child lifecycle remains a separate integration check."
    );
}
