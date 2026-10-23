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
fn a_quiet_controller_keeps_the_same_server_and_shell_after_the_idle_timeout() {
    let server = Server::new();
    let client = server.client();
    let (session, events) =
        server.create(&client, "echo ready; while read x; do echo ack-$x; done");
    wait(&session, &events, "ready");
    let before = client.state().unwrap();
    let pid = session.child_pid();
    // This duration is the failure condition: exceed the former ten-second
    // socket idle timeout while the shell attachment continues polling.
    std::thread::sleep(Duration::from_secs(12));
    let after = client.state().unwrap();
    assert_eq!(before.instance, after.instance);
    assert_eq!(after.sessions[0].pid, pid);
    session.write(b"still-alive\n".to_vec());
    wait(&session, &events, "ack-still-alive");
    session.terminate();
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
    let mut fallback = event;
    fallback.timestamp = 3;
    fallback.kind = HookKind::WaitingInput;
    chda_agents::ipc::append_fallback(&server.data, &serde_json::to_string(&fallback).unwrap())
        .unwrap();
    eventually(|| {
        server
            .record(&client)
            .agent
            .is_some_and(|e| e.kind == HookKind::WaitingInput)
    });
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

// Runs only as a private successor launched by the test below. It exercises
// the real broker reply descriptor, not a simulated ownership callback.
#[test]
fn legacy_adoption_fixture() {
    let Some(fd) = std::env::var("CHDA_IMPORT_FD")
        .ok()
        .and_then(|s| s.parse().ok())
    else {
        return;
    };
    let data = PathBuf::from(std::env::var_os("CHDA_IMPORT_DATA").unwrap());
    let inherited = chda_pty::handoff::receive(fd).unwrap();
    let state = chda_core::handoff::Handoff::decode(&inherited.state).unwrap();
    let client = Client::connect(&data).unwrap();
    let mut sessions = Vec::new();
    for (metadata, pty) in state.panes.into_iter().zip(inherited.ptys) {
        let options = SessionOptions {
            cwd: Some(data.clone()),
            ..Default::default()
        };
        let detached = chda_term::DetachedSession {
            pty,
            snapshot: metadata.snapshot.clone(),
            size: chda_term::Size {
                cols: metadata.cols,
                rows: metadata.rows,
            },
        };
        sessions.push(
            client
                .prepare_import(
                    metadata.pane,
                    options,
                    detached,
                    metadata,
                    mpsc::channel().0,
                    || {},
                )
                .unwrap(),
        );
    }
    std::fs::write(data.join("prepared-import"), b"ready").unwrap();
    if std::env::var("CHDA_IMPORT_ABORT").is_ok() {
        inherited.reply.fail("test rejected before commit").unwrap();
        return;
    }
    client
        .commit_imports(inherited.reply, state.session, Default::default())
        .unwrap();
    std::fs::write(data.join("committed-import"), b"done").unwrap();
    // GUI process destruction only detaches the new remote handles.
}

#[test]
fn legacy_handoff_is_atomic_and_gui_exit_preserves_the_same_process() {
    for abort in [true, false] {
        let server = Server::new();
        let client = server.client();
        let mut workspace = chda_core::Workspace::new();
        let (_, pane) = workspace.new_tab();
        let (tx, events) = mpsc::channel();
        let session = chda_term::Session::spawn(
            server.options(
                "echo legacy-ready:$$; while IFS= read -r line; do echo legacy:$line; done",
            ),
            tx,
            || {},
        )
        .unwrap();
        let session = Session::Local(session);
        wait(&session, &events, "legacy-ready:");
        let pid = session.child_pid();
        let detached = session
            .detach()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        drop(session);
        let mut state = chda_core::handoff::Handoff::default();
        state.session.windows.push(workspace.handoff_snapshot());
        state.panes.push(chda_core::handoff::HandoffPane::new(
            pane.raw(),
            detached.size.cols,
            detached.size.rows,
            detached.snapshot.clone(),
        ));
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "legacy_adoption_fixture", "--nocapture"])
            .env_clear()
            .env("HOME", &server.data)
            .env("PATH", "/usr/bin:/bin")
            .env("CHDA_IMPORT_DATA", &server.data);
        if abort {
            command.env("CHDA_IMPORT_ABORT", "1");
        }
        let mut successor = chda_pty::handoff::spawn_command(
            command,
            &[detached.pty.try_clone().unwrap()],
            &state.encode().unwrap(),
            |c, fd| {
                c.env("CHDA_IMPORT_FD", fd.to_string());
            },
        )
        .unwrap();
        if abort {
            assert!(successor.wait_prepared(Duration::from_secs(10)).is_err());
            eventually(|| client.state().unwrap().sessions.is_empty());
            let (tx, events) = mpsc::channel();
            let restored = Session::Local(
                chda_term::Session::adopt(server.options("unused"), detached, tx, || {}).unwrap(),
            );
            assert_eq!(restored.child_pid(), pid);
            restored.write(b"rollback\n".to_vec());
            wait(&restored, &events, "legacy:rollback");
            drop(restored);
        } else {
            successor.wait_prepared(Duration::from_secs(10)).unwrap();
            // Provisional imports have no persistent checkpoint and cannot read
            // new PTY output, receive keystrokes or terminate the old child.
            assert!(
                !server
                    .data
                    .join("session-service/sessions")
                    .read_dir()
                    .unwrap()
                    .any(|p| p.unwrap().path().extension().is_some_and(|s| s == "json"))
            );
            let prepared = client
                .state()
                .unwrap()
                .sessions
                .into_iter()
                .find(|s| s.pane == pane.raw())
                .unwrap();
            assert!(client.terminate(prepared.identity).is_err());
            successor.commit().unwrap();
            drop(detached);
            eventually(|| server.data.join("committed-import").exists());
            let record = client
                .state()
                .unwrap()
                .sessions
                .into_iter()
                .find(|s| s.pane == pane.raw())
                .unwrap();
            assert_eq!(record.pid, pid);
            assert!(record.legacy_hooks);
            let (tx, events) = mpsc::channel();
            let resumed = Session::Remote(client.attach(record.identity, tx, || {}).unwrap());
            wait(&resumed, &events, "legacy-ready:");
            resumed.write(b"committed\n".to_vec());
            wait(&resumed, &events, "legacy:committed");
            resumed.terminate();
        }
    }
}

