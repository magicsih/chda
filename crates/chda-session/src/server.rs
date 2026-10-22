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
    collections::{BTreeMap, VecDeque},
    io,
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

struct Running {
    checkpoint: Checkpoint,
    terminal: Option<Session>,
    events: mpsc::Receiver<Event>,
    sequence: u64,
    pending: VecDeque<(u64, Event)>,
    dirty: bool,
    last_save: Instant,
}
struct Runtime {
    instance: String,
    store: Store,
    sessions: BTreeMap<String, Running>,
    layout: Option<chda_core::SavedSession>,
    save_error: Option<String>,
    connections: usize,
    next_pane: u64,
    sequence: u64,
    hooks: VecDeque<(u64, HookEvent)>,
    quotas: VecDeque<(u64, chda_agents::quota::QuotaSnapshot)>,
    requests: BTreeMap<u64, (Control, mpsc::Sender<Reply>, Instant)>,
}
impl Runtime {
    fn state(&self) -> State {
        State {
            instance: self.instance.clone(),
            executable_version: env!("CARGO_PKG_VERSION").into(),
            layout: self.layout.clone(),
            sessions: self
                .sessions
                .values()
                .map(|s| s.checkpoint.record.clone())
                .collect(),
            save_error: self.save_error.clone(),
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
            launch: previous.as_ref().and_then(|id| {
                self.sessions
                    .get(&id.session)?
                    .checkpoint
                    .record
                    .launch
                    .clone()
            }),
            agent: None,
            exit: None,
            saved_at: None,
            save_error: None,
            scrollback: options.scrollback,
        };
        let output = previous
            .as_ref()
            .and_then(|id| self.sessions.get(&id.session))
            .map(|s| s.checkpoint.output.clone())
            .unwrap_or_default();
        self.sessions.insert(
            identity.session.clone(),
            Running {
                checkpoint: Checkpoint {
                    record: record.clone(),
                    output,
                },
                terminal: Some(terminal),
                events,
                sequence: 0,
                pending: VecDeque::new(),
                dirty: true,
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
        if capture && let Some(t) = &s.terminal {
            match t.checkpoint().recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(text)) => s.checkpoint.output = text,
                _ => {
                    s.checkpoint.record.save_error = Some("Could not capture terminal text".into());
                    return;
                }
            }
        }
        let text_saved = capture || !s.checkpoint.record.running;
        let prior_error = s.checkpoint.record.save_error.take();
        match self.store.checkpoint(&mut s.checkpoint, text_saved) {
            Ok(()) => {
                s.checkpoint.record.save_error = None;
                if text_saved {
                    s.dirty = false;
                    s.last_save = Instant::now();
                }
            }
            Err(e) => {
                let _ = prior_error;
                s.checkpoint.record.save_error = Some(e.to_string());
                s.last_save = Instant::now();
            }
        }
    }
    fn tick(&mut self) {
        let mut flush = Vec::new();
        for (id, s) in &mut self.sessions {
            for event in s.events.try_iter() {
                match &event {
                    Event::Frame { has_output: true } => s.dirty = true,
                    Event::TitleChanged(t) => {
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
            if s.dirty && s.last_save.elapsed() >= Duration::from_secs(5) {
                flush.push((id.clone(), s.checkpoint.record.running));
            }
        }
        flush.sort();
        flush.dedup();
        for (id, capture) in flush {
            self.flush(&id, capture);
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
    fn hook(&mut self, event: HookEvent) {
        let Some(pane) = event.pane else {
            return;
        };
        let Some(s) = self
            .sessions
            .values_mut()
            .find(|s| s.checkpoint.record.pane == pane && s.checkpoint.record.running)
        else {
            return;
        };
        if event.run.as_deref() != Some(s.checkpoint.record.identity.run.as_str()) {
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
        }
        let id = s.checkpoint.record.identity.session.clone();
        self.flush(&id, false);
        self.sequence += 1;
        self.hooks.push_back((self.sequence, event));
        if self.hooks.len() > 4096 {
            self.hooks.pop_front();
        }
    }
    fn dispatch(&mut self, request: Request) -> io::Result<Response> {
        match request {
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
            } => {
                self.tick();
                let s = self.get(&identity)?;
                let frame = s
                    .terminal
                    .as_ref()
                    .map(Session::frame)
                    .filter(|f| f.generation != generation)
                    .map(|f| Box::new((*f).clone()));
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
                    match self.store.save_layout(&layout) {
                        Ok(()) => self.save_error = None,
                        Err(e) => self.save_error = Some(e.to_string()),
                    }
                }
                let ids: Vec<_> = self.sessions.keys().cloned().collect();
                for id in ids {
                    if self.sessions[&id].dirty {
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
        let frame = Box::new((*s.terminal.as_ref().unwrap().frame()).clone());
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

/// Run until no process and no connection remains. Loading checkpoints never
/// starts a process. The caller must use a separate process for real ownership.
pub fn serve(data: &Path) -> io::Result<()> {
    let listener = chda_pty::local::Listener::bind(&crate::socket_path(data))?;
    let store = Store::new(data)?;
    let layout = store.migrate(data)?;
    let sessions: BTreeMap<String, Running> = store
        .load()
        .into_iter()
        .map(|c| {
            (
                c.record.identity.session.clone(),
                Running {
                    checkpoint: c,
                    terminal: None,
                    events: mpsc::channel().1,
                    sequence: 0,
                    pending: VecDeque::new(),
                    dirty: false,
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
        instance: chda_pty::local::token()?,
        store,
        sessions,
        layout,
        save_error: None,
        connections: 0,
        next_pane,
        sequence: 0,
        hooks: VecDeque::new(),
        quotas: VecDeque::new(),
        requests: BTreeMap::new(),
    }));
    let (incoming_tx, incoming) = mpsc::channel();
    let _hooks = chda_agents::ipc::serve(&chda_agents::ipc::socket_path(data), incoming_tx, || {})?;
    let started = Instant::now();
    let mut accepted = false;
    let mut idle = Instant::now();
    loop {
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
                        } else if r.connections == 0 {
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
    loop {
        let Envelope { version, request } = match protocol::read::<Envelope>(&mut stream, true) {
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
        let response = runtime
            .lock()
            .unwrap()
            .dispatch(request)
            .unwrap_or_else(|e| Response::Error(e.to_string()));
        if protocol::write(&mut stream, &response).is_err() {
            break;
        }
    }
    let mut r = runtime.lock().unwrap();
    r.connections -= 1;
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
