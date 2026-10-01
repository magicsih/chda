//! Platform-specific UI pieces: default font, alert sound, notifications and
//! the Dock badge.

use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod macos;

/// Monospace face every install of the platform ships with.
#[cfg(target_os = "macos")]
pub const DEFAULT_MONOSPACE: &str = "Menlo";
#[cfg(target_os = "windows")]
pub const DEFAULT_MONOSPACE: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const DEFAULT_MONOSPACE: &str = "DejaVu Sans Mono";

/// Play the system alert sound.
pub fn beep() {
    #[cfg(target_os = "macos")]
    macos::beep();
}

/// Where clicking a notification leads: the agent's pane, else its worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationTarget {
    pub pane: Option<u64>,
    pub worktree: PathBuf,
}

impl NotificationTarget {
    /// Encode as a notification identifier: `chda/<nonce>/<pane or ->/<path>`.
    /// The nonce keeps identifiers unique so notifications do not replace
    /// each other.
    fn encode(&self, nonce: u64) -> String {
        let pane = self.pane.map_or("-".to_owned(), |p| p.to_string());
        format!("chda/{nonce}/{pane}/{}", self.worktree.to_string_lossy())
    }

    fn decode(id: &str) -> Option<Self> {
        let mut parts = id.splitn(4, '/');
        if parts.next()? != "chda" {
            return None;
        }
        parts.next()?;
        let pane = parts.next()?;
        Some(Self {
            pane: pane.parse().ok(),
            worktree: PathBuf::from(parts.next()?),
        })
    }
}

/// Start delivering notification clicks to `on_click`. Call once, on the
/// main thread.
pub fn init_notifications(on_click: impl Fn(NotificationTarget) + 'static) {
    #[cfg(target_os = "macos")]
    macos::init_notifications(move |id| {
        if let Some(target) = NotificationTarget::decode(&id) {
            on_click(target);
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = on_click;
}

/// Show a user notification; clicking it reports `target`.
pub fn notify(title: &str, body: &str, target: &NotificationTarget) {
    #[cfg(target_os = "macos")]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let id = target.encode(NONCE.fetch_add(1, Ordering::Relaxed));
        macos::notify(title, body, &id);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (title, body, target);
}

/// Un-minimize the app's windows (before bringing one to the front).
pub fn restore_windows() {
    #[cfg(target_os = "macos")]
    macos::restore_windows();
}

/// Show `count` on the Dock icon; zero clears it.
pub fn set_badge(count: usize) {
    #[cfg(target_os = "macos")]
    macos::set_badge(count);
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_targets_round_trip() {
        let t = NotificationTarget {
            pane: Some(12),
            worktree: PathBuf::from("/src/app.worktrees/feat-x"),
        };
        assert_eq!(NotificationTarget::decode(&t.encode(3)), Some(t));
        let t = NotificationTarget {
            pane: None,
            worktree: PathBuf::from("/a/b"),
        };
        assert_eq!(NotificationTarget::decode(&t.encode(0)), Some(t));
        assert_eq!(NotificationTarget::decode("other/1/2/x"), None);
    }
}
