//! Preserve the legacy broker's prepared/commit ownership boundary in the service.
use super::*;
use std::io::Read;

pub(super) fn receive(
    stream: &mut chda_pty::local::Stream,
    runtime: &Arc<Mutex<Runtime>>,
    request: Request,
) -> io::Result<Response> {
    let Request::PrepareImport {
        group,
        pane,
        options,
        pid,
        size,
        snapshot_len,
        agent,
        agent_live,
    } = request
    else {
        unreachable!()
    };
    if !crate::store::valid_id(&group)
        || pane == 0
        || pane >= u64::MAX - 4096
        || size.cols == 0
        || size.rows == 0
        || size.cols > 1000
        || size.rows > 1000
        || snapshot_len > 512 * 1024 * 1024
    {
        return Err(io::Error::other("Invalid legacy terminal import"));
    }
    let pty = stream.receive_pty(pid)?;
    let mut snapshot = vec![0; snapshot_len];
    stream.read_exact(&mut snapshot)?;
    let (tx, events) = mpsc::channel();
    let terminal = Session::prepare_adoption(
        options.clone(),
        chda_term::DetachedSession {
            pty,
            snapshot,
            size,
        },
        tx,
        || {},
    )?;
    terminal
        .barrier()
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| io::Error::other("Legacy terminal did not prepare"))?;
    let mut r = runtime.lock().unwrap();
    let previous_id = r
        .sessions
        .iter()
        .find(|(_, s)| s.checkpoint.record.pane == pane)
        .map(|(id, s)| (id.clone(), s.checkpoint.record.running));
    if previous_id.as_ref().is_some_and(|(_, running)| *running) {
        return Err(io::Error::other(
            "The service already owns a running terminal with this pane ID",
        ));
    }
    let identity = Identity {
        session: previous_id
            .as_ref()
            .map(|(id, _)| id.clone())
            .unwrap_or(chda_pty::local::token()?),
        run: chda_pty::local::token()?,
        server: r.instance.clone(),
    };
    let agent = agent.filter(|_| agent_live).map(|a| HookEvent {
        agent: a.agent,
        session_id: String::new(),
        cwd: options.cwd.clone().unwrap_or_default(),
        kind: match a.status {
            chda_core::AgentStatus::Working => HookKind::PromptSubmitted,
            chda_core::AgentStatus::WaitingInput => HookKind::WaitingInput,
            chda_core::AgentStatus::Review => HookKind::Stopped,
            chda_core::AgentStatus::Idle => HookKind::Idle,
        },
        timestamp: a.since,
        pane: Some(pane),
        run: None,
        child: None,
    });
    let record = SessionRecord {
        pane,
        identity: identity.clone(),
        pid: Some(pid),
        running: true,
        closed: false,
        title: terminal.frame().title.clone(),
        cwd: options.cwd,
        launch: None,
        agent,
        exit: None,
        saved_at: None,
        save_error: None,
        scrollback: options.scrollback,
        legacy_hooks: true,
    };
    let previous = previous_id
        .and_then(|(id, _)| r.sessions.remove(&id))
        .map(Box::new);
    r.sessions.insert(
        identity.session.clone(),
        Running {
            importing: Some(Importing {
                group,
                previous,
                deferred: vec![],
            }),
            checkpoint: Checkpoint {
                record,
                output: String::new(),
            },
            terminal: Some(terminal),
            events,
            sequence: 0,
            pending: VecDeque::new(),
            dirty: false,
            last_save: Instant::now(),
        },
    );
    match r.attached(&identity) {
        Ok(response) => Ok(response),
        Err(e) => {
            r.abort_import(&identity.session);
            Err(e)
        }
    }
}

fn layout_panes(layout: &chda_core::SavedSession) -> io::Result<(BTreeSet<u64>, BTreeSet<u64>)> {
    fn visit(
        n: &chda_core::SavedNode,
        all: &mut BTreeSet<u64>,
        live: &mut BTreeSet<u64>,
    ) -> io::Result<()> {
        match n {
            chda_core::SavedNode::Pane {
                pane: Some(id),
                run,
                ..
            } if *id > 0 && all.insert(*id) => {
                if !run.as_deref().is_some_and(chda_core::PaneRun::stopped) {
                    live.insert(*id);
                }
            }
            chda_core::SavedNode::Split { first, second, .. } => {
                visit(first, all, live)?;
                visit(second, all, live)?;
            }
            _ => return Err(io::Error::other("Missing or repeated imported pane ID")),
        }
        Ok(())
    }
    let (mut all, mut live) = (BTreeSet::new(), BTreeSet::new());
    for w in &layout.windows {
        for t in &w.tabs {
            if let chda_core::SavedTabContent::Terminal { root, .. } = &t.content {
                visit(root, &mut all, &mut live)?;
            }
        }
    }
    Ok((all, live))
}

