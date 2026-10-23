//! Profile-wide ordered layout saves, without waiting in a GPUI callback.
use super::*;
use std::sync::Condvar;

#[derive(Default)]
struct Pending {
    layout: Option<chda_core::SavedSession>,
    failed: Option<chda_core::SavedSession>,
    presence: Option<bool>,
    running: bool,
    error: Option<String>,
}
#[derive(Default)]
pub(super) struct LayoutQueue(Mutex<Pending>, Condvar);

impl Client {
    pub fn queue_layout(&self, layout: chda_core::SavedSession) {
        let mut pending = self.layout.0.lock().unwrap();
        pending.failed = None;
        pending.layout = Some(layout);
        drop(pending);
        self.save_pending();
    }
    pub(super) fn retry_layout(&self) {
        let mut pending = self.layout.0.lock().unwrap();
        if pending.layout.is_some() || pending.running {
            return;
        }
        let Some(layout) = pending.failed.take() else {
            return;
        };
        pending.layout = Some(layout);
        drop(pending);
        self.save_pending();
    }
    pub fn queue_gui_available(&self, available: bool) {
        self.gui_available
            .store(available, std::sync::atomic::Ordering::Release);
        self.layout.0.lock().unwrap().presence = Some(available);
        self.save_pending();
    }
    pub fn layout_error(&self) -> Option<String> {
        self.layout.0.lock().unwrap().error.clone()
    }
    /// Call from a background quit task, after queueing the final snapshot.
    pub fn flush_layout(&self) -> io::Result<()> {
        self.retry_layout();
        let mut pending = self.layout.0.lock().unwrap();
        while pending.running {
            pending = self.layout.1.wait(pending).unwrap();
        }
        match &pending.error {
            Some(error) => Err(io::Error::other(error.clone())),
            None => Ok(()),
        }
    }
    fn save_pending(&self) {
        let mut pending = self.layout.0.lock().unwrap();
        if pending.running {
            return;
        }
        pending.running = true;
        let client = self.clone();
        // A worker exists only while work remains, so it cannot keep its own
        // Client/queue alive forever after the last GUI lease disappears.
        std::thread::spawn(move || {
            loop {
                let (layout, presence) = {
                    let mut pending = client.layout.0.lock().unwrap();
                    if pending.layout.is_none() && pending.presence.is_none() {
                        pending.running = false;
                        client.layout.1.notify_all();
                        return;
                    }
                    (pending.layout.take(), pending.presence.take())
                };
                let retry = layout.clone();
                let saving_layout = layout.is_some();
                let result = (|| {
                    if let Some(available) = presence {
                        client.request(Request::GuiPresence(available))?;
                    }
                    if let Some(layout) = layout {
                        let state = client.checkpoint(Some(layout))?;
                        if let Some(error) = state.save_error {
                            return Err(io::Error::other(error));
                        }
                    }
                    Ok::<_, io::Error>(())
                })();
                let mut pending = client.layout.0.lock().unwrap();
                match result {
                    Err(error) => {
                        if unavailable(&error) && pending.layout.is_none() {
                            pending.failed = retry;
                        }
                        pending.error = Some(error.to_string());
                    }
                    Ok(()) if saving_layout => pending.error = None,
                    Ok(()) => {}
                }
            }
        });
    }
}
