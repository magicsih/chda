use super::*;

fn record() -> SessionRecord {
    SessionRecord {
        pane: 1,
        identity: Identity {
            session: "a".repeat(32),
            run: "b".repeat(32),
            server: "c".repeat(32),
        },
        pid: Some(1),
        running: true,
        closed: false,
        title: String::new(),
        cwd: None,
        launch: None,
        agent: None,
        exit: None,
        saved_at: None,
        save_error: None,
        scrollback: 1000,
        legacy_hooks: false,
    }
}
fn state() -> State {
    State {
        pid: 1,
        sequence: 0,
        children: Default::default(),
        executable: PathBuf::new(),
        instance: record().identity.server,
        executable_version: "test".into(),
        layout: None,
        sessions: vec![record()],
        save_error: None,
        layout_saved_at: None,
    }
}

#[test]
fn layout_queue_returns_while_a_save_is_blocked_and_persists_the_latest_snapshot() {
    let data =
        std::env::temp_dir().join(format!("chda-layout-{}", chda_pty::local::token().unwrap()));
    std::fs::create_dir(&data).unwrap();
    let listener = chda_pty::local::Listener::bind(&crate::socket_path(&data)).unwrap();
    let connection = Connection::open(&data).unwrap();
    let mut peer = listener.accept().unwrap();
    let client = Client {
        data: data.clone(),
        connection: Arc::new(Mutex::new(connection)),
        instance: Arc::new(Mutex::new(record().identity.server)),
        executable: Default::default(),
        layout: Default::default(),
        imports: Default::default(),
        group: String::new(),
        pending_executable: Default::default(),
        gui_available: Default::default(),
    };
    let (saving, blocked) = mpsc::channel();
    let (release, proceed) = mpsc::channel();
    let server = std::thread::spawn(move || {
        peer.timeout(Duration::from_secs(3)).unwrap();
        let first: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(
            matches!(first.request, Request::Checkpoint { layout: Some(s) } if s.active_window == 0)
        );
        saving.send(()).unwrap();
        proceed.recv().unwrap();
        protocol::write(&mut peer, &Response::State(state())).unwrap();
        let last: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(
            matches!(last.request, Request::Checkpoint { layout: Some(s) } if s.active_window == 100)
        );
        protocol::write(&mut peer, &Response::State(state())).unwrap();
    });
    client.queue_layout(Default::default());
    blocked.recv_timeout(Duration::from_secs(3)).unwrap();
    let (done, queued) = mpsc::channel();
    let producer = client.clone();
    let thread = std::thread::spawn(move || {
        for active_window in 1..=100 {
            producer.queue_layout(chda_core::SavedSession {
                active_window,
                windows: vec![],
            });
        }
        assert_eq!(producer.helper_executable().unwrap(), PathBuf::new());
        done.send(()).unwrap();
    });
    let result = queued.recv_timeout(Duration::from_millis(200));
    release.send(()).unwrap();
    thread.join().unwrap();
    client.flush_layout().unwrap();
    server.join().unwrap();
    drop(client);
    drop(listener);
    std::fs::remove_dir_all(data).unwrap();
    assert!(result.is_ok(), "queueing a layout waited for the service");
}

