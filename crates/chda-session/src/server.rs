use crate::{
    protocol::{self, Envelope, Identity, Request, Response, SessionRecord, State},
    store::{Checkpoint, Store},
};
use chda_agents::{
    HookEvent, HookKind,
    control::{Incoming, Reply, Request as Control},
};
use chda_term::{Event, Session, SessionOptions};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
mod adoption;
mod persistence;

struct Importing {
    group: String,
    previous: Option<Box<Running>>,
    deferred: Vec<crate::Action>,
}

struct Running {
    importing: Option<Importing>,
    checkpoint: Checkpoint,
    terminal: Option<Session>,
    events: mpsc::Receiver<Event>,
    sequence: u64,
    pending: VecDeque<(u64, Event)>,
    dirty: bool,
    revision: u64,
    last_save: Instant,
}
struct Runtime {
    retiring: bool,
    retire_committed: bool,
    committing: BTreeSet<String>,
    legacy_pending: VecDeque<HookEvent>,
    children: chda_agents::ChildBoard,
    instance: String,
    persistence: persistence::Persistence,
    sessions: BTreeMap<String, Running>,
    layout: Option<chda_core::SavedSession>,
    save_error: Option<String>,
    layout_saved_at: Option<u64>,
    connections: usize,
    gui_connections: usize,
    next_pane: u64,
    sequence: u64,
    hooks: VecDeque<(u64, HookEvent)>,
    quotas: VecDeque<(u64, chda_agents::quota::QuotaSnapshot)>,
    requests: BTreeMap<u64, (Control, mpsc::Sender<Reply>, Instant)>,
}
impl Runtime {
    fn state(&self) -> State {
        State {
            pid: std::process::id(),
            sequence: self.sequence,
            children: self.children.clone(),
            executable: std::env::current_exe().unwrap_or_default(),
            instance: self.instance.clone(),
            executable_version: env!("CARGO_PKG_VERSION").into(),
            layout: self.layout.clone(),
            sessions: self
                .sessions
                .values()
                .map(|s| s.checkpoint.record.clone())
                .collect(),
            save_error: self.save_error.clone(),
            layout_saved_at: self.layout_saved_at,
        }
    }
    fn get(&mut self, id: &Identity) -> io::Result<&mut Running> {
        self.sessions
            .get_mut(&id.session)
            .filter(|s| s.checkpoint.record.identity == *id)
            .ok_or_else(|| io::Error::other("This session belongs to an earlier run or server"))
    }
    fn start(
        &mut self,
        pane: u64,
        mut options: SessionOptions,
        previous: Option<Identity>,
    ) -> io::Result<SessionRecord> {
        if self.retiring {
            return Err(io::Error::other(
                "The idle service is switching versions; try again",
            ));
        }
        if pane == 0 || pane >= u64::MAX - 4096 {
            return Err(io::Error::other("Invalid pane ID"));
        }
        let identity = Identity {
            session: previous
                .as_ref()
                .map(|p| p.session.clone())
                .unwrap_or(chda_pty::local::token()?),
            run: chda_pty::local::token()?,
            server: self.instance.clone(),
        };
        if let Some(id) = &previous {
            if self.get(id)?.checkpoint.record.running {
                return Err(io::Error::other("The session is already running"));
            }
        } else if self
            .sessions
            .values()
            .any(|s| s.checkpoint.record.pane == pane)
        {
            return Err(io::Error::other("The pane ID is already in use"));
        }
        if options.size.cols == 0
            || options.size.rows == 0
            || options.size.cols > 1000
            || options.size.rows > 1000
        {
            return Err(io::Error::other("Invalid terminal size"));
        }
        // Only extra variables for this new process are transmitted. They are
        // never copied into a checkpoint, log, or response.
        options
            .env
            .retain(|(k, _)| !matches!(k.as_str(), "CHDA_RUN_ID" | "CHDA_SERVER_ID"));
        options.env.extend([
            ("CHDA_RUN_ID".into(), identity.run.clone()),
            ("CHDA_SERVER_ID".into(), identity.server.clone()),
        ]);
        let (tx, events) = mpsc::channel();
        let terminal = Session::spawn(options.clone(), tx, || {})?;
        let record = SessionRecord {
            pane,
            identity: identity.clone(),
            pid: terminal.child_pid(),
            running: true,
            closed: false,
            title: String::new(),
            cwd: options.cwd,
            // The GUI checkpoints the chosen launch context for this run.
            // A manually started shell must not inherit an old AI launch.
            launch: None,
            agent: None,
            exit: None,
            saved_at: None,
            save_error: None,
            scrollback: options.scrollback,
            legacy_hooks: false,
        };
        let output = previous
            .as_ref()
            .and_then(|id| self.sessions.get(&id.session))
            .map(|s| s.checkpoint.output.clone())
            .unwrap_or_default();
        self.sessions.insert(
            identity.session.clone(),
            Running {
                importing: None,
                checkpoint: Checkpoint {
                    record: record.clone(),
                    output,
                },
                terminal: Some(terminal),
                events,
                sequence: 0,
                pending: VecDeque::new(),
                dirty: true,
                revision: 1,
                last_save: Instant::now() - Duration::from_secs(5),
            },
        );
        self.flush(&identity.session, false);
        Ok(record)
    }
    fn flush(&mut self, id: &str, capture: bool) {
        let Some(s) = self.sessions.get_mut(id) else {
            return;
        };
        if s.importing.is_some() {
            return;
        }
        let text = if !s.checkpoint.record.running {
            persistence::Text::Final(s.checkpoint.output.clone())
        } else if capture {
            match &s.terminal {
                Some(t) => persistence::Text::Capture(t.checkpoint()),
                None => return,
            }
        } else {
            persistence::Text::Retain
        };
        self.persistence
            .session(s.checkpoint.record.clone(), text, s.revision);
        s.last_save = Instant::now();
    }
    fn tick(&mut self) {
        self.saved();
        let mut flush = Vec::new();
        let mut title_hooks = Vec::new();
        for (id, s) in &mut self.sessions {
            for event in s.events.try_iter() {
                s.revision = s.revision.wrapping_add(1);
                match &event {
                    Event::Frame { has_output: true } => s.dirty = true,
                    Event::TitleChanged(t) => {
                        if let Some(kind) = codex_title_hook(&s.checkpoint.record, Some(t)) {
                            title_hooks.push((s.checkpoint.record.clone(), kind));
                        }
                        s.checkpoint.record.title = t.clone();
                        s.dirty = true;
                    }
                    Event::PwdChanged(p) => {
                        if let Some(p) = chda_term::parse_pwd_report(p) {
                            s.checkpoint.record.cwd = Some(p);
                        }
                        s.dirty = true;
                    }
                    Event::Cwd(Some(p)) => {
                        s.checkpoint.record.cwd = Some(p.clone());
                    }
                    Event::PromptShown => {
                        if let Some(kind) = codex_title_hook(&s.checkpoint.record, None) {
                            title_hooks.push((s.checkpoint.record.clone(), kind));
                        }
                    }
                    Event::FinalOutput(Ok(text)) => s.checkpoint.output = text.clone(),
                    Event::Exited(status) => {
                        s.checkpoint.record.running = false;
                        s.checkpoint.record.exit = status.clone();
                        s.dirty = true;
                        flush.push((id.clone(), false));
                    }
                    _ => {}
                }
                // Screen notifications coalesce in the frame slot. Effects
                // have a bounded sequence and are never replayed on Attach.
                if !matches!(event, Event::Frame { .. }) {
                    s.sequence += 1;
                    s.pending.push_back((s.sequence, event));
                    if s.pending.len() > 256 {
                        s.pending.pop_front();
                    }
                }
            }
            if s.importing.is_none() && s.dirty && s.last_save.elapsed() >= Duration::from_secs(5) {
                flush.push((id.clone(), s.checkpoint.record.running));
            }
        }
        flush.sort();
        flush.dedup();
        for (id, capture) in flush {
            self.flush(&id, capture);
        }
        for (record, kind) in title_hooks {
            if let Some(cwd) = record.cwd {
                let session_id = record
                    .agent
                    .filter(|e| e.agent == "codex")
                    .map(|e| e.session_id)
                    .or_else(|| {
                        record
                            .launch
                            .filter(|l| l.agent == chda_agents::AgentId::Codex)
                            .and_then(|l| l.session.map(|s| s.0))
                    })
                    .unwrap_or_default();
                self.hook(HookEvent {
                    agent: "codex".into(),
                    session_id,
                    cwd,
                    kind,
                    timestamp: protocol::now(),
                    pane: Some(record.pane),
                    child: None,
                    run: Some(record.identity.run),
                });
            }
        }
        let expired: Vec<_> = self
            .requests
            .iter()
            .filter(|(_, (_, _, at))| at.elapsed() > Duration::from_secs(55))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some((_, tx, _)) = self.requests.remove(&id) {
                let _ = tx.send(Reply::err("No GUI answered the request"));
            }
        }
    }
    fn saved(&mut self) {
        for saved in self.persistence.completed.try_iter() {
            match saved {
                persistence::Saved::Layout { saved_at, error } => {
                    if saved_at.is_some() {
                        self.layout_saved_at = saved_at;
                    }
                    self.save_error = error;
                }
                persistence::Saved::Session {
                    identity,
                    output,
                    captured_revision,
                    saved_at,
                    error,
                } => {
                    let Some(s) = self
                        .sessions
                        .get_mut(&identity.session)
                        .filter(|s| s.checkpoint.record.identity == identity)
                    else {
                        continue;
                    };
                    if let Some(output) = output {
                        s.checkpoint.output = output;
                    }
                    if error.is_none() && captured_revision == Some(s.revision) {
                        s.dirty = false;
                    }
                    s.checkpoint.record.saved_at = saved_at;
                    s.checkpoint.record.save_error = error;
                }
            }
        }
    }
    fn hook(&mut self, event: HookEvent) {
        let Some(pane) = event.pane else {
            return;
        };
        let Some(s) = self
            .sessions
            .values_mut()
            .find(|s| s.checkpoint.record.pane == pane && s.checkpoint.record.running)
        else {
            if event.run.is_none() {
                self.legacy_pending.push_back(event);
                if self.legacy_pending.len() > 4096 {
                    self.legacy_pending.pop_front();
                }
            }
            return;
        };
        if event.run.as_deref() != Some(s.checkpoint.record.identity.run.as_str())
            && !(event.run.is_none() && s.checkpoint.record.legacy_hooks)
        {
            return;
        }
        if s.checkpoint
            .record
            .agent
            .as_ref()
            .is_some_and(|old| old.timestamp > event.timestamp)
        {
            return;
        }
        if event.child.is_none() {
            s.checkpoint.record.agent = Some(event.clone());
            if let Some(context) = &mut s.checkpoint.record.launch
                && context.agent.as_str() == event.agent
            {
                context.session = Some(chda_agents::SessionId(event.session_id.clone()));
                context.cwd = event.cwd.clone();
            }
            s.dirty = true;
            s.revision = s.revision.wrapping_add(1);
        }
        let id = s.checkpoint.record.identity.session.clone();
        if let Some(child) = &event.child {
            self.children.apply(chda_agents::ChildActivity {
                agent: event.agent.clone(),
                parent: event.session_id.clone(),
                child: child.clone(),
                observed_at: event.timestamp,
            });
        }
        self.flush(&id, false);
        self.sequence += 1;
        self.hooks.push_back((self.sequence, event));
        if self.hooks.len() > 4096 {
            self.hooks.pop_front();
        }
    }
    fn retire_if_idle(&mut self, requester_is_non_gui: bool) -> Response {
        let mut idle = !self.sessions.values().any(|s| s.checkpoint.record.running)
            && self.connections == 1
            && self.connections == self.gui_connections + usize::from(requester_is_non_gui)
            && self.committing.is_empty();
        if idle {
            let ids: Vec<_> = self
                .sessions
                .iter()
                .filter(|(_, s)| s.dirty)
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                self.flush(&id, true);
            }
            let settled = self.persistence.idle();
            if settled {
                self.saved();
            }
            idle = settled
                && self.save_error.is_none()
                && self
                    .sessions
                    .values()
                    .all(|s| s.checkpoint.record.save_error.is_none());
            self.retiring = idle;
        }
        Response::Retired(idle)
    }
    fn dispatch(&mut self, request: Request) -> io::Result<Response> {
        match request {
            Request::RetireIfIdle => Ok(Response::Retired(false)),
            Request::PrepareImport { .. } | Request::CommitImports { .. } => {
                Err(io::Error::other("Import requires descriptor transport"))
            }
            Request::GuiPresence(_) => Ok(Response::Ok),
            Request::ReserveIds => {
                let first = self.next_pane;
                self.next_pane = first
                    .checked_add(1 << 32)
                    .ok_or_else(|| io::Error::other("Pane ID space exhausted"))?;
                Ok(Response::IdRange(first))
            }
            Request::Hello | Request::List => Ok(Response::State(self.state())),
            Request::Create { pane, options } => {
                let record = self.start(pane, options, None)?;
                self.attached(&record.identity)
            }
            Request::Resume { identity, options } => {
                let pane = self.get(&identity)?.checkpoint.record.pane;
                let record = self.start(pane, options, Some(identity))?;
                self.attached(&record.identity)
            }
            Request::Attach { identity } => self.attached(&identity),
            Request::Input { identity, action } => {
                let s = self.get(&identity)?;
                if let Some(importing) = &mut s.importing {
                    match &action {
                        crate::Action::Resize { .. } => importing
                            .deferred
                            .retain(|a| !matches!(a, crate::Action::Resize { .. })),
                        crate::Action::SetColors(_) => importing
                            .deferred
                            .retain(|a| !matches!(a, crate::Action::SetColors(_))),
                        crate::Action::Focus(_) => importing
                            .deferred
                            .retain(|a| !matches!(a, crate::Action::Focus(_))),
                        _ => return Ok(Response::Ok),
                    }
                    importing.deferred.push(action);
                    return Ok(Response::Ok);
                }
                if !s.checkpoint.record.running {
                    return Err(io::Error::other("Previous output is read-only"));
                }
                action.apply(
                    s.terminal
                        .as_ref()
                        .ok_or_else(|| io::Error::other("No running terminal"))?,
                );
                Ok(Response::Ok)
            }
            Request::Poll {
                identity,
                generation,
                sequence,
                images,
            } => {
                self.tick();
                let s = self.get(&identity)?;
                let frame = s
                    .terminal
                    .as_ref()
                    .map(Session::frame)
                    .filter(|f| f.generation != generation)
                    .map(|f| Box::new(protocol::frame_for_wire((*f).clone(), &images)));
                let events = s
                    .pending
                    .iter()
                    .filter(|(seq, _)| *seq > sequence)
                    .map(|(_, e)| e.clone())
                    .collect();
                Ok(Response::Update {
                    record: s.checkpoint.record.clone(),
                    frame,
                    events,
                    sequence: s.sequence,
                })
            }
            Request::Detach => Ok(Response::Ok),
            Request::Terminate { identity } => {
                let s = self.get(&identity)?;
                if s.importing.is_some() {
                    return Err(io::Error::other("The predecessor still owns this terminal"));
                }
                s.checkpoint.record.closed = true;
                if let Some(t) = s.terminal.take() {
                    drop(t);
                }
                self.tick();
                self.flush(&identity.session, false);
                Ok(Response::Ok)
            }
            Request::Checkpoint { layout } => {
                let capture = layout.is_none();
                if let Some(layout) = layout {
                    self.update_launches(&layout);
                    self.layout = Some(layout.clone());
                    self.persistence.layout(layout);
                }
                let ids: Vec<_> = self.sessions.keys().cloned().collect();
                for id in ids {
                    if capture || self.sessions[&id].dirty {
                        self.flush(&id, capture);
                    }
                }
                Ok(Response::State(self.state()))
            }
            Request::Output { identity } => {
                let s = self.get(&identity)?;
                Ok(Response::Output(s.checkpoint.output.clone()))
            }
            Request::HookEvents { sequence } => Ok(Response::Hooks {
                events: self
                    .hooks
                    .iter()
                    .filter(|(s, _)| *s > sequence)
                    .map(|(_, e)| e.clone())
                    .collect(),
                quotas: self
                    .quotas
                    .iter()
                    .filter(|(s, _)| *s > sequence)
                    .map(|(_, q)| q.clone())
                    .collect(),
                requests: self
                    .requests
                    .iter()
                    .filter(|(s, _)| **s > sequence)
                    .map(|(id, (r, _, _))| (*id, r.clone()))
                    .collect(),
                sequence: self.sequence,
            }),
            Request::GuiReply { id, reply } => {
                if let Some((_, tx, _)) = self.requests.remove(&id) {
                    let _ = tx.send(reply);
                }
                Ok(Response::Ok)
            }
        }
    }
    fn attached(&mut self, identity: &Identity) -> io::Result<Response> {
        if let Some(t) = &self.get(identity)?.terminal {
            t.barrier()
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| io::Error::other("Terminal attach barrier timed out"))?;
        }
        self.tick();
        let s = self.get(identity)?;
        if !s.checkpoint.record.running {
            return Err(io::Error::other("The process ended; use its saved output"));
        }
        let frame = Box::new(protocol::frame_for_wire(
            (*s.terminal.as_ref().unwrap().frame()).clone(),
            &[],
        ));
        Ok(Response::Attached {
            record: s.checkpoint.record.clone(),
            frame,
            sequence: s.sequence,
        })
    }
    fn update_launches(&mut self, layout: &chda_core::SavedSession) {
        fn visit(node: &chda_core::SavedNode, sessions: &mut BTreeMap<String, Running>) {
            match node {
                chda_core::SavedNode::Pane {
                    pane: Some(pane),
                    launch,
                    ..
                } => {
                    if let Some(s) = sessions
                        .values_mut()
                        .find(|s| s.checkpoint.record.pane == *pane)
                        && s.checkpoint.record.launch != launch.as_deref().cloned()
                    {
                        s.checkpoint.record.launch = launch.as_deref().cloned();
                        s.dirty = true;
                        s.revision = s.revision.wrapping_add(1);
                    }
                }
                chda_core::SavedNode::Split { first, second, .. } => {
                    visit(first, sessions);
                    visit(second, sessions);
                }
                _ => {}
            }
        }
        for w in &layout.windows {
            for t in &w.tabs {
                if let chda_core::SavedTabContent::Terminal { root, .. } = &t.content {
                    visit(root, &mut self.sessions);
                }
            }
        }
    }
}

