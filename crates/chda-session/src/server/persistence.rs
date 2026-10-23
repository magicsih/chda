//! One ordered writer. Runtime locks cover queueing and completed state only.
use super::*;
use std::sync::Condvar;

pub(super) enum Text {
    Retain,
    Capture(mpsc::Receiver<Result<String, String>>),
    Final(String),
}
struct Save {
    record: SessionRecord,
    text: Text,
    captured_revision: Option<u64>,
}
enum Work {
    Session(Box<Save>),
    Layout(chda_core::SavedSession),
    Barrier(mpsc::Sender<()>),
}
#[derive(Default)]
struct Queue {
    work: VecDeque<Work>,
    busy: bool,
    stopped: bool,
}
pub(super) enum Saved {
    Session {
        identity: Identity,
        output: Option<String>,
        captured_revision: Option<u64>,
        saved_at: Option<u64>,
        error: Option<String>,
    },
    Layout {
        saved_at: Option<u64>,
        error: Option<String>,
    },
}
pub(super) struct Persistence {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    pub completed: mpsc::Receiver<Saved>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Persistence {
    pub fn new(store: Store) -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let pending = queue.clone();
        let (tx, completed) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("chda-session-save".into())
            .spawn(move || {
                let (lock, wake) = &*pending;
                loop {
                    let work = {
                        let mut q = lock.lock().unwrap();
                        while q.work.is_empty() && !q.stopped {
                            q = wake.wait(q).unwrap();
                        }
                        let Some(work) = q.work.pop_front() else {
                            return;
                        };
                        q.busy = true;
                        work
                    };
                    match work {
                        Work::Session(save) => {
                            let _ = tx.send(save_session(&store, *save));
                        }
                        Work::Layout(layout) => {
                            let result = store.save_layout(&layout);
                            let _ = tx.send(Saved::Layout {
                                saved_at: result.as_ref().ok().map(|_| protocol::now()),
                                error: result.err().map(|e| e.to_string()),
                            });
                        }
                        Work::Barrier(done) => {
                            let _ = done.send(());
                        }
                    }
                    lock.lock().unwrap().busy = false;
                }
            })
            .expect("start checkpoint writer");
        Self {
            queue,
            completed,
            thread: Some(thread),
        }
    }
    pub fn session(&self, record: SessionRecord, text: Text, revision: u64) {
        let mut save = Box::new(Save {
            record,
            captured_revision: (!matches!(text, Text::Retain)).then_some(revision),
            text,
        });
        let mut queue = self.queue.0.lock().unwrap();
        if let Some(Work::Session(previous)) = queue.work.iter_mut().find(|job| {
            matches!(job, Work::Session(s) if s.record.identity.session == save.record.identity.session)
        }) {
            // Metadata arriving while a capture waits must retain that capture,
            // but it cannot claim that later output was included in it.
            if matches!(save.text, Text::Retain) {
                save.text = std::mem::replace(&mut previous.text, Text::Retain);
                save.captured_revision = (previous.record.identity == save.record.identity)
                    .then_some(previous.captured_revision)
                    .flatten();
            }
            *previous = save;
        } else {
            queue.work.push_back(Work::Session(save));
        }
        self.queue.1.notify_one();
    }
    pub fn layout(&self, layout: chda_core::SavedSession) {
        let mut queue = self.queue.0.lock().unwrap();
        if let Some(Work::Layout(previous)) = queue
            .work
            .iter_mut()
            .find(|job| matches!(job, Work::Layout(_)))
        {
            *previous = layout;
        } else {
            queue.work.push_back(Work::Layout(layout));
        }
        self.queue.1.notify_one();
    }
    pub fn barrier(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel();
        self.queue
            .0
            .lock()
            .unwrap()
            .work
            .push_back(Work::Barrier(tx));
        self.queue.1.notify_one();
        rx
    }
    pub fn idle(&self) -> bool {
        let queue = self.queue.0.lock().unwrap();
        !queue.busy && queue.work.is_empty()
    }
}
impl Drop for Persistence {
    fn drop(&mut self) {
        self.queue.0.lock().unwrap().stopped = true;
        self.queue.1.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn save_session(store: &Store, save: Save) -> Saved {
    let old = store.session(&save.record.identity.session);
    let mut checkpoint = Checkpoint {
        record: save.record,
        output: String::new(),
    };
    if let Some(old) = old {
        checkpoint.output = old.output;
        checkpoint.record.saved_at = old.record.saved_at;
    }
    checkpoint.record.save_error = None;
    let text_saved = !matches!(save.text, Text::Retain);
    let capture = match save.text {
        Text::Retain => Ok(None),
        Text::Final(text) => Ok(Some(text)),
        Text::Capture(rx) => rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Could not capture terminal text".to_owned())
            .and_then(|text| text)
            .map(Some),
    };
    let mut output = None;
    let result = capture.and_then(|text| {
        if let Some(text) = text {
            checkpoint.output = text;
        }
        let result = store
            .checkpoint(&mut checkpoint, text_saved)
            .map_err(|e| e.to_string());
        if text_saved {
            output = Some(std::mem::take(&mut checkpoint.output));
        }
        result
    });
    Saved::Session {
        identity: checkpoint.record.identity,
        output,
        captured_revision: save.captured_revision,
        saved_at: checkpoint.record.saved_at,
        error: result.err(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_capture_bounds_pending_saves_and_metadata_keeps_the_latest_saved_text() {
        let data =
            std::env::temp_dir().join(format!("chda-writer-{}", chda_pty::local::token().unwrap()));
        std::fs::create_dir(&data).unwrap();
        let store = Store::new(&data).unwrap();
        let writer = Persistence::new(store.clone());
        let record = SessionRecord {
            pane: 1,
            identity: Identity {
                session: "a".repeat(32),
                run: "b".repeat(32),
                server: "c".repeat(32),
            },
            pid: Some(1),
            running: true,
            closed: false,
            title: String::new(),
            cwd: None,
            launch: None,
            agent: None,
            exit: None,
            saved_at: None,
            save_error: None,
            scrollback: 1000,
            legacy_hooks: false,
        };
        let (capture, text) = mpsc::channel();
        writer.session(record.clone(), Text::Capture(text), 1);
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if writer.queue.0.lock().unwrap().busy {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        for n in 0..100 {
            let mut record = record.clone();
            record.title = format!("metadata-{n}");
            writer.session(record, Text::Retain, n + 2);
            writer.layout(chda_core::SavedSession {
                active_window: n as usize,
                windows: vec![],
            });
        }
        assert_eq!(
            writer.queue.0.lock().unwrap().work.len(),
            2,
            "only one pending save per session and layout"
        );
        let saved = writer.barrier();
        assert!(
            saved.try_recv().is_err(),
            "a barrier cannot confirm unfinished capture"
        );
        capture.send(Ok("captured before metadata".into())).unwrap();
        saved.recv_timeout(Duration::from_secs(3)).unwrap();
        let checkpoint = store.session(&record.identity.session).unwrap();
        assert_eq!(checkpoint.output, "captured before metadata");
        assert_eq!(checkpoint.record.title, "metadata-99");
        assert!(checkpoint.record.saved_at.is_some());
        assert_eq!(store.layout().unwrap().active_window, 99);
        drop(writer);
        std::fs::remove_dir_all(data).unwrap();
    }
}
