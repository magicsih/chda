//! Real processes in private temporary profiles. No GUI, user shell setup,
//! provider authentication, installed applications or physical reboot.
use chda_session::{Client, Session, SessionRecord};
use chda_term::{Event, SessionOptions};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Server {
    data: PathBuf,
    child: Child,
}
impl Server {
    fn new() -> Self {
        let data = std::env::temp_dir().join(format!("cs-{}", chda_pty::local::token().unwrap()));
        std::fs::create_dir(&data).unwrap();
        Self::at(data)
    }
    fn at(data: PathBuf) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_chda-session"))
            .arg("serve")
            .arg(&data)
            .env_clear()
            .env("HOME", &data)
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .env("CHDA_HOOK_DATA_DIR", &data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let s = Self { data, child };
        let _ = s.client();
        s
    }
    fn client(&self) -> Client {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match Client::connect(&self.data) {
                Ok(c) => return c,
                Err(e) if Instant::now() >= deadline => panic!("server connection: {e}"),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
    fn options(&self, script: &str) -> SessionOptions {
        let mut options = SessionOptions {
            command: Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            cwd: Some(self.data.clone()),
            cell_width_px: 8,
            cell_height_px: 16,
            ..Default::default()
        };
        options.env.extend([
            ("HOME".into(), self.data.to_string_lossy().into_owned()),
            ("PATH".into(), "/usr/bin:/bin".into()),
            (
                "CHDA_HOOK_DATA_DIR".into(),
                self.data.to_string_lossy().into_owned(),
            ),
            ("CHDA_PANE_ID".into(), "1".into()),
        ]);
        options
    }
    fn create(&self, client: &Client, script: &str) -> (Session, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (
            Session::Remote(client.create(1, self.options(script), tx, || {}).unwrap()),
            rx,
        )
    }
    fn record(&self, client: &Client) -> SessionRecord {
        client
            .state()
            .unwrap()
            .sessions
            .into_iter()
            .find(|s| s.pane == 1)
            .unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("test server status: {:?}", self.child.try_wait());
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data);
    }
}
fn wait(session: &Session, events: &mpsc::Receiver<Event>, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let f = session.frame();
        if (0..f.size.rows).any(|y| f.row_text(y).contains(text)) {
            return;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        events
            .recv_timeout(left)
            .unwrap_or_else(|e| panic!("missing {text}: {e}; {f:?}"));
    }
}
fn eventually(mut f: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < until, "condition timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn disconnect_reconnect_preserves_pid_output_and_input_until_explicit_termination() {
    let server = Server::new();
    let client = server.client();
    let (session, events) = server.create(
        &client,
        "echo ready:$$; while IFS= read -r line; do printf 'got:%s\\n' \"$line\"; done",
    );
    wait(&session, &events, "ready:");
    let before = server.record(&client);
    let pid = session.child_pid();
    session.write(b"one\n".to_vec());
    wait(&session, &events, "got:one");
    drop(session);
    drop(client);
    let client = server.client();
    let after = server.record(&client);
    assert_eq!(after.pid, pid);
    assert_eq!(before.identity, after.identity);
    assert!(after.running);
    let (tx, events) = mpsc::channel();
    let session = Session::Remote(client.attach(after.identity.clone(), tx, || {}).unwrap());
    wait(&session, &events, "got:one");
    session.write(b"two\n".to_vec());
    wait(&session, &events, "got:two");
    client.terminate(after.identity.clone()).unwrap();
    eventually(|| !server.record(&client).running);
    assert!(
        client
            .attach(after.identity, mpsc::channel().0, || {})
            .is_err()
    );
}

#[test]
fn server_restart_restores_text_without_spawning_and_resume_has_a_new_identity() {
    let mut server = Server::new();
    let client = server.client();
    let (session, events) = server.create(
        &client,
        "echo checkpoint-text; while read line; do echo $line; done",
    );
    wait(&session, &events, "checkpoint-text");
    client.checkpoint(None).unwrap();
    let before = server.record(&client);
    assert!(before.saved_at.is_some());
    drop(session);
    drop(client);
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    let data = server.data.clone();
    let mut restarted = Server::at(data);
    server.data = PathBuf::new();
    let client = restarted.client();
    let after = restarted.record(&client);
    assert!(!after.running);
    assert_eq!(after.pid, None);
    assert!(
        client
            .output(after.identity.clone())
            .unwrap()
            .contains("checkpoint-text")
    );
    assert!(
        client
            .attach(after.identity.clone(), mpsc::channel().0, || {})
            .is_err()
    );
    let (tx, events) = mpsc::channel();
    let session = Session::Remote(
        client
            .resume(
                after.identity.clone(),
                restarted.options("echo resumed; read line"),
                tx,
                || {},
            )
            .unwrap(),
    );
    wait(&session, &events, "resumed");
    let resumed = restarted.record(&client);
    assert_ne!(resumed.identity.run, before.identity.run);
    assert_ne!(resumed.identity.server, before.identity.server);
    assert_eq!(resumed.identity.session, before.identity.session);
    assert!(
        client
            .resume(
                after.identity.clone(),
                restarted.options("echo should-not-run"),
                mpsc::channel().0,
                || {}
            )
            .is_err()
    );
    assert!(client.terminate(after.identity).is_err());
    assert!(restarted.record(&client).running);
    client.terminate(resumed.identity).unwrap();
    drop(session);
    let _ = restarted.child.kill();
}

#[test]
fn detached_completion_keeps_exit_status_and_last_output() {
    let server = Server::new();
    let client = server.client();
    let (session, events) = server.create(
        &client,
        "echo before; read line; echo finished-detached; exit 7",
    );
    wait(&session, &events, "before");
    session.write(b"done\n".to_vec());
    drop(session);
    eventually(|| !server.record(&client).running);
    let record = server.record(&client);
    assert_eq!(record.exit.as_ref().unwrap().code, 7);
    assert!(
        client
            .output(record.identity)
            .unwrap()
            .contains("finished-detached")
    );
}

#[test]
fn duplicate_server_cannot_unlink_the_active_socket() {
    let server = Server::new();
    let client = server.client();
    let instance = client.state().unwrap().instance;
    let output = Command::new(env!("CARGO_BIN_EXE_chda-session"))
        .arg("serve")
        .arg(&server.data)
        .env_clear()
        .env("HOME", &server.data)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(server.client().state().unwrap().instance, instance);
}

#[test]
fn checkpoints_do_not_contain_environment_or_an_input_log() {
    let server = Server::new();
    let client = server.client();
    let mut options = server.options("echo output-only; read line");
    options
        .env
        .push(("UNUSED_TEST_SECRET".into(), "private-env-sentinel".into()));
    let (tx, events) = mpsc::channel();
    let session = Session::Remote(client.create(1, options, tx, || {}).unwrap());
    wait(&session, &events, "output-only");
    client.checkpoint(None).unwrap();
    for file in std::fs::read_dir(server.data.join("session-service/sessions"))
        .unwrap()
        .flatten()
    {
        let bytes = std::fs::read_to_string(file.path()).unwrap();
        assert!(!bytes.contains("private-env-sentinel"));
        assert!(!bytes.contains("UNUSED_TEST_SECRET"));
        assert!(!bytes.contains("\"env\""));
    }
    session.terminate();
}

#[test]
fn reconnect_gets_images_but_does_not_replay_clipboard_or_notifications() {
    let server = Server::new();
    let client = server.client();
    let(session,events)=server.create(&client,"printf '\\033]52;c;Y2xpcA==\\007\\033]9;old-notification\\007'; printf '\\033_Ga=T,f=32,s=1,v=1;/////w==\\033\\\\'; echo image-ready; printf '\\033]0;current-title\\007\\033]7;file://localhost/tmp\\007\\033]133;A\\007prompt%% \\033]133;B\\007'; read line");
    wait(&session, &events, "image-ready");
    eventually(|| session.frame().at_prompt());
    assert!(!session.frame().images.is_empty());
    let id = server.record(&client).identity;
    drop(session);
    let (tx, events) = mpsc::channel();
    let session = Session::Remote(client.attach(id, tx, || {}).unwrap());
    wait(&session, &events, "image-ready");
    assert!(!session.frame().images.is_empty());
    let mut title = false;
    let mut cwd = false;
    let mut prompt = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(title && cwd && prompt) {
        assert!(
            Instant::now() < deadline,
            "Attach must return current metadata and prompt readiness"
        );
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        title |= matches!(&event, Event::TitleChanged(t) if t == "current-title");
        cwd |= matches!(&event, Event::PwdChanged(p) if p == "file://localhost/tmp");
        prompt |= matches!(event, Event::PromptShown);
        assert!(!matches!(
            event,
            Event::ClipboardWrite(_) | Event::Notification(_)
        ));
    }
    session.terminate();
}

#[test]
fn incompatible_or_oversized_connections_leave_the_existing_run_alive() {
    use std::io::{Read, Write};
    let server = Server::new();
    let client = server.client();
    let (session, events) = server.create(&client, "echo protocol-ready; read line");
    wait(&session, &events, "protocol-ready");
    let before = server.record(&client);
    let mut stream =
        chda_pty::local::Stream::connect(&chda_session::socket_path(&server.data)).unwrap();
    stream.timeout(Duration::from_secs(3)).unwrap();
    let bytes = br#"{"version":999,"request":"Hello"}"#;
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(bytes).unwrap();
    let mut size = [0; 4];
    stream.read_exact(&mut size).unwrap();
    let mut response = vec![0; u32::from_be_bytes(size) as usize];
    stream.read_exact(&mut response).unwrap();
    assert!(
        String::from_utf8(response)
            .unwrap()
            .contains("Incompatible")
    );
    let mut stream =
        chda_pty::local::Stream::connect(&chda_session::socket_path(&server.data)).unwrap();
    stream.timeout(Duration::from_secs(3)).unwrap();
    stream.write_all(&u32::MAX.to_be_bytes()).unwrap();
    assert!(stream.read_exact(&mut size).is_err());
    let after = server.record(&client);
    assert_eq!(before.identity, after.identity);
    assert_eq!(before.pid, after.pid);
    assert!(after.running);
    session.terminate();
}

#[test]
fn hooks_and_read_only_mcp_work_without_a_gui_and_stale_runs_are_rejected() {
    use chda_agents::{HookEvent, HookKind, control::Request};
    let server = Server::new();
    // Keep a GUI lease while git starts (which can exceed the empty-server
    // idle timeout on a cold CI runner). Drop it below to test no-GUI access.
    let client = server.client();
    assert!(
        Command::new("/usr/bin/git")
            .args(["init", "-q"])
            .arg(&server.data)
            .env("HOME", &server.data)
            .status()
            .unwrap()
            .success()
    );
    let (session, events) = server.create(&client, "echo no-gui-ready; read line");
    wait(&session, &events, "no-gui-ready");
    let record = server.record(&client);
    drop(session);
    drop(client);
    let socket = chda_agents::ipc::socket_path(&server.data);
    let output = chda_agents::ipc::request(
        &socket,
        &Request::ReadOutput {
            cwd: server.data.clone(),
            pane: 1,
            lines: Some(10),
        },
    )
    .unwrap();
    assert!(output.ok, "{output:?}");
    assert!(
        output.data["text"]
            .as_str()
            .unwrap()
            .contains("no-gui-ready")
    );
    let event = HookEvent {
        run: Some(record.identity.run.clone()),
        agent: "claude".into(),
        session_id: "exact-test-conversation".into(),
        cwd: server.data.clone(),
        kind: HookKind::Stopped,
        timestamp: 1,
        pane: Some(1),
        child: None,
    };
    chda_agents::ipc::send(&socket, &event).unwrap();
    let status = chda_agents::ipc::request(
        &socket,
        &Request::WaitForStatus {
            cwd: server.data.clone(),
            pane: 1,
            states: vec!["turn_complete".into()],
            timeout_ms: Some(2000),
        },
    )
    .unwrap();
    assert!(status.ok);
    assert_eq!(status.data["state"], "turn_complete");
    let reply = chda_agents::ipc::request(
        &socket,
        &Request::OpenTab {
            path: server.data.clone(),
            agent: None,
        },
    )
    .unwrap();
    assert!(!reply.ok);
    assert!(reply.message.contains("connected"));
    let mut stale = event.clone();
    stale.run = Some("old-run".into());
    stale.timestamp = 2;
    stale.kind = HookKind::PromptSubmitted;
    chda_agents::ipc::send(&socket, &stale).unwrap();
    let client = server.client();
    eventually(|| server.record(&client).agent.is_some());
    assert_eq!(
        server.record(&client).agent.unwrap().kind,
        HookKind::Stopped
    );
    client.terminate(record.identity).unwrap();
}

#[test]
fn codex_title_completion_is_owned_by_the_service_without_a_gui() {
    let server = Server::new();
    let client = server.client();
    let (session, events) = server.create(&client, "echo codex-ready; while [ ! -f \"$HOME/go\" ]; do sleep 0.02; done; printf '\\033]0;codex | Working\\007'; sleep 0.1; printf '\\033]0;codex | Ready\\007'; read line");
    wait(&session, &events, "codex-ready");
    drop(session);
    drop(client);
    std::fs::write(server.data.join("go"), "").unwrap();
    let client = server.client();
    eventually(|| {
        server
            .record(&client)
            .agent
            .as_ref()
            .is_some_and(|e| e.agent == "codex" && e.kind == chda_agents::HookKind::Stopped)
    });
    let record = server.record(&client);
    assert!(
        record.agent.unwrap().session_id.is_empty(),
        "A title cannot invent an exact conversation ID"
    );
    assert!(record.running);
    client.terminate(record.identity).unwrap();
}

#[test]
fn a_checkpoint_write_failure_keeps_the_live_process_and_last_successful_time() {
    let server = Server::new();
    let client = server.client();
    let (session, events) = server.create(
        &client,
        "echo save-ready; while read line; do echo saved:$line; done",
    );
    wait(&session, &events, "save-ready");
    client.checkpoint(None).unwrap();
    let before = server.record(&client);
    let path = server.data.join("session-service/sessions");
    let backup = path.with_extension("healthy");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::write(&path, b"blocked storage").unwrap();
    session.write(b"write-failure\n".to_vec());
    wait(&session, &events, "saved:write-failure");
    client.checkpoint(None).unwrap();
    let failed = server.record(&client);
    assert!(failed.running);
    assert_eq!(failed.pid, before.pid);
    assert_eq!(failed.saved_at, before.saved_at);
    assert!(failed.save_error.is_some());
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    client.checkpoint(None).unwrap();
    let recovered = server.record(&client);
    assert!(recovered.save_error.is_none());
    assert!(recovered.saved_at >= before.saved_at);
    assert!(
        client
            .output(recovered.identity)
            .unwrap()
            .contains("saved:write-failure")
    );
    session.terminate();
}