fn codex_title_hook(record: &SessionRecord, title: Option<&String>) -> Option<HookKind> {
    use chda_agents::{CodexRunState as C, parse_codex_title};
    let old = parse_codex_title(&record.title);
    match title.map(String::as_str) {
        None | Some("") => {
            let codex = old.is_some()
                || record
                    .agent
                    .as_ref()
                    .is_some_and(|e| e.agent == "codex" && e.kind != HookKind::SessionEnd);
            codex.then_some(HookKind::SessionEnd)
        }
        Some(t) => {
            let next = parse_codex_title(t)?;
            if old == Some(next)
                && match next {
                    C::Working => record.agent.as_ref().is_some_and(|e| {
                        matches!(e.kind, HookKind::SessionStart | HookKind::PromptSubmitted)
                    }),
                    C::WaitingInput => record
                        .agent
                        .as_ref()
                        .is_some_and(|e| e.kind == HookKind::WaitingInput),
                    C::Ready => true,
                }
            {
                return None;
            }
            Some(match next {
                C::Working => HookKind::PromptSubmitted,
                C::WaitingInput => HookKind::WaitingInput,
                C::Ready if matches!(old, Some(C::Working | C::WaitingInput)) => HookKind::Stopped,
                C::Ready => HookKind::Idle,
            })
        }
    }
}

