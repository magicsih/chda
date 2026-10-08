//! What a running chda hands to its successor in a live upgrade, besides
//! the PTYs themselves: every window's layout with pane ids, and per pane
//! its title, agent state and terminal snapshot.
//!
//! Encoded as one JSON line followed by the snapshots back to back, so
//! megabytes of scrollback do not go through JSON.

use std::io;

use serde::{Deserialize, Serialize};

use crate::restore::SavedSession;
use crate::workspace::PaneAgent;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Handoff {
    /// Windows from [`crate::Workspace::handoff_snapshot`].
    pub session: SavedSession,
    /// One entry per handed PTY, in the same order.
    pub panes: Vec<HandoffPane>,
    #[serde(default)]
    pub started_at: u64,
    #[serde(default)]
    pub update: Option<UpdateHandoff>,
    #[serde(default)]
    pub recovery_error: Option<String>,
    /// Same order as session.windows; intentionally absent from cold saves.
    #[serde(default)]
    pub notifications: Vec<crate::notifications::NotificationQueue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateHandoff {
    pub directory: std::path::PathBuf,
    pub target_exe: std::path::PathBuf,
    pub recovery_exe: std::path::PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HandoffPane {
    /// The pane's id; its programs report to it through `CHDA_PANE_ID`.
    pub pane: u64,
    pub title: String,
    pub agent: Option<PaneAgent>,
    pub agent_live: bool,
    /// Terminal size of the snapshot, in cells.
    pub cols: u16,
    pub rows: u16,
    /// The terminal's screen and state as VT sequences.
    #[serde(skip)]
    pub snapshot: Vec<u8>,
    snapshot_len: usize,
}

impl HandoffPane {
    pub fn new(pane: u64, cols: u16, rows: u16, snapshot: Vec<u8>) -> Self {
        Self {
            pane,
            cols,
            rows,
            snapshot_len: snapshot.len(),
            snapshot,
            ..Default::default()
        }
    }
}

impl Handoff {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let mut header = serde_json::to_value(self)?;
        for (encoded, pane) in header["panes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(&self.panes)
        {
            encoded["snapshot_len"] = pane.snapshot.len().into();
        }
        let mut out = serde_json::to_vec(&header)?;
        out.push(b'\n');
        for pane in &self.panes {
            out.extend_from_slice(&pane.snapshot);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let newline = bytes
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| io::Error::other("handoff state has no header"))?;
        let mut handoff: Self = serde_json::from_slice(&bytes[..newline])?;
        let mut rest = &bytes[newline + 1..];
        for pane in &mut handoff.panes {
            if rest.len() < pane.snapshot_len {
                return Err(io::Error::other("handoff state is cut short"));
            }
            let (snapshot, tail) = rest.split_at(pane.snapshot_len);
            pane.snapshot = snapshot.to_vec();
            rest = tail;
        }
        if !rest.is_empty() {
            return Err(io::Error::other("unexpected bytes after handoff snapshots"));
        }
        let mut ids = std::collections::HashSet::new();
        if handoff
            .panes
            .iter()
            .any(|p| p.cols == 0 || p.rows == 0 || !ids.insert(p.pane))
        {
            return Err(io::Error::other("invalid or repeated handoff pane"));
        }
        Ok(handoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentStatus;

    #[test]
    fn validates_snapshot_lengths_and_pane_identity() {
        let mut handoff = Handoff {
            session: SavedSession::default(),
            panes: vec![HandoffPane::new(7, 80, 24, b"before".to_vec())],
            ..Default::default()
        };
        handoff.panes[0].snapshot = b"changed snapshot".to_vec();
        let bytes = handoff.encode().unwrap();
        assert_eq!(
            Handoff::decode(&bytes).unwrap().panes[0].snapshot,
            b"changed snapshot"
        );
        let mut extra = bytes;
        extra.push(1);
        assert!(Handoff::decode(&extra).is_err());
        handoff.panes.push(handoff.panes[0].clone());
        assert!(Handoff::decode(&handoff.encode().unwrap()).is_err());
        handoff.panes.pop();
        handoff.panes[0].rows = 0;
        assert!(Handoff::decode(&handoff.encode().unwrap()).is_err());
    }

    #[test]
    fn handoff_round_trips_with_binary_snapshots() {
        let mut first = HandoffPane::new(7, 80, 24, b"\x1b[?2004h\nline\n".to_vec());
        first.title = "build".into();
        first.agent = Some(PaneAgent {
            agent: "claude".into(),
            status: AgentStatus::Working,
            since: 5,
            seen: false,
        });
        first.agent_live = true;
        let mut notifications = crate::notifications::NotificationQueue::default();
        notifications.record(
            1,
            crate::notifications::Severity::Info,
            "retained".into(),
            Default::default(),
            None,
        );
        notifications.mark_open();
        notifications.record(
            2,
            crate::notifications::Severity::Error,
            "unread".into(),
            Default::default(),
            None,
        );
        let handoff = Handoff {
            session: SavedSession::default(),
            panes: vec![first, HandoffPane::new(9, 100, 30, vec![0, 255, 10])],
            notifications: vec![notifications],
            ..Default::default()
        };
        let decoded = Handoff::decode(&handoff.encode().unwrap()).unwrap();
        assert_eq!(decoded, handoff);
        assert_eq!(decoded.notifications[0].unread(), 1);
        let bytes = handoff.encode().unwrap();
        assert!(Handoff::decode(&bytes[..bytes.len() - 1]).is_err());
    }
}