#[test]
fn idle_service_upgrade_fixture() {
    let Some(data) = std::env::var_os("CHDA_IDLE_PROFILE").map(PathBuf::from) else {
        return;
    };
    let client = Client::connect(&data).unwrap();
    let candidate = data.join("new-chda-session");
    std::fs::copy(env!("CARGO_BIN_EXE_chda-session"), &candidate).unwrap();
    let original = client.state().unwrap();
    client.set_pending_executable(data.join("missing-chda-session"));
    assert!(client.upgrade_if_idle().is_err());
    assert_eq!(
        client.state().unwrap().instance,
        original.instance,
        "a failed candidate probe keeps the previous idle server"
    );
    client.set_pending_executable(candidate.clone());
    let observer = Client::connect(&data).unwrap();
    assert!(
        !client.upgrade_if_idle().unwrap(),
        "another non-GUI connection is not exempted"
    );
    drop(observer);
    eventually(|| client.upgrade_if_idle().unwrap());
    let startup = client.state().unwrap();
    assert_ne!(startup.instance, original.instance);
    assert_eq!(startup.executable, candidate);
    let no_gui = chda_agents::ipc::request(
        &chda_agents::ipc::socket_path(&data),
        &chda_agents::control::Request::OpenTab {
            path: data.clone(),
            agent: None,
        },
    )
    .unwrap();
    assert!(
        !no_gui.ok && no_gui.message.contains("connected"),
        "startup must preserve GuiPresence=false"
    );
    client.set_gui_available(true).unwrap();
    let candidate = data.join("next-chda-session");
    std::fs::copy(env!("CARGO_BIN_EXE_chda-session"), &candidate).unwrap();
    client.set_pending_executable(candidate.clone());
    let before = client.state().unwrap();
    let options = SessionOptions {
        command: Some(vec![
            "/bin/sh".into(),
            "-c".into(),
            "echo upgrade-ready; read line".into(),
        ]),
        cwd: Some(data.clone()),
        ..Default::default()
    };
    let (tx, events) = mpsc::channel();
    let terminal = Session::Remote(client.create(1, options, tx, || {}).unwrap());
    wait(&terminal, &events, "upgrade-ready");
    assert!(!client.upgrade_if_idle().unwrap());
    assert_eq!(client.state().unwrap().instance, before.instance);
    let sessions = data.join("session-service/sessions");
    let preserved = data.join("session-service/sessions-retained");
    std::fs::rename(&sessions, &preserved).unwrap();
    std::fs::write(&sessions, b"test write failure").unwrap();
    terminal.terminate();
    eventually(|| !client.state().unwrap().sessions[0].running);
    assert!(
        !client.upgrade_if_idle().unwrap(),
        "an attached old handle prevents retirement"
    );
    drop(terminal);
    assert!(
        !client.upgrade_if_idle().unwrap(),
        "unsaved text prevents retirement"
    );
    assert_eq!(client.state().unwrap().instance, before.instance);
    std::fs::remove_file(&sessions).unwrap();
    std::fs::rename(preserved, sessions).unwrap();
    eventually(|| client.upgrade_if_idle().unwrap());
    let after = client.state().unwrap();
    assert_ne!(after.instance, before.instance);
    assert_eq!(after.executable, candidate);
    assert!(after.sessions.iter().all(|s| !s.running && s.pid.is_none()));
    std::fs::write(data.join("idle-server-pid"), after.pid.to_string()).unwrap();
    client.set_gui_available(false).unwrap();
}

#[test]
fn service_binary_changes_only_after_running_processes_and_attachments_end() {
    let server = Server::new();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "idle_service_upgrade_fixture", "--nocapture"])
        .env_clear()
        .env("HOME", &server.data)
        .env("PATH", "/usr/bin:/bin")
        .env("CHDA_HOOK_DATA_DIR", &server.data)
        .env("CHDA_IDLE_PROFILE", &server.data)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The new idle server exits on its own when the last GUI connection closes.
    let pid = std::fs::read_to_string(server.data.join("idle-server-pid")).unwrap();
    eventually(|| {
        !Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .output()
            .unwrap()
            .status
            .success()
    });
}