/// Run until no process and no connection remains. Loading checkpoints never
/// starts a process. The caller must use a separate process for real ownership.
pub fn serve(data: &Path) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let retained = data
        .join("session-service/bin")
        .canonicalize()
        .unwrap_or_else(|_| data.join("session-service/bin"));
    let _executable_lease = if executable.starts_with(&retained) {
        chda_pty::local::file_lease(&executable.with_file_name("server.lock"), true)?
    } else {
        None
    };
    let listener = chda_pty::local::Listener::bind(&crate::socket_path(data))?;
    let store = Store::new(data)?;
    let layout = store.migrate(data)?;
    let layout_saved_at = store.layout_saved_at();
    let sessions: BTreeMap<String, Running> = store
        .load()
        .into_iter()
        .map(|c| {
            (
                c.record.identity.session.clone(),
                Running {
                    importing: None,
                    checkpoint: c,
                    terminal: None,
                    events: mpsc::channel().1,
                    sequence: 0,
                    pending: VecDeque::new(),
                    dirty: false,
                    revision: 0,
                    last_save: Instant::now(),
                },
            )
        })
        .collect();
    let next_pane = sessions
        .values()
        .map(|s| s.checkpoint.record.pane)
        .max()
        .unwrap_or(0)
        + 1;
    let runtime = Arc::new(Mutex::new(Runtime {
        retiring: false,
        retire_committed: false,
        committing: BTreeSet::new(),
        legacy_pending: VecDeque::new(),
        children: Default::default(),
        instance: chda_pty::local::token()?,
        persistence: persistence::Persistence::new(store),
        sessions,
        layout,
        save_error: None,
        layout_saved_at,
        connections: 0,
        gui_connections: 0,
        next_pane,
        sequence: 0,
        hooks: VecDeque::new(),
        quotas: VecDeque::new(),
        requests: BTreeMap::new(),
    }));
    let (incoming_tx, incoming) = mpsc::channel();
    let _hooks = chda_agents::ipc::serve(
        &chda_agents::ipc::socket_path(data),
        incoming_tx.clone(),
        || {},
    )?;
    let started = Instant::now();
    let mut accepted = false;
    let mut idle = Instant::now();
    let mut fallback = Instant::now() - Duration::from_secs(1);
    loop {
        if fallback.elapsed() >= Duration::from_secs(1) {
            if let Ok(text) = chda_agents::ipc::take_fallback(data) {
                for line in text.lines() {
                    if let Ok(event) = serde_json::from_str::<HookEvent>(line) {
                        let _ = incoming_tx.send(Incoming::Event(event));
                    } else if let Ok(quota) =
                        serde_json::from_str::<chda_agents::quota::QuotaSnapshot>(line)
                    {
                        let _ = incoming_tx.send(Incoming::Quota(quota));
                    }
                }
            }
            fallback = Instant::now();
        }
        match listener.accept() {
            Ok(stream) => {
                let mut r = runtime.lock().unwrap();
                if r.connections >= 256 {
                    continue;
                }
                r.connections += 1;
                accepted = true;
                drop(r);
                let runtime = runtime.clone();
                std::thread::spawn(move || connection(stream, runtime));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        {
            let mut r = runtime.lock().unwrap();
            if r.retire_committed {
                return Ok(());
            }
            r.tick();
            for event in incoming.try_iter() {
                match event {
                    Incoming::Event(e) => r.hook(e),
                    Incoming::Quota(q) => {
                        r.sequence += 1;
                        let seq = r.sequence;
                        r.quotas.push_back((seq, q));
                        if r.quotas.len() > 64 {
                            r.quotas.pop_front();
                        }
                    }
                    Incoming::Request(request, tx) => {
                        if matches!(
                            request,
                            Control::ListSessions { .. }
                                | Control::ReadOutput { .. }
                                | Control::WaitForStatus { .. }
                        ) {
                            let state = runtime.clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(control(state, request));
                            });
                        } else if r.gui_connections == 0 {
                            let _ =
                                tx.send(Reply::err("This command needs a connected chda window"));
                        } else {
                            r.sequence += 1;
                            let seq = r.sequence;
                            r.requests.insert(seq, (request, tx, Instant::now()));
                        }
                    }
                }
            }
            if r.connections > 0 || r.sessions.values().any(|s| s.checkpoint.record.running) {
                idle = Instant::now();
            }
            if r.connections == 0
                && r.persistence.idle()
                && !r.sessions.values().any(|s| s.checkpoint.record.running)
                && ((accepted && idle.elapsed() > Duration::from_secs(1))
                    || (!accepted && started.elapsed() > Duration::from_secs(30)))
            {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}
fn connection(mut stream: chda_pty::local::Stream, runtime: Arc<Mutex<Runtime>>) {
    let _ = stream.timeout(Duration::from_secs(10));
    let mut gui = false;
    let mut provisional = None;
    let mut attached = None;
    let mut established = false;
    loop {
        let Envelope { version, request } = match protocol::read_request(&mut stream, established) {
            Ok(envelope) => envelope,
            Err(e) => {
                if e.kind() == io::ErrorKind::InvalidData {
                    let _ = protocol::write(
                        &mut stream,
                        &Response::Error("Invalid session request".into()),
                    );
                }
                break;
            }
        };
        if version != protocol::PROTOCOL_VERSION {
            let _ = protocol::write(
                &mut stream,
                &Response::Error(
                    "Incompatible session protocol; the running server and its processes were kept"
                        .into(),
                ),
            );
            break;
        }
        established = true;
        let checkpoint = matches!(request, Request::Checkpoint { .. });
        let durable = checkpoint
            || matches!(
                request,
                Request::Create { .. } | Request::Resume { .. } | Request::Terminate { .. }
            );
        let mut response = match request {
            Request::RetireIfIdle => {
                if attached.is_some() {
                    Response::Retired(false)
                } else {
                    runtime.lock().unwrap().retire_if_idle(!gui)
                }
            }
            request @ Request::PrepareImport { .. } => {
                let result = adoption::receive(&mut stream, &runtime, request);
                if let Ok(Response::Attached { record, .. }) = &result {
                    provisional = Some(record.identity.clone());
                }
                result.unwrap_or_else(|e| Response::Error(e.to_string()))
            }
            Request::CommitImports {
                group,
                layout,
                children,
            } => adoption::commit(&mut stream, &runtime, group, layout, children)
                .map(|()| Response::Ok)
                .unwrap_or_else(|e| Response::Error(e.to_string())),
            request => {
                let mut r = runtime.lock().unwrap();
                if let Request::GuiPresence(available) = &request
                    && gui != *available
                {
                    if *available {
                        r.gui_connections += 1;
                    } else {
                        r.gui_connections -= 1;
                    }
                    gui = *available;
                }
                r.dispatch(request)
                    .unwrap_or_else(|e| Response::Error(e.to_string()))
            }
        };
        if durable {
            let pending = runtime.lock().unwrap().persistence.barrier();
            // Capturing text and syncing files must not hold the shared lock.
            if pending.recv().is_err() {
                break;
            }
            let mut r = runtime.lock().unwrap();
            r.saved();
            if checkpoint {
                response = Response::State(r.state());
            }
        }
        if let Response::Attached { record, .. } = &response {
            attached = Some(record.identity.clone());
        }
        let retired = matches!(response, Response::Retired(true));
        let written = protocol::write(&mut stream, &response);
        if retired {
            let mut r = runtime.lock().unwrap();
            r.retire_committed = written.is_ok();
            if written.is_err() {
                r.retiring = false;
            }
        }
        if written.is_err() {
            break;
        }
    }
    let mut r = runtime.lock().unwrap();
    if let Some(id) = provisional {
        r.abort_import(&id.session);
    }
    if let Some(id) = attached
        && r.get(&id).is_ok()
    {
        r.flush(&id.session, true);
    }
    r.connections -= 1;
    if gui {
        r.gui_connections -= 1;
    }
    let ids: Vec<_> = r
        .sessions
        .iter()
        .filter(|(_, s)| s.dirty)
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        r.flush(&id, true);
    }
}

fn control(runtime: Arc<Mutex<Runtime>>, request: Control) -> Reply {
    use serde_json::json;
    let (cwd, pane) = match &request {
        Control::ListSessions { cwd } => (cwd, None),
        Control::ReadOutput { cwd, pane, .. } | Control::WaitForStatus { cwd, pane, .. } => {
            (cwd, Some(*pane))
        }
        _ => return Reply::err("This command needs a connected chda window"),
    };
    let Ok(repo) = chda_git::main_worktree(cwd) else {
        return Reply::err("The caller is not in a repository");
    };
    let deadline = Instant::now()
        + Duration::from_millis(match &request {
            Control::WaitForStatus { timeout_ms, .. } => timeout_ms.unwrap_or(30_000).min(55_000),
            _ => 0,
        });
    loop {
        let r = runtime.lock().unwrap();
        let entries: Vec<_> = r
            .sessions
            .values()
            .filter(|s| {
                s.checkpoint
                    .record
                    .cwd
                    .as_deref()
                    .and_then(|p| chda_git::main_worktree(p).ok())
                    .as_ref()
                    == Some(&repo)
            })
            .collect();
        let state = |s: &Running| -> &'static str {
            if !s.checkpoint.record.running {
                "ended"
            } else {
                match s.checkpoint.record.agent.as_ref().map(|e| e.kind) {
                    Some(HookKind::PromptSubmitted | HookKind::SessionStart) => "working",
                    Some(HookKind::WaitingInput) => "waiting_for_input",
                    Some(HookKind::Stopped) => "turn_complete",
                    Some(HookKind::Idle) => "idle",
                    Some(HookKind::SessionEnd) => "ended",
                    None => "terminal",
                }
            }
        };
        if let Control::ListSessions { .. } = &request {
            return Reply::ok(
                "Sessions",
                json!({"sessions": entries.iter().filter(|s| !s.checkpoint.record.closed).map(|s|json!({"pane":s.checkpoint.record.pane,"state":state(s),"cwd":s.checkpoint.record.cwd,"title":s.checkpoint.record.title,"agent":s.checkpoint.record.agent.as_ref().map(|e|&e.agent),"session":s.checkpoint.record.agent.as_ref().map(|e|&e.session_id)})).collect::<Vec<_>>()}),
            );
        }
        let Some(s) = entries
            .iter()
            .find(|s| Some(s.checkpoint.record.pane) == pane)
        else {
            return Reply::err("The pane is unavailable in this repository");
        };
        match &request {
            Control::ReadOutput { lines, .. } => {
                let record = s.checkpoint.record.clone();
                let frame = s.terminal.as_ref().map(Session::frame);
                let output = if frame.is_none() {
                    s.checkpoint.output.clone()
                } else {
                    String::new()
                };
                drop(r);
                let (source, text) = match readable_output(&record, frame.as_deref(), output) {
                    Ok(v) => v,
                    Err(e) => return Reply::err(e),
                };
                let lines: Vec<_> = text
                    .trim_end()
                    .lines()
                    .rev()
                    .take(lines.unwrap_or(100).clamp(1, 500))
                    .collect();
                let mut text = lines.into_iter().rev().collect::<Vec<_>>().join("\n");
                if text.len() > 65536 {
                    let mut at = text.len() - 65536;
                    while !text.is_char_boundary(at) {
                        at += 1;
                    }
                    text = text[at..].to_owned();
                }
                return Reply::ok(
                    "Recent output",
                    json!({"pane":pane,"source":source,"text":text}),
                );
            }
            Control::WaitForStatus { states, .. } => {
                if states.is_empty()
                    || states
                        .iter()
                        .any(|s| !chda_agents::control::AGENT_STATES.contains(&s.as_str()))
                {
                    return Reply::err("Unknown wait state");
                }
                let status = state(s);
                if states.iter().any(|v| v == status)
                    || status == "ended"
                    || Instant::now() >= deadline
                {
                    return Reply::ok(
                        "Session status",
                        json!({"pane":pane,"state":status,"timed_out":Instant::now() >= deadline}),
                    );
                }
            }
            _ => unreachable!(),
        }
        drop(r);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn readable_output(
    record: &SessionRecord,
    frame: Option<&chda_term::Frame>,
    output: String,
) -> Result<(&'static str, String), String> {
    if record.agent.is_some() || record.launch.is_some() {
        let provider = record
            .agent
            .as_ref()
            .and_then(|e| chda_agents::AgentId::parse(&e.agent))
            .or_else(|| record.launch.as_ref().map(|c| c.agent))
            .ok_or("Provider unavailable")?;
        let id = record
            .agent
            .as_ref()
            .map(|e| chda_agents::SessionId(e.session_id.clone()))
            .or_else(|| record.launch.as_ref().and_then(|c| c.session.clone()))
            .ok_or("No exact conversation ID is available")?;
        let adapters = chda_agents::adapters();
        let roots = adapters
            .iter()
            .find(|a| a.id() == provider)
            .map(|a| a.session_roots())
            .ok_or("Provider unavailable")?;
        return chda_agents::recent_answers(provider,&id,&roots,20).map(|a|("transcript",a.join("\n\n"))).ok_or_else(||"The exact conversation transcript cannot be read; the screen may contain unfinished input".into());
    }
    let Some(frame) = frame else {
        return Ok(("previous_output", output));
    };
    if frame.alternate_screen {
        return Err(
            "This pane shows a full-screen program; its screen may contain unfinished input".into(),
        );
    }
    let mut end = frame.size.rows;
    if frame.at_prompt()
        && let Some(cursor) = frame.cursor
    {
        end = cursor.y;
        while end > 0
            && frame.rows[usize::from(end)].semantic_prompt
                == chda_term::SemanticPrompt::Continuation
        {
            end -= 1;
        }
    }
    Ok((
        "screen",
        (0..end)
            .map(|y| frame.row_text(y))
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_checkpoint_captures_visible_output_before_its_dirty_event_is_drained() {
        let data = std::env::temp_dir().join(format!(
            "chda-checkpoint-race-{}",
            chda_pty::local::token().unwrap()
        ));
        std::fs::create_dir(&data).unwrap();
        let mut runtime = Runtime {
            retiring: false,
            retire_committed: false,
            committing: BTreeSet::new(),
            legacy_pending: VecDeque::new(),
            children: Default::default(),
            instance: chda_pty::local::token().unwrap(),
            persistence: persistence::Persistence::new(Store::new(&data).unwrap()),
            sessions: BTreeMap::new(),
            layout: None,
            save_error: None,
            layout_saved_at: None,
            connections: 0,
            gui_connections: 0,
            next_pane: 1,
            sequence: 0,
            hooks: VecDeque::new(),
            quotas: VecDeque::new(),
            requests: BTreeMap::new(),
        };
        let record = runtime
            .start(
                1,
                SessionOptions {
                    command: Some(vec![
                        "/usr/bin/env".into(),
                        "-i".into(),
                        format!("HOME={}", data.display()),
                        "PATH=/usr/bin:/bin".into(),
                        "/bin/sh".into(),
                        "-c".into(),
                        "read line; echo checkpoint-race; read line".into(),
                    ]),
                    cwd: Some(data.clone()),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        let id = &record.identity.session;
        runtime.flush(id, true);
        runtime
            .persistence
            .barrier()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        runtime.saved();
        assert!(!runtime.sessions[id].dirty);
        let terminal = runtime.sessions[id].terminal.as_ref().unwrap();
        terminal.write(b"go\n".to_vec());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let frame = terminal.frame();
            if (0..frame.size.rows).any(|y| frame.row_text(y).contains("checkpoint-race")) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        // The frame is visible but tick has not received its change event.
        assert!(!runtime.sessions[id].dirty);
        runtime
            .dispatch(Request::Checkpoint { layout: None })
            .unwrap();
        runtime
            .persistence
            .barrier()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        runtime.saved();
        assert!(
            runtime.sessions[id]
                .checkpoint
                .output
                .contains("checkpoint-race")
        );
        let saved = Store::new(&data).unwrap().load();
        assert!(saved.iter().any(|c| c.output.contains("checkpoint-race")));
        drop(runtime);
        std::fs::remove_dir_all(data).unwrap();
    }
}