pub(super) fn commit(
    stream: &mut chda_pty::local::Stream,
    runtime: &Arc<Mutex<Runtime>>,
    group: String,
    layout: chda_core::SavedSession,
    children: chda_agents::ChildBoard,
) -> io::Result<()> {
    let reply = stream.receive_reply()?;
    let ids = {
        let mut r = runtime.lock().unwrap();
        let (all, live) = layout_panes(&layout)?;
        let ids: Vec<_> = r
            .sessions
            .iter()
            .filter(|(_, s)| s.importing.as_ref().is_some_and(|i| i.group == group))
            .map(|(id, _)| id.clone())
            .collect();
        if ids.is_empty() || !crate::store::valid_id(&group) || !r.committing.insert(group.clone())
        {
            return Err(io::Error::other(
                "No prepared import group, or commit already in progress",
            ));
        }
        let valid = ids
            .iter()
            .all(|id| all.contains(&r.sessions[id].checkpoint.record.pane))
            && live.iter().all(|pane| {
                r.sessions.values().any(|s| {
                    s.checkpoint.record.pane == *pane
                        && s.checkpoint.record.running
                        && s.importing.as_ref().is_none_or(|i| i.group == group)
                })
            });
        if !valid {
            r.committing.remove(&group);
            return Err(io::Error::other(
                "The layout does not match every prepared terminal",
            ));
        }
        r.update_launches(&layout);
        for w in &layout.windows {
            for t in &w.tabs {
                if let chda_core::SavedTabContent::Terminal { root, .. } = &t.content {
                    r.import_conversations(root);
                }
            }
        }
        ids
    };
    // Do not hold the runtime lock during the predecessor handshake. Other
    // live sessions and output parsing keep running while the GUI prepares.
    if let Err(error) = reply.prepared(Duration::from_secs(30)) {
        let mut r = runtime.lock().unwrap();
        r.committing.remove(&group);
        for id in ids {
            r.abort_import(&id);
        }
        return Err(error);
    }
    let mut r = runtime.lock().unwrap();
    for id in &ids {
        let s = r.sessions.get_mut(id).unwrap();
        let importing = s.importing.take().unwrap();
        let t = s.terminal.as_ref().unwrap();
        t.commit_adoption();
        for action in importing.deferred {
            action.apply(t);
        }
        s.dirty = true;
        s.last_save = Instant::now() - Duration::from_secs(5);
    }
    r.committing.remove(&group);
    r.children = children;
    r.layout = Some(layout.clone());
    match r.store.save_layout(&layout) {
        Ok(()) => {
            r.save_error = None;
            r.layout_saved_at = Some(protocol::now());
        }
        Err(e) => r.save_error = Some(e.to_string()),
    }
    for id in ids {
        r.flush(&id, false);
    }
    let pending = std::mem::take(&mut r.legacy_pending);
    for event in pending {
        r.hook(event);
    }
    Ok(())
}

impl Runtime {
    pub(super) fn abort_import(&mut self, id: &str) {
        let Some(importing) = self.sessions.get(id).and_then(|s| s.importing.as_ref()) else {
            return;
        };
        if self.committing.contains(&importing.group) {
            return;
        }
        let mut session = self.sessions.remove(id).unwrap();
        let previous = session.importing.take().unwrap().previous;
        // Uncommitted chda-term drops neither read the PTY nor signal its child.
        drop(session);
        if let Some(previous) = previous {
            self.sessions.insert(id.to_owned(), *previous);
        }
    }
    fn import_conversations(&mut self, node: &chda_core::SavedNode) {
        match node {
            chda_core::SavedNode::Pane {
                pane: Some(pane),
                agent,
                session,
                agent_cwd,
                launch,
                cwd,
                ..
            } => {
                if let Some(s) = self
                    .sessions
                    .values_mut()
                    .find(|s| s.checkpoint.record.pane == *pane && s.importing.is_some())
                    && let Some(e) = &mut s.checkpoint.record.agent
                {
                    if agent.as_deref() == Some(e.agent.as_str()) {
                        e.session_id = session.clone().unwrap_or_default();
                        e.cwd = agent_cwd
                            .clone()
                            .or_else(|| launch.as_ref().map(|l| l.cwd.clone()))
                            .or(cwd.clone())
                            .unwrap_or_default();
                    }
                    if e.session_id.is_empty()
                        && let Some(l) = launch.as_ref().filter(|l| l.agent.as_str() == e.agent)
                    {
                        e.session_id = l.session.as_ref().map(|s| s.0.clone()).unwrap_or_default();
                    }
                }
            }
            chda_core::SavedNode::Split { first, second, .. } => {
                self.import_conversations(first);
                self.import_conversations(second);
            }
            _ => {}
        }
    }
}
