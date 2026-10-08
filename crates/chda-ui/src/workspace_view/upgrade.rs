use super::*;
use chda_core::{
    handoff::{Handoff, HandoffPane},
    self_update::UpdateProgress,
};
use std::time::Duration;

impl WorkspaceView {
    pub(crate) fn start_update(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(release) = self.release_notice.clone() else {
            return;
        };
        let target = {
            let mut registry = self.env.windows.borrow_mut();
            if registry.update.progress.busy() {
                return;
            }
            registry.update.progress = UpdateProgress::Checking;
            registry.update.job = None;
            registry.update.target.clone()
        };
        let launcher = self.env.update_launcher.clone();
        let work = cx.background_spawn(async move { launcher(release.version, target) });
        let env = self.env.clone();
        cx.spawn(async move |_, cx| {
            let result = work.await;
            cx.update(|cx| {
                let mut registry = env.windows.borrow_mut();
                match result {
                    Ok(job) => registry.update.job = Some(job),
                    Err(message) => registry.update.progress = UpdateProgress::Failed { message },
                }
                let entries = registry.entries.clone();
                drop(registry);
                for entry in entries {
                    let _ = entry.view.update(cx, |_, cx| cx.notify());
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn watch_update(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let mut previous = UpdateProgress::Idle;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if this
                    .update_in(cx, |view, window, cx| {
                        let first = view
                            .env
                            .windows
                            .borrow()
                            .entries
                            .first()
                            .is_some_and(|e| e.view == view.self_weak);
                        if first
                            && !view.env.windows.borrow().update.frozen
                            && let Some(dir) = &view.env.data_dir
                            && let Ok(text) = ipc::take_fallback(dir)
                        {
                            let mut registry = view.env.windows.borrow_mut();
                            for line in text.lines() {
                                if let Ok(q) = serde_json::from_str::<
                                    chda_core::agents::quota::QuotaSnapshot,
                                >(line)
                                {
                                    registry.replay.push(Incoming::Quota(q));
                                } else if let Ok(e) = serde_json::from_str::<HookEvent>(line) {
                                    registry.replay.push(Incoming::Event(e));
                                }
                            }
                        }
                        view.drain_hook_events(window, cx);
                        let prepare = {
                            let mut registry = view.env.windows.borrow_mut();
                            if !registry.update.preparing
                                && !registry.update.adopting
                                && let Some(job) = &registry.update.job
                                && let Some(progress) = job.progress()
                            {
                                registry.update.progress = progress;
                            }
                            if matches!(registry.update.progress, UpdateProgress::Failed { .. })
                                && !registry.update.preparing
                                && let Some(job) = registry.update.job.take()
                            {
                                crate::platform::update::discard(job);
                            }
                            let prepare = registry.update.progress == UpdateProgress::Ready
                                && !registry.update.preparing;
                            if prepare {
                                registry.update.preparing = true;
                            }
                            if previous != registry.update.progress {
                                previous = registry.update.progress.clone();
                                cx.notify();
                            }
                            prepare
                        };
                        if prepare {
                            view.prepare_update(window, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn prepare_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (job, entries, active, started_at) = {
            let registry = self.env.windows.borrow();
            (
                registry.update.job.clone().unwrap(),
                registry.entries.clone(),
                registry.active,
                registry.started_at.unwrap_or(0),
            )
        };
        // Let in-flight mutations complete, including callbacks that open a
        // terminal. Do not discard an unfinished sheet or rename on relaunch.
        let busy = self.env.windows.borrow().update.has_operations()
            || entries.iter().any(|entry| {
                let pending = |view: &Self| {
                    view.sheet.is_some()
                        || view.launch_sheet.is_some()
                        || view.note_sheet.is_some()
                        || view.confirm.is_some()
                        || view.palette.is_some()
                        || view.renaming.is_some()
                        || !view.initializing_panes.is_empty()
                };
                if entry.view == self.self_weak {
                    pending(self)
                } else {
                    entry
                        .view
                        .upgrade()
                        .is_some_and(|view| pending(view.read(cx)))
                }
            });
        if busy {
            let mut registry = self.env.windows.borrow_mut();
            registry.update.preparing = false;
            registry.update.progress = UpdateProgress::Waiting;
            cx.notify();
            return;
        }
        let journal = self
            .env
            .windows
            .borrow()
            .ipc_server
            .as_ref()
            .map(|s| s.journal_to(Some(&job.handoff.directory.join("events.jsonl"))))
            .transpose();
        if let Err(error) = journal {
            let _ = job.signal("cancel");
            let mut registry = self.env.windows.borrow_mut();
            registry.update.progress = UpdateProgress::Failed {
                message: error.to_string(),
            };
            registry.update.preparing = false;
            registry.update.job = None;
            return;
        }
        // Synchronize the journal barrier, then apply everything queued before it.
        self.drain_hook_events(window, cx);
        self.env.windows.borrow_mut().update.frozen = true;
        self.env.windows.borrow_mut().update.progress = UpdateProgress::Preparing;
        let mut state = Handoff {
            started_at,
            update: Some(job.handoff.clone()),
            ..Default::default()
        };
        state.session.active_window = entries
            .iter()
            .position(|e| Some(e.window.window_id()) == active)
            .unwrap_or(0);
        let mut targets = HashMap::new();
        let mut receivers = Vec::new();
        let mut metadata = HashMap::new();
        for entry in &entries {
            let mut collect = |view: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                view.note_bounds(window);
                let mut saved = view.ws.handoff_snapshot();
                saved.bounds = view.bounds;
                state.session.windows.push(saved);
                state.notifications.push(view.notifications.clone());
                for tab in view.ws.tabs() {
                    for pane in tab.panes() {
                        if let Some((terminal, _)) = view.panes.get(&pane) {
                            let info = view.ws.pane(pane).unwrap();
                            metadata.insert(
                                pane.raw(),
                                (info.title.clone(), info.agent.clone(), info.agent_live),
                            );
                            targets.insert(pane.raw(), (entry.window, terminal.clone()));
                            receivers.push((pane.raw(), terminal.read(cx).detach()));
                        }
                    }
                }
                cx.notify();
            };
            if entry.window == window.window_handle() {
                collect(self, window, cx);
            } else {
                let _ = entry.window.update(cx, |_, window, cx| {
                    let _ = entry.view.update(cx, |v, cx| collect(v, window, cx));
                });
            }
        }
        let work = cx.background_spawn(async move {
            let mut detached = Vec::new();
            let mut errors = Vec::new();
            // Do not abandon a pending detach receiver: it may own the last PTY.
            for (id, receiver) in receivers {
                match receiver.recv() {
                    Ok(Ok(session)) => detached.push((id, session)),
                    Ok(Err(error)) => errors.push(error.to_string()),
                    Err(error) => errors.push(error.to_string()),
                }
            }
            let result = (|| -> std::io::Result<()> {
                if !errors.is_empty() {
                    return Err(std::io::Error::other(errors.join("; ")));
                }
                let mut ptys = Vec::new();
                for (id, session) in &detached {
                    let mut pane = HandoffPane::new(
                        *id,
                        session.size.cols,
                        session.size.rows,
                        session.snapshot.clone(),
                    );
                    let (title, agent, live) = metadata.remove(id).unwrap();
                    pane.title = title;
                    pane.agent = agent;
                    pane.agent_live = live;
                    state.panes.push(pane);
                    ptys.push(session.pty.try_clone()?);
                }
                crate::upgrade::validate(&state, ptys.len())?;
                let mut broker = chda_term::handoff::spawn_broker(
                    &job.handoff.recovery_exe,
                    &ptys,
                    &state.encode()?,
                )?;
                broker.wait_prepared(Duration::from_secs(30))?;
                broker.commit()?;
                // From this point the broker owns recovery, including failure
                // to signal the installer. Never let old GUI destructors kill a shell.
                let _ = job.signal("install");
                std::process::exit(0);
            })();
            let _ = job.signal("cancel");
            crate::platform::update::discard(job);
            (result.unwrap_err().to_string(), detached)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (mut message, detached) = work.await;
            let mut pending = detached;
            while !pending.is_empty() {
                let mut retry = Vec::new();
                for (id, session) in pending {
                    // Keep the original master until the replacement terminal
                    // has produced its first frame and committed ownership.
                    let result =
                        session
                            .pty
                            .try_clone()
                            .map_err(|e| e.to_string())
                            .and_then(|pty| {
                                let copy = chda_term::DetachedSession {
                                    pty,
                                    snapshot: session.snapshot.clone(),
                                    size: session.size,
                                };
                                let (target, terminal) = targets
                                    .get(&id)
                                    .ok_or_else(|| "Recovery window disappeared".to_owned())?;
                                target
                                    .update(cx, |_, window, cx| {
                                        terminal.update(cx, |v, cx| v.reconnect(copy, window, cx))
                                    })
                                    .map_err(|e| e.to_string())?
                                    .map_err(|e| e.to_string())
                            });
                    if let Err(error) = result {
                        message = format!("Reconnecting your sessions: {error}");
                        retry.push((id, session));
                    }
                }
                pending = retry;
                if !pending.is_empty() {
                    let error = message.clone();
                    let _ = this.update(cx, |view, cx| {
                        view.env.windows.borrow_mut().update.progress =
                            UpdateProgress::Failed { message: error };
                        cx.notify();
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                }
            }
            let _ = this.update_in(cx, |view, window, cx| {
                {
                    let mut registry = view.env.windows.borrow_mut();
                    if let Some(server) = &registry.ipc_server {
                        let _ = server.journal_to(None);
                    }
                    registry.update.frozen = false;
                    registry.update.preparing = false;
                    registry.update.job = None;
                    registry.update.progress = UpdateProgress::Failed { message };
                }
                view.drain_hook_events(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn adoption_ready(&self, cx: &App) -> bool {
        self.panes
            .values()
            .all(|(p, _)| p.read(cx).adoption_ready())
    }
    pub(crate) fn finish_adoption(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (pane, _) in self.panes.values() {
            pane.read(cx).commit_adoption();
        }
        self.drain_hook_events(window, cx);
        // Window focus was restored before preparation. Focusing every
        // window here would replace the saved active window with the last one.
        self.save_session();
        window.refresh();
        cx.notify();
    }
}
