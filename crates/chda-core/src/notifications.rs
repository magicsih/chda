//! Window-local operational history. Only live update handoff persists it.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
};

pub const NOTIFICATION_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationContext {
    pub repository: Option<PathBuf>,
    /// The tab label at the event, rather than a reusable tab index.
    pub tab: Option<String>,
    /// Process-unique pane identity, preserved by live handoff.
    pub pane: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationEntry {
    pub id: u64,
    pub timestamp: u64,
    pub severity: Severity,
    pub message: String,
    pub context: NotificationContext,
    operation: Option<String>,
    sequence: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationQueue {
    entries: VecDeque<NotificationEntry>,
    next_sequence: u64,
    read_through: u64,
    selected: BTreeSet<u64>,
    pub dropped: usize,
}

impl NotificationQueue {
    /// Provider retries of the same event must not revive its unread badge.
    pub fn record_once(
        &mut self,
        key: String,
        timestamp: u64,
        severity: Severity,
        message: String,
        context: NotificationContext,
    ) -> u64 {
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| e.operation.as_ref() == Some(&key))
        {
            return entry.id;
        }
        self.record(timestamp, severity, message, context, Some(key))
    }
    pub fn start_operation(
        &mut self,
        timestamp: u64,
        message: String,
        context: NotificationContext,
    ) -> u64 {
        let key = format!("operation-{}", self.next_sequence + 1);
        self.record(timestamp, Severity::Info, message, context, Some(key))
    }

    pub fn update_operation(
        &mut self,
        id: u64,
        timestamp: u64,
        severity: Severity,
        message: String,
        context: NotificationContext,
    ) -> u64 {
        self.record(
            timestamp,
            severity,
            message,
            context,
            Some(format!("operation-{id}")),
        )
    }

    /// Independent events always append, even when their messages are equal.
    /// An operation updates its existing row and becomes unread again.
    pub fn record(
        &mut self,
        timestamp: u64,
        severity: Severity,
        message: String,
        context: NotificationContext,
        operation: Option<String>,
    ) -> u64 {
        self.next_sequence += 1;
        let sequence = self.next_sequence;
        let existing = operation.as_ref().and_then(|key| {
            self.entries
                .iter()
                .position(|entry| entry.operation.as_ref() == Some(key))
        });
        let id = existing.map(|i| self.entries[i].id).unwrap_or(sequence);
        if let Some(i) = existing {
            self.entries.remove(i);
        }
        self.entries.push_back(NotificationEntry {
            id,
            timestamp,
            severity,
            message,
            context,
            operation,
            sequence,
        });
        while self.entries.len() > NOTIFICATION_LIMIT {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.selected
            .retain(|seq| self.entries.iter().any(|e| e.sequence == *seq));
        id
    }

    pub fn latest(&self) -> Option<&str> {
        self.entries.back().map(|e| e.message.as_str())
    }

    pub fn newest(&self) -> Vec<&NotificationEntry> {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by_key(|entry| std::cmp::Reverse((entry.timestamp, entry.sequence)));
        entries
    }

    pub fn get(&self, id: u64) -> Option<&NotificationEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn unread_entry(&self, entry: &NotificationEntry) -> bool {
        entry.sequence > self.read_through && !self.selected.contains(&entry.sequence)
    }

    pub fn unread(&self) -> usize {
        self.entries.iter().filter(|e| self.unread_entry(e)).count()
    }

    /// Snapshot the current arrival watermark. Later events remain unread.
    pub fn mark_open(&mut self) {
        self.read_through = self.next_sequence;
        self.selected.clear();
    }

    pub fn mark_selected(&mut self, id: u64) {
        if let Some(entry) = self.get(id) {
            self.selected.insert(entry.sequence);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(q: &mut NotificationQueue, at: u64, key: Option<&str>) -> u64 {
        q.record(
            at,
            Severity::Info,
            "same text".into(),
            Default::default(),
            key.map(str::to_owned),
        )
    }
    #[test]
    fn opening_reads_only_existing_events_and_selection_reads_one_arrival() {
        let mut q = NotificationQueue::default();
        let a = event(&mut q, 10, None);
        let b = event(&mut q, 10, None);
        assert_eq!(q.newest().iter().map(|e| e.id).collect::<Vec<_>>(), [b, a]);
        assert_eq!(q.unread(), 2);
        q.mark_open();
        let c = event(&mut q, 11, None);
        event(&mut q, 12, None);
        assert_eq!(q.unread(), 2);
        q.mark_selected(c);
        assert_eq!(q.unread(), 1);
        q.mark_open();
        assert_eq!(q.unread(), 0);
    }
    #[test]
    fn operation_updates_keep_their_identity_and_become_unread() {
        let mut q = NotificationQueue::default();
        let a = q.start_operation(10, "starting".into(), Default::default());
        event(&mut q, 11, None);
        q.mark_open();
        assert_eq!(
            q.update_operation(
                a,
                12,
                Severity::Success,
                "finished".into(),
                Default::default()
            ),
            a
        );
        assert_eq!(q.newest().len(), 2);
        assert_eq!(q.newest()[0].id, a);
        assert_eq!(q.unread(), 1);
        q.mark_selected(a);
        assert_eq!(
            q.update_operation(a, 13, Severity::Error, "failed".into(), Default::default()),
            a
        );
        assert_eq!(q.unread(), 1);
    }
    #[test]
    fn repeated_provider_events_do_not_append_or_become_unread_again() {
        let mut q = NotificationQueue::default();
        let id = q.record_once(
            "event-1".into(),
            10,
            Severity::Success,
            "finished".into(),
            Default::default(),
        );
        q.mark_open();
        assert_eq!(
            q.record_once(
                "event-1".into(),
                10,
                Severity::Success,
                "finished".into(),
                Default::default()
            ),
            id
        );
        assert_eq!(q.unread(), 0);
        assert_eq!(q.newest().len(), 1);
    }
    #[test]
    fn retained_history_is_bounded_and_handoff_preserves_read_state() {
        let mut q = NotificationQueue::default();
        for at in 0..(NOTIFICATION_LIMIT as u64 + 3) {
            event(&mut q, at, None);
        }
        assert_eq!(q.newest().len(), NOTIFICATION_LIMIT);
        assert_eq!(q.dropped, 3);
        q.mark_open();
        event(&mut q, 1000, None);
        let restored: NotificationQueue =
            serde_json::from_slice(&serde_json::to_vec(&q).unwrap()).unwrap();
        assert_eq!(restored, q);
        assert_eq!(restored.unread(), 1);
        assert_eq!(NotificationQueue::default().unread(), 0);
    }
}
