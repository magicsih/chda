use crate::{
    PROTOCOL_VERSION,
    protocol::{self, Action, Envelope, Identity, Request, Response, SessionRecord, State},
};
use chda_term::{Event, Frame, SessionOptions};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

struct Connection(chda_pty::local::Stream);
impl Connection {
    fn open(data: &Path) -> io::Result<Self> {
        let s = chda_pty::local::Stream::connect(&crate::socket_path(data))?;
        s.timeout(Duration::from_secs(5))?;
        Ok(Self(s))
    }
    fn request(&mut self, request: Request) -> io::Result<Response> {
        protocol::write(
            &mut self.0,
            &Envelope {
                version: PROTOCOL_VERSION,
                request,
            },
        )?;
        match protocol::read(&mut self.0, false)? {
            Response::Error(e) => Err(io::Error::other(e)),
            response => Ok(response),
        }
    }
}

/// A GUI lease. Keeping it alive prevents an empty server from exiting while
/// a window is open; a failed handshake never kills or replaces a server.
#[derive(Clone)]
pub struct Client {
    data: PathBuf,
    connection: Arc<Mutex<Connection>>,
}

pub struct Watch {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl Client {
    pub fn helper_executable(&self) -> io::Result<PathBuf> {
        Ok(self.state()?.executable)
    }
    pub fn connect(data: &Path) -> io::Result<Self> {
        let mut connection = Connection::open(data)?;
        if !matches!(connection.request(Request::Hello)?, Response::State(_)) {
            return Err(io::Error::other("Invalid session handshake"));
        }
        Ok(Self {
            data: data.to_owned(),
            connection: Arc::new(Mutex::new(connection)),
        })
    }
    /// Start only when there is no listener. An incompatible or unresponsive
    /// existing listener is an error, never authority to kill a running server.
    pub fn connect_or_start(data: &Path, executable: &Path) -> io::Result<Self> {
        match Connection::open(data) {
            Ok(mut connection) => {
                if matches!(connection.request(Request::Hello)?, Response::State(_)) {
                    return Ok(Self {
                        data: data.to_owned(),
                        connection: Arc::new(Mutex::new(connection)),
                    });
                }
                return Err(io::Error::other("Invalid session handshake"));
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) => {}
            Err(e) => return Err(e),
        }
        let mut child = chda_pty::local::spawn_detached(
            std::process::Command::new(executable)
                .arg("serve")
                .arg(data),
        )?;
        // Reap only the server we started after it eventually exits.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match Self::connect(data) {
                Ok(client) => return Ok(client),
                Err(e) if std::time::Instant::now() >= deadline => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
    fn request(&self, request: Request) -> io::Result<Response> {
        self.connection
            .lock()
            .map_err(|_| io::Error::other("Session connection lock failed"))?
            .request(request)
    }
    pub fn state(&self) -> io::Result<State> {
        match self.request(Request::List)? {
            Response::State(s) => Ok(s),
            _ => Err(io::Error::other("Invalid session state")),
        }
    }
    pub fn set_gui_available(&self, available: bool) -> io::Result<()> {
        self.request(Request::GuiPresence(available))?;
        Ok(())
    }
    pub fn reserve_ids(&self) -> io::Result<u64> {
        match self.request(Request::ReserveIds)? {
            Response::IdRange(first) => Ok(first),
            _ => Err(io::Error::other("Invalid pane ID range")),
        }
    }
    pub fn checkpoint(&self, layout: Option<chda_core::SavedSession>) -> io::Result<State> {
        match self.request(Request::Checkpoint { layout })? {
            Response::State(s) => Ok(s),
            _ => Err(io::Error::other("Invalid checkpoint response")),
        }
    }
    pub fn output(&self, identity: Identity) -> io::Result<String> {
        match self.request(Request::Output { identity })? {
            Response::Output(t) => Ok(t),
            _ => Err(io::Error::other("Invalid output response")),
        }
    }
    pub fn terminate(&self, identity: Identity) -> io::Result<()> {
        self.request(Request::Terminate { identity })?;
        Ok(())
    }
    pub fn create(
        &self,
        pane: u64,
        options: SessionOptions,
        events: mpsc::Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<RemoteSession> {
        self.open(Request::Create { pane, options }, events, wake)
    }
    pub fn attach(
        &self,
        identity: Identity,
        events: mpsc::Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<RemoteSession> {
        self.open(Request::Attach { identity }, events, wake)
    }
    pub fn resume(
        &self,
        identity: Identity,
        options: SessionOptions,
        events: mpsc::Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<RemoteSession> {
        self.open(Request::Resume { identity, options }, events, wake)
    }
    pub fn pane(
        &self,
        pane: u64,
        options: SessionOptions,
        events: mpsc::Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Session> {
        let record = self.state()?.sessions.into_iter().find(|s| s.pane == pane);
        let remote = match record {
            Some(r) if r.running => self.attach(r.identity, events, wake),
            Some(r) => self.resume(r.identity, options, events, wake),
            None => self.create(pane, options, events, wake),
        }?;
        Ok(Session::Remote(remote))
    }
    fn open(
        &self,
        request: Request,
        events: mpsc::Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<RemoteSession> {
        let reattached = matches!(request, Request::Attach { .. });
        let mut connection = Connection::open(&self.data)?;
        let Response::Attached {
            record,
            mut frame,
            mut sequence,
        } = connection.request(request)?
        else {
            return Err(io::Error::other("Invalid attach response"));
        };
        protocol::complete_images(&mut frame, &Frame::default())?;
        let slot = Arc::new(Mutex::new(Arc::new(*frame)));
        let (tx, rx) = mpsc::sync_channel(256);
        let identity = record.identity.clone();
        let initial_status = (record.saved_at, record.save_error.clone());
        let view = slot.clone();
        let thread = std::thread::Builder::new()
            .name("chda-session-view".into())
            .spawn(move || {
                let emit = |e| {
                    let _ = events.send(e);
                    wake();
                };
                emit(Event::Frame { has_output: false });
                // These describe the current screen, unlike one-shot effects.
                // A fast shell can reach its prompt before Attach's barrier.
                let initial = view.lock().unwrap().clone();
                emit(Event::TitleChanged(initial.title.clone()));
                if !initial.pwd.is_empty() {
                    emit(Event::PwdChanged(initial.pwd.clone()));
                }
                if initial.at_prompt() {
                    emit(Event::PromptShown);
                }
                let mut status = initial_status;
                emit(Event::PersistenceStatus {
                    saved_at: status.0,
                    error: status.1.clone(),
                });
                let mut generation = view.lock().unwrap().generation;
                let result = (|| -> io::Result<()> {
                    loop {
                        match rx.recv_timeout(Duration::from_millis(8)) {
                            Ok(action) => {
                                for action in std::iter::once(action).chain(rx.try_iter()).take(256)
                                {
                                    connection.request(Request::Input {
                                        identity: identity.clone(),
                                        action,
                                    })?;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                let _ = connection.request(Request::Detach);
                                return Ok(());
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                        let response = connection.request(Request::Poll {
                            identity: identity.clone(),
                            generation,
                            sequence,
                            images: view
                                .lock()
                                .unwrap()
                                .images
                                .iter()
                                .map(|p| (p.image.id, p.image.generation))
                                .collect(),
                        })?;
                        let Response::Update {
                            record,
                            frame,
                            events,
                            sequence: next,
                            ..
                        } = response
                        else {
                            return Err(io::Error::other("Invalid terminal update"));
                        };
                        sequence = next;
                        let next_status = (record.saved_at, record.save_error);
                        if status != next_status {
                            status = next_status;
                            emit(Event::PersistenceStatus {
                                saved_at: status.0,
                                error: status.1.clone(),
                            });
                        }
                        if let Some(mut frame) = frame {
                            protocol::complete_images(&mut frame, &view.lock().unwrap())?;
                            generation = frame.generation;
                            *view.lock().unwrap() = Arc::new(*frame);
                            emit(Event::Frame { has_output: true });
                        }
                        for event in events {
                            emit(event);
                        }
                    }
                })();
                if let Err(e) = result {
                    emit(Event::Disconnected(e.to_string()));
                }
            })?;
        Ok(RemoteSession {
            reattached,
            record: Box::new(record),
            frame: slot,
            tx: Some(tx),
            thread: Some(thread),
            client: self.clone(),
        })
    }
    /// Relay server-validated metadata and GUI-only requests to the existing
    /// app event dispatcher. Current agent states are returned separately by
    /// State, so a fresh connection need not replay old notifications.
    pub fn watch(
        &self,
        tx: mpsc::Sender<chda_agents::control::Incoming>,
        wake: impl Fn() + Send + 'static,
    ) -> Watch {
        let client = self.clone();
        let initial = self.state().map(|s| s.sequence).unwrap_or(u64::MAX);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut seq = initial;
            while !stopping.load(std::sync::atomic::Ordering::Acquire) {
                let response = client.request(Request::HookEvents { sequence: seq });
                let Ok(Response::Hooks {
                    events,
                    quotas,
                    requests,
                    sequence,
                }) = response
                else {
                    break;
                };
                seq = sequence;
                let changed = !events.is_empty() || !quotas.is_empty() || !requests.is_empty();
                for e in events {
                    if tx.send(chda_agents::control::Incoming::Event(e)).is_err() {
                        return;
                    }
                }
                for q in quotas {
                    if tx.send(chda_agents::control::Incoming::Quota(q)).is_err() {
                        return;
                    }
                }
                for (id, r) in requests {
                    let (reply_tx, rx) = mpsc::channel();
                    if tx
                        .send(chda_agents::control::Incoming::Request(r, reply_tx))
                        .is_err()
                    {
                        return;
                    }
                    let client = client.clone();
                    std::thread::spawn(move || {
                        if let Ok(reply) = rx.recv_timeout(Duration::from_secs(55)) {
                            let _ = client.request(Request::GuiReply { id, reply });
                        }
                    });
                }
                if changed {
                    wake();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Watch {
            stop,
            thread: Some(thread),
        }
    }
}

pub struct RemoteSession {
    reattached: bool,
    pub record: Box<SessionRecord>,
    frame: Arc<Mutex<Arc<Frame>>>,
    tx: Option<mpsc::SyncSender<Action>>,
    thread: Option<std::thread::JoinHandle<()>>,
    client: Client,
}
impl Drop for RemoteSession {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Local sessions remain the Linux/Windows and isolated scenario backend.
/// A remote handle's Drop disconnects; Terminate is always explicit.
pub enum Session {
    Local(chda_term::Session),
    Remote(RemoteSession),
}
impl Session {
    pub fn reattached(&self) -> bool {
        matches!(self, Self::Remote(s) if s.reattached)
    }
    pub fn spawn(
        options: SessionOptions,
        events: mpsc::Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        chda_term::Session::spawn(options, events, wake).map(Self::Local)
    }
    pub fn prepare_adoption(
        options: SessionOptions,
        detached: chda_term::DetachedSession,
        events: mpsc::Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        chda_term::Session::prepare_adoption(options, detached, events, wake).map(Self::Local)
    }
    pub fn frame(&self) -> Arc<Frame> {
        match self {
            Self::Local(s) => s.frame(),
            Self::Remote(s) => s.frame.lock().unwrap().clone(),
        }
    }
    pub fn child_pid(&self) -> Option<u32> {
        match self {
            Self::Local(s) => s.child_pid(),
            Self::Remote(s) => s.record.pid,
        }
    }
    pub fn terminate(&self) {
        if let Self::Remote(s) = self {
            let _ = s.client.terminate(s.record.identity.clone());
        }
    }
    pub fn commit_adoption(&self) {
        if let Self::Local(s) = self {
            s.commit_adoption();
        }
    }
    pub fn detach(&self) -> mpsc::Receiver<io::Result<chda_term::DetachedSession>> {
        match self {
            Self::Local(s) => s.detach(),
            Self::Remote(_) => {
                let (tx, rx) = mpsc::channel();
                let _ = tx.send(Err(io::Error::other(
                    "Server-owned terminals reconnect without a PTY handoff",
                )));
                rx
            }
        }
    }
    fn action(&self, action: Action) {
        match self {
            Self::Local(s) => action.apply(s),
            Self::Remote(s) => {
                if let Some(tx) = &s.tx {
                    let _ = tx.send(action);
                }
            }
        }
    }
    pub fn key(&self, i: chda_term::KeyInput) {
        self.action(Action::Key(i));
    }
    pub fn paste(&self, t: String) {
        self.action(Action::Paste(t));
    }
    pub fn text(&self, t: String) {
        self.action(Action::Text(t));
    }
    pub fn write(&self, b: Vec<u8>) {
        self.action(Action::Bytes(b));
    }
    pub fn focus(&self, f: bool) {
        self.action(Action::Focus(f));
    }
    pub fn resize(&self, size: chda_term::Size, cell_width_px: u32, cell_height_px: u32) {
        self.action(Action::Resize {
            size,
            cell_width_px,
            cell_height_px,
        });
    }
    pub fn scroll(&self, lines: isize, mods: chda_term::Modifiers, px_x: u32, px_y: u32) {
        self.action(Action::Scroll {
            lines,
            mods,
            px_x,
            px_y,
        });
    }
    pub fn mouse(&self, i: chda_term::MouseInput) {
        self.action(Action::Mouse(i));
    }
    pub fn copy_selection(&self) {
        self.action(Action::CopySelection);
    }
    pub fn read_selection(&self, r: u64) {
        self.action(Action::ReadSelection(r));
    }
    pub fn find_last_diagram(&self) {
        self.action(Action::FindLastDiagram);
    }
    pub fn query_cwd(&self) {
        self.action(Action::QueryCwd);
    }
    pub fn search(&self, q: Option<chda_term::SearchQuery>) {
        self.action(Action::Search(q));
    }
    pub fn search_step(&self, o: bool) {
        self.action(Action::SearchStep(o));
    }
    pub fn set_colors(&self, c: chda_term::ColorConfig) {
        self.action(Action::SetColors(c));
    }
    pub fn scroll_to_bottom(&self) {
        self.action(Action::ScrollToBottom);
    }
    pub fn jump_to_last_prompt(&self) {
        self.action(Action::JumpToLastPrompt);
    }
    pub fn jump_to_prompt(&self, d: i32) {
        self.action(Action::JumpToPrompt(d));
    }
}
