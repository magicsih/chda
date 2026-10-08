//! Cross-window update state and the session holding process.
use crate::platform::update::{self, UpdateJob};
use chda_core::{
    handoff::{Handoff, HandoffPane},
    self_update::UpdateProgress,
};
use chda_term::DetachedSession;
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) type LaunchUpdate =
    std::sync::Arc<dyn Fn(String, Option<PathBuf>) -> Result<UpdateJob, String> + Send + Sync>;

#[derive(Default)]
pub(crate) struct UpdateState {
    pub progress: UpdateProgress,
    pub job: Option<UpdateJob>,
    pub frozen: bool,
    pub preparing: bool,
    pub target: Option<PathBuf>,
    pub journal: Option<PathBuf>,
    pub adopting: bool,
    pub adoption_failed: bool,
    pub inherited: HashMap<u64, (HandoffPane, DetachedSession)>,
    operations: Arc<AtomicUsize>,
}

// A detached UI task may outlive its window. Keep the count app-owned until
// both its background mutation and foreground completion have finished.
pub(crate) struct UpdateOperation(Arc<AtomicUsize>);
impl Drop for UpdateOperation {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl UpdateState {
    pub(crate) fn operation(&self) -> UpdateOperation {
        self.operations.fetch_add(1, Ordering::SeqCst);
        UpdateOperation(self.operations.clone())
    }
    pub(crate) fn has_operations(&self) -> bool {
        self.operations.load(Ordering::SeqCst) != 0
    }
}

/// Invoked only in a separately spawned, non-GUI process.
pub fn run_broker(fd: i32) -> io::Result<()> {
    let inherited = chda_term::handoff::receive(fd)?;
    let mut state = Handoff::decode(&inherited.state)?;
    validate(&state, inherited.ptys.len())?;
    let update = state
        .update
        .clone()
        .ok_or_else(|| io::Error::other("Missing update information"))?;
    update::verify(&update.recovery_exe, &update.recovery_exe)?;
    let job = UpdateJob {
        handoff: update.clone(),
    };
    inherited.reply.prepared(Duration::from_secs(30))?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let installed = loop {
        match job.progress() {
            Some(UpdateProgress::Installed) => break Ok(()),
            Some(UpdateProgress::Failed { message }) => break Err(message),
            _ if update.directory.join("helper-exited").exists() => {
                break Err("The installer stopped before completion.".into());
            }
            _ if Instant::now() > deadline => {
                break Err("The installer did not finish within five minutes.".into());
            }
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let launch = |exe: &std::path::Path, state: &Handoff| -> io::Result<()> {
        update::verify(exe, &update.recovery_exe)?;
        let mut successor =
            chda_term::handoff::spawn_successor(exe, &inherited.ptys, &state.encode()?)?;
        successor.wait_prepared(Duration::from_secs(30))?;
        successor.commit()
    };
    let error = match installed {
        Ok(()) => match launch(&update.target_exe, &state) {
            Ok(()) => return Ok(()),
            Err(error) => format!("The new app could not reconnect: {error}"),
        },
        Err(error) => error,
    };
    state.recovery_error = Some(format!(
        "Update failed. Your sessions were recovered. {error}"
    ));
    // Never release the last PTY descriptors merely because a GUI failed.
    loop {
        match launch(&update.recovery_exe, &state) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = std::fs::write(
                    update.directory.join("recovery-error.txt"),
                    error.to_string(),
                );
                update::recovery_notice();
            }
        }
    }
}

pub(crate) fn inherited(fd: i32) -> io::Result<(Handoff, chda_term::handoff::Inherited)> {
    let inherited = chda_term::handoff::receive(fd)?;
    let state = Handoff::decode(&inherited.state)?;
    validate(&state, inherited.ptys.len())?;
    Ok((state, inherited))
}

pub(crate) fn validate(state: &Handoff, terminals: usize) -> io::Result<()> {
    if state.panes.len() != terminals || state.session.windows.is_empty() {
        return Err(io::Error::other("Invalid window/terminal handoff"));
    }
    // Reject duplicated, absent or out-of-range IDs before constructing a GUI.
    fn visit(
        node: &chda_core::SavedNode,
        ids: &mut Vec<u64>,
        live: &mut Vec<u64>,
    ) -> io::Result<()> {
        match node {
            chda_core::SavedNode::Pane {
                pane: Some(id),
                run,
                ..
            } if *id > 0 && *id < u64::MAX - 4096 => {
                ids.push(*id);
                if matches!(run.as_deref(), Some(chda_core::ManagedRun::Starting { .. })) {
                    return Err(io::Error::other("A pane is still starting"));
                }
                if !run.as_deref().is_some_and(chda_core::ManagedRun::stopped) {
                    live.push(*id);
                }
            }
            chda_core::SavedNode::Pane { .. } => {
                return Err(io::Error::other("Missing handoff pane ID"));
            }
            chda_core::SavedNode::Split { first, second, .. } => {
                visit(first, ids, live)?;
                visit(second, ids, live)?;
            }
        }
        Ok(())
    }
    let mut ids = Vec::new();
    let mut live = Vec::new();
    for window in &state.session.windows {
        for tab in &window.tabs {
            if let chda_core::SavedTabContent::Terminal { root, .. } = &tab.content {
                visit(root, &mut ids, &mut live)?;
            }
        }
    }
    ids.sort_unstable();
    live.sort_unstable();
    let mut expected: Vec<_> = state.panes.iter().map(|p| p.pane).collect();
    expected.sort_unstable();
    if live != expected || ids.windows(2).any(|p| p[0] == p[1]) {
        return Err(io::Error::other(
            "Window layout does not match the inherited terminals",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_panes_keep_layout_identity_without_inherited_ptys() {
        let mut ws = chda_core::Workspace::new();
        let (_, pane) = ws.new_tab();
        ws.pane_mut(pane).unwrap().managed_run = Some(chda_core::ManagedRun::Stopped {
            code: Some(7),
            signal: None,
            error: None,
        });
        let mut state = Handoff::default();
        state.session.windows.push(ws.handoff_snapshot());
        assert!(validate(&state, 0).is_ok());
        state
            .panes
            .push(HandoffPane::new(pane.raw(), 80, 24, vec![]));
        assert!(
            validate(&state, 1).is_err(),
            "stopped panes cannot consume a live descriptor"
        );
        state.panes.clear();
        ws.pane_mut(pane).unwrap().managed_run =
            Some(chda_core::ManagedRun::Starting { started_at: 1 });
        state.session.windows[0] = ws.handoff_snapshot();
        assert!(
            validate(&state, 0).is_err(),
            "updates wait for process preparation"
        );
        ws.pane_mut(pane).unwrap().managed_run = None;
        state.session.windows[0] = ws.handoff_snapshot();
        assert!(
            validate(&state, 0).is_err(),
            "live shells still require their exact descriptor"
        );
    }
}