#[test]
fn controller_reconnects_for_observation_but_does_not_retry_a_mutation() {
    let data = std::env::temp_dir().join(format!(
        "chda-controller-{}",
        chda_pty::local::token().unwrap()
    ));
    std::fs::create_dir(&data).unwrap();
    let listener = chda_pty::local::Listener::bind(&crate::socket_path(&data)).unwrap();
    let connection = Connection::open(&data).unwrap();
    let mut peer = listener.accept().unwrap();
    let client = Client {
        data: data.clone(),
        connection: Arc::new(Mutex::new(connection)),
        instance: Arc::new(Mutex::new(record().identity.server)),
        executable: Default::default(),
        layout: Default::default(),
        imports: Default::default(),
        group: String::new(),
        pending_executable: Default::default(),
        gui_available: Default::default(),
    };
    let server = std::thread::spawn(move || {
        let request: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(matches!(request.request, Request::ReserveIds));
        // Losing this reply does not authorize repeating the allocation.
        drop(peer);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut peer = loop {
            match listener.accept() {
                Ok(peer) => break peer,
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::yield_now()
                }
                Err(e) => panic!("no controller reconnect: {e}"),
            }
        };
        peer.timeout(Duration::from_secs(3)).unwrap();
        let hello: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(matches!(hello.request, Request::Hello));
        protocol::write(&mut peer, &Response::State(state())).unwrap();
        let presence: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(matches!(presence.request, Request::GuiPresence(false)));
        protocol::write(&mut peer, &Response::Ok).unwrap();
        let observation: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(matches!(observation.request, Request::List));
        protocol::write(&mut peer, &Response::State(state())).unwrap();
    });
    assert!(client.reserve_ids().is_err());
    assert_eq!(client.state().unwrap().instance, record().identity.server);
    server.join().unwrap();
    drop(client);
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn a_blocked_poll_does_not_block_reading_the_last_frame() {
    let data = std::env::temp_dir().join(format!(
        "chda-slow-poll-{}",
        chda_pty::local::token().unwrap()
    ));
    std::fs::create_dir(&data).unwrap();
    let listener = chda_pty::local::Listener::bind(&crate::socket_path(&data)).unwrap();
    let connection = Connection::open(&data).unwrap();
    let mut peer = listener.accept().unwrap();
    let client = Client {
        data: data.clone(),
        connection: Arc::new(Mutex::new(Connection::open(&data).unwrap())),
        instance: Arc::new(Mutex::new(record().identity.server)),
        executable: Default::default(),
        layout: Default::default(),
        imports: Default::default(),
        group: String::new(),
        pending_executable: Default::default(),
        gui_available: Default::default(),
    };
    let _controller = listener.accept().unwrap();
    let (tx, events) = mpsc::channel();
    let remote = client
        .remote(
            connection,
            Response::Attached {
                record: record(),
                frame: Box::default(),
                sequence: 0,
            },
            false,
            tx,
            || {},
        )
        .unwrap();
    let (polled, polling) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let envelope: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(matches!(envelope.request, Request::Poll { .. }));
        let mut saved = record();
        saved.saved_at = Some(42);
        protocol::write(
            &mut peer,
            &Response::Update {
                record: saved,
                frame: None,
                events: vec![],
                sequence: 0,
            },
        )
        .unwrap();
        let envelope: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(matches!(envelope.request, Request::Poll { .. }));
        polled.send(()).unwrap();
        blocked.recv().unwrap();
        // Closing a deliberately delayed response also exercises teardown.
    });
    polling.recv_timeout(Duration::from_secs(2)).unwrap();
    let (done, ready) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let session = Session::Remote(remote);
        let frame = session.frame();
        for _ in 0..512 {
            session.write(b"queued".to_vec());
        }
        done.send(frame.generation).unwrap();
        session
    });
    let result = ready.recv_timeout(Duration::from_millis(200));
    release.send(()).unwrap();
    server.join().unwrap();
    drop(reader.join().unwrap());
    assert!(
        events.try_iter().any(|e| matches!(e,
        Event::PersistenceStatus { saved_at: Some(42), error: Some(error) }
            if error.contains("input queue is full"))),
        "backpressure must retain the last confirmed save time"
    );
    drop(client);
    drop(listener);
    std::fs::remove_dir_all(data).unwrap();
    assert!(result.is_ok(), "a network wait held the UI frame lock");
}

