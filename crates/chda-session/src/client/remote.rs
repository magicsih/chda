//! A stalled or interrupted transport never owns the UI's frame lock.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct Signals {
    pub accepting: AtomicBool,
    pub stop: AtomicBool,
    events: mpsc::Sender<Event>,
    wake: Mutex<Box<dyn Fn() + Send>>,
}
impl Signals {
    pub fn emit(&self, event: Event) {
        if !self.stop.load(Ordering::Acquire) {
            let _ = self.events.send(event);
            (self.wake.lock().unwrap())();
        }
    }
}
struct Reader {
    connection: Connection,
    data: PathBuf,
    identity: Identity,
    pid: Option<u32>,
    view: Arc<Mutex<Arc<Frame>>>,
    signals: Arc<Signals>,
    rx: mpsc::Receiver<Action>,
    sequence: u64,
    status: (Option<u64>, Option<String>),
}
pub(super) fn start(
    client: &Client,
    connection: Connection,
    response: Response,
    reattached: bool,
    events: mpsc::Sender<Event>,
    wake: impl Fn() + Send + 'static,
) -> io::Result<RemoteSession> {
    let Response::Attached {
        record,
        mut frame,
        sequence,
    } = response
    else {
        return Err(io::Error::other("Invalid attach response"));
    };
    protocol::complete_images(&mut frame, &Frame::default())?;
    let frame = Arc::new(Mutex::new(Arc::new(*frame)));
    let (tx, rx) = mpsc::sync_channel(256);
    let signals = Arc::new(Signals {
        accepting: AtomicBool::new(true),
        stop: AtomicBool::new(false),
        events,
        wake: Mutex::new(Box::new(wake)),
    });
    let reader = Reader {
        connection,
        data: client.data.clone(),
        identity: record.identity.clone(),
        pid: record.pid,
        view: frame.clone(),
        signals: signals.clone(),
        rx,
        sequence,
        status: (record.saved_at, record.save_error.clone()),
    };
    let thread = std::thread::Builder::new()
        .name("chda-session-view".into())
        .spawn(move || reader.run())?;
    Ok(RemoteSession {
        reattached,
        record: Box::new(record),
        frame,
        signals,
        tx: Some(tx),
        thread: Some(thread),
        client: client.clone(),
    })
}
impl Reader {
    fn screen(&self) {
        let frame = self.view.lock().unwrap().clone();
        self.signals.emit(Event::Frame { has_output: false });
        self.signals.emit(Event::TitleChanged(frame.title.clone()));
        if !frame.pwd.is_empty() {
            self.signals.emit(Event::PwdChanged(frame.pwd.clone()));
        }
        if frame.at_prompt() {
            self.signals.emit(Event::PromptShown);
        }
        self.signals.emit(Event::PersistenceStatus {
            saved_at: self.status.0,
            error: self.status.1.clone(),
        });
    }
    fn run(mut self) {
        self.screen();
        loop {
            match self.poll() {
                Ok(false) => return,
                Ok(true) => {}
                Err(error) if unavailable(&error) => {
                    self.signals.accepting.store(false, Ordering::Release);
                    self.signals.emit(Event::PersistenceStatus {
                        saved_at: self.status.0,
                        error: Some("Session connection interrupted. Reconnecting; queued input is not replayed.".into()),
                    });
                    match self.reconnect() {
                        Ok(false) => return,
                        Ok(true) => {}
                        Err(error) => {
                            self.failed(error);
                            return;
                        }
                    }
                }
                Err(error) => {
                    self.failed(error);
                    return;
                }
            }
        }
    }
    fn failed(&self, error: io::Error) {
        self.signals.accepting.store(false, Ordering::Release);
        self.signals.emit(Event::Disconnected(error.to_string()));
    }
    fn poll(&mut self) -> io::Result<bool> {
        if !self.signals.accepting.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Input queue overflow",
            ));
        }
        match self.rx.recv_timeout(Duration::from_millis(8)) {
            Ok(action) => {
                for action in std::iter::once(action).chain(self.rx.try_iter()).take(256) {
                    if !self.signals.accepting.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "Input queue overflow",
                        ));
                    }
                    self.connection.request(Request::Input {
                        identity: self.identity.clone(),
                        action,
                    })?;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(false),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // A normal view close still drains input accepted before that close.
        // A failed transport instead discards it at the reconnect boundary.
        if self.signals.stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        let current = self.view.lock().unwrap().clone();
        let images = current
            .images
            .iter()
            .map(|p| (p.image.id, p.image.generation))
            .collect();
        let response = self.connection.request(Request::Poll {
            identity: self.identity.clone(),
            generation: current.generation,
            sequence: self.sequence,
            images,
        })?;
        let Response::Update {
            record,
            frame,
            events,
            sequence,
        } = response
        else {
            return Err(io::Error::other("Invalid terminal update"));
        };
        self.sequence = sequence;
        let status = (record.saved_at, record.save_error);
        if self.status != status {
            self.status = status;
            self.signals.emit(Event::PersistenceStatus {
                saved_at: self.status.0,
                error: self.status.1.clone(),
            });
        }
        if let Some(mut frame) = frame {
            protocol::complete_images(&mut frame, &current)?;
            *self.view.lock().unwrap() = Arc::new(*frame);
            self.signals.emit(Event::Frame { has_output: true });
        }
        for event in events {
            self.signals.emit(event);
        }
        Ok(true)
    }
    fn reconnect(&mut self) -> io::Result<bool> {
        loop {
            if self.signals.stop.load(Ordering::Acquire) {
                return Ok(false);
            }
            // An unacknowledged action may already have reached the PTY.
            // Discard both it and every action queued behind the failed call.
            while self.rx.try_recv().is_ok() {}
            let result = Connection::open(&self.data).and_then(|mut connection| {
                let response = connection.request(Request::Attach {
                    identity: self.identity.clone(),
                })?;
                Ok((connection, response))
            });
            match result {
                Ok((
                    connection,
                    Response::Attached {
                        record,
                        mut frame,
                        sequence,
                    },
                )) => {
                    if record.identity != self.identity || record.pid != self.pid || !record.running
                    {
                        return Err(io::Error::other(
                            "The original session ended. Reopen chda for explicit recovery; no command was restarted.",
                        ));
                    }
                    protocol::complete_images(&mut frame, &Frame::default())?;
                    self.connection = connection;
                    *self.view.lock().unwrap() = Arc::new(*frame);
                    self.sequence = sequence;
                    self.status = (record.saved_at, record.save_error);
                    while self.rx.try_recv().is_ok() {}
                    self.signals.accepting.store(true, Ordering::Release);
                    self.screen();
                    return Ok(true);
                }
                Ok(_) => return Err(io::Error::other("Invalid session reattachment")),
                Err(error) if unavailable(&error) => {}
                Err(error) => return Err(error),
            }
            if matches!(
                self.rx.recv_timeout(Duration::from_millis(250)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ) {
                return Ok(false);
            }
        }
    }
}