#[test]
fn interrupted_input_reattaches_without_replaying_uncertain_or_queued_actions() {
    let data = std::env::temp_dir().join(format!(
        "chda-reconnect-{}",
        chda_pty::local::token().unwrap()
    ));
    std::fs::create_dir(&data).unwrap();
    let listener = chda_pty::local::Listener::bind(&crate::socket_path(&data)).unwrap();
    let connection = Connection::open(&data).unwrap();
    let mut peer = listener.accept().unwrap();
    let client = Client {
        data: data.clone(),
        connection: Arc::new(Mutex::new(Connection::open(&data).unwrap())),
        instance: Arc::new(Mutex::new(record().identity.server)),
        executable: Default::default(),
        layout: Default::default(),
        imports: Default::default(),
        group: String::new(),
        pending_executable: Default::default(),
        gui_available: Default::default(),
    };
    let _controller = listener.accept().unwrap();
    let (tx, events) = mpsc::channel();
    let session = Session::Remote(
        client
            .remote(
                connection,
                Response::Attached {
                    record: record(),
                    frame: Box::default(),
                    sequence: 0,
                },
                false,
                tx,
                || {},
            )
            .unwrap(),
    );
    let (received, input) = mpsc::channel();
    let (interrupt, blocked) = mpsc::channel();
    let (attached, reattached) = mpsc::channel();
    let (done, complete) = mpsc::channel();
    let server = std::thread::spawn(move || {
        // The action reached the server; its acknowledgement will be lost.
        loop {
            let envelope: Envelope = protocol::read(&mut peer, true).unwrap();
            match envelope.request {
                Request::Input {
                    action: Action::Bytes(bytes),
                    ..
                } => {
                    assert_eq!(bytes, b"uncertain\n");
                    received.send(()).unwrap();
                    blocked.recv().unwrap();
                    break;
                }
                Request::Poll { .. } => protocol::write(
                    &mut peer,
                    &Response::Update {
                        record: record(),
                        frame: None,
                        events: vec![],
                        sequence: 0,
                    },
                )
                .unwrap(),
                _ => panic!("unexpected request before interruption"),
            }
        }
        drop(peer);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut peer = loop {
            match listener.accept() {
                Ok(peer) => break peer,
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::yield_now()
                }
                Err(e) => panic!("no reattachment: {e}"),
            }
        };
        peer.timeout(Duration::from_secs(3)).unwrap();
        let envelope: Envelope = protocol::read(&mut peer, true).unwrap();
        assert!(
            matches!(envelope.request, Request::Attach { identity } if identity == record().identity)
        );
        protocol::write(
            &mut peer,
            &Response::Attached {
                record: record(),
                frame: Box::default(),
                sequence: 42,
            },
        )
        .unwrap();
        attached.send(()).unwrap();
        loop {
            let envelope: Envelope = protocol::read(&mut peer, true).unwrap();
            match envelope.request {
                Request::Poll { sequence, .. } => {
                    assert_eq!(sequence, 42, "old effects must not be requested again");
                    protocol::write(
                        &mut peer,
                        &Response::Update {
                            record: record(),
                            frame: None,
                            events: vec![],
                            sequence: 42,
                        },
                    )
                    .unwrap();
                }
                Request::Input {
                    action: Action::Bytes(bytes),
                    ..
                } => {
                    assert_eq!(
                        bytes, b"after-reconnect\n",
                        "uncertain and queued input must be discarded"
                    );
                    protocol::write(&mut peer, &Response::Ok).unwrap();
                    done.send(()).unwrap();
                    break;
                }
                _ => panic!("unexpected request after reattachment"),
            }
        }
    });
    session.write(b"uncertain\n".to_vec());
    input.recv_timeout(Duration::from_secs(3)).unwrap();
    session.write(b"queued\n".to_vec());
    interrupt.send(()).unwrap();
    reattached.recv_timeout(Duration::from_secs(3)).unwrap();
    // Wait for the replacement frame/status publication, not just Attach's
    // bytes: input remains disabled until the complete frame is installed.
    loop {
        if matches!(
            events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::PersistenceStatus { error: None, .. }
        ) && let Session::Remote(remote) = &session
            && remote
                .signals
                .accepting
                .load(std::sync::atomic::Ordering::Acquire)
        {
            break;
        }
    }
    session.write(b"after-reconnect\n".to_vec());
    complete.recv_timeout(Duration::from_secs(3)).unwrap();
    drop(session);
    server.join().unwrap();
    drop(client);
    std::fs::remove_dir_all(data).unwrap();
}
