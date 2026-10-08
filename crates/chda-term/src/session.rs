//! A terminal session: a PTY, a child process, and the thread that owns the
//! VT state. The UI only ever sees [`Frame`] snapshots and [`Event`]s.
//!
//! Two threads per session: a reader thread that blocks on the PTY and a
//! terminal thread that parses output, encodes input and builds frames.

use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chda_pty::{DetachedPty, ExitStatus, Pty, PtyReader, PtySize, ReaderStop, SpawnOptions};

use crate::frame::{ColorConfig, Frame, Size};
use crate::input::{KeyCode, KeyInput, Modifiers, MouseInput};
use crate::search::{SearchQuery, SearchStatus};
use crate::vt::Terminal;

/// Default scrollback kept per session, in bytes (Ghostty's default).
///
/// libghostty's header calls `max_scrollback` a line count, but Ghostty
/// passes its byte-sized `scrollback-limit` straight through and the screen
/// treats it as bytes.
pub const DEFAULT_SCROLLBACK: usize = 50_000_000;

/// Shortest interval between two frame rebuilds while output is streaming.
const FRAME_INTERVAL: Duration = Duration::from_millis(8);
/// Shortest interval between two re-runs of an active search while output
/// is streaming; each run scans the whole scrollback.
const SEARCH_INTERVAL: Duration = Duration::from_millis(250);

/// Something the UI should react to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A new frame is available from [`Session::frame`].
    Frame,
    TitleChanged(String),
    /// Raw OSC 7 value.
    PwdChanged(String),
    Bell,
    /// An application asked to set the clipboard (OSC 52).
    ClipboardWrite(String),
    /// Reply to [`Session::copy_selection`]; absent when nothing is selected.
    SelectionText(Option<String>),
    /// Reply to [`Session::query_cwd`]: the foreground process's directory.
    Cwd(Option<std::path::PathBuf>),
    /// A shell prompt appeared, so the previous command finished.
    PromptShown,
    /// The active search ran: after [`Session::search`], a step, or new output.
    Search(SearchStatus),
    /// Reply to [`Session::find_last_diagram`]: the last Mermaid source in
    /// the scrollback, if any.
    LastDiagram(Option<String>),
    /// Final screen and retained scrollback, only for opted-in managed runs.
    FinalOutput(Result<String, String>),
    /// The child exited; the session is finished. The status is unknown
    /// for a child adopted from an earlier chda process.
    Exited(Option<ExitStatus>),
}

/// Session configuration.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub size: Size,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    /// Scrollback limit in bytes. Zero disables scrollback.
    pub scrollback: usize,
    pub colors: ColorConfig,
    /// Program and arguments. `None` runs the login shell.
    pub command: Option<Vec<String>>,
    pub cwd: Option<std::path::PathBuf>,
    pub env: Vec<(String, String)>,
    pub capture_exit_output: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            size: Size { cols: 80, rows: 24 },
            cell_width_px: 0,
            cell_height_px: 0,
            scrollback: DEFAULT_SCROLLBACK,
            colors: ColorConfig::default(),
            command: None,
            cwd: None,
            capture_exit_output: false,
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
                ("TERM_PROGRAM".into(), "chda".into()),
            ],
        }
    }
}

enum Command {
    Key(KeyInput),
    Paste(String),
    Text(String),
    Bytes(Vec<u8>),
    Focus(bool),
    Resize {
        size: Size,
        cell_width_px: u32,
        cell_height_px: u32,
    },
    Scroll {
        lines: isize,
        mods: Modifiers,
        px_x: u32,
        px_y: u32,
    },
    ScrollToBottom,
    JumpToPrompt(i32),
    JumpToLastPrompt,
    Mouse(MouseInput),
    CopySelection,
    FindLastDiagram,
    QueryCwd,
    Search(Option<SearchQuery>),
    SearchStep {
        older: bool,
    },
    SetColors(ColorConfig),
    Detach(Sender<std::io::Result<DetachedSession>>),
    CommitAdoption,
}

/// A session let go by [`Session::detach`]: the PTY with its running child
/// and the terminal's content, for [`Session::adopt`] here or in a
/// successor process.
#[derive(Debug)]
pub struct DetachedSession {
    pub pty: DetachedPty,
    /// [`Terminal::snapshot`] of the screen and scrollback.
    pub snapshot: Vec<u8>,
    /// The terminal size the snapshot was taken at.
    pub size: Size,
}

enum Msg {
    Output(Vec<u8>),
    OutputClosed,
    Command(Command),
}

/// Handle to a running session. Dropping it ends the child process.
pub struct Session {
    child_pid: Option<u32>,
    tx: Sender<Msg>,
    frame: Arc<Mutex<Arc<Frame>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

impl Session {
    pub fn child_pid(&self) -> Option<u32> {
        self.child_pid
    }
    /// Spawn the child and start the threads. `events` receives every
    /// [`Event`]; `wake` is called after each event so an event loop that
    /// cannot block on the channel can poll it.
    pub fn spawn(
        opts: SessionOptions,
        events: Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let command: Option<Vec<&str>> = opts
            .command
            .as_ref()
            .map(|c| c.iter().map(String::as_str).collect());
        let env: Vec<(&str, &str)> = opts
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let pty_size = PtySize {
            cols: opts.size.cols,
            rows: opts.size.rows,
            pixel_width: (opts.cell_width_px * u32::from(opts.size.cols)) as u16,
            pixel_height: (opts.cell_height_px * u32::from(opts.size.rows)) as u16,
        };
        let pty = Pty::spawn(
            pty_size,
            SpawnOptions {
                command: command.as_deref(),
                cwd: opts.cwd.as_deref().map(Path::new),
                env: &env,
            },
        )?;
        Self::start(opts, pty, None, events, wake, false)
    }

    /// Continue a session another chda process (or this one) detached: the
    /// child keeps running and the screen and scrollback come back.
    pub fn adopt(
        opts: SessionOptions,
        detached: DetachedSession,
        events: Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let session = Self::prepare_adoption(opts, detached, events, wake)?;
        session.commit_adoption();
        Ok(session)
    }

    /// Build the terminal without consuming PTY output or owning the child.
    /// Until committed, dropping this session leaves recovery resources intact.
    /// Wait for the first `Event::Frame` before reporting preparation complete.
    pub fn prepare_adoption(
        opts: SessionOptions,
        detached: DetachedSession,
        events: Sender<Event>,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        let pty = Pty::adopt(detached.pty)?;
        let restore = Restore {
            snapshot: detached.snapshot,
            size: detached.size,
        };
        Self::start(opts, pty, Some(restore), events, wake, true)
    }

    /// Called only after the broker commits every prepared window.
    pub fn commit_adoption(&self) {
        self.send(Command::CommitAdoption);
    }

    fn start(
        opts: SessionOptions,
        pty: Pty,
        restore: Option<Restore>,
        events: Sender<Event>,
        wake: impl Fn() + Send + 'static,
        provisional: bool,
    ) -> std::io::Result<Self> {
        let child_pid = pty.pid();
        let reader = pty.reader()?;
        let stop = reader.stopper();

        let (tx, rx) = mpsc::channel();
        let frame = Arc::new(Mutex::new(Arc::new(Frame::default())));

        let pending_reader = if provisional {
            Some((reader, tx.clone()))
        } else {
            spawn_reader(reader, tx.clone());
            None
        };
        let thread = {
            let frame = Arc::clone(&frame);
            let output_tx = tx.clone();
            thread::Builder::new()
                .name("chda-term".into())
                .spawn(move || {
                    run(
                        opts,
                        Owned {
                            pty,
                            stop,
                            restore,
                            pending_reader,
                            output_tx,
                        },
                        rx,
                        frame,
                        events,
                        wake,
                    )
                })?
        };

        Ok(Self {
            child_pid,
            tx,
            frame,
            thread: Some(thread),
        })
    }

    /// Stop reading, capture the screen and let go of the PTY without
    /// ending the child. The reply arrives on the returned channel once the
    /// output read so far is parsed; the session then ends without an
    /// [`Event::Exited`]. Fails when the child already exited.
    pub fn detach(&self) -> Receiver<std::io::Result<DetachedSession>> {
        let (tx, rx) = mpsc::channel();
        self.send(Command::Detach(tx));
        rx
    }

    /// The latest frame. Cheap to call; the terminal thread swaps in new ones.
    pub fn frame(&self) -> Arc<Frame> {
        Arc::clone(&self.frame.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn key(&self, input: KeyInput) {
        self.send(Command::Key(input));
    }

    pub fn paste(&self, text: String) {
        self.send(Command::Paste(text));
    }

    /// Send typed or IME-committed text. Goes through the key encoder when
    /// the application enabled the kitty keyboard protocol.
    pub fn text(&self, text: String) {
        self.send(Command::Text(text));
    }

    /// Send raw bytes to the child.
    pub fn write(&self, bytes: Vec<u8>) {
        self.send(Command::Bytes(bytes));
    }

    pub fn focus(&self, focused: bool) {
        self.send(Command::Focus(focused));
    }

    pub fn resize(&self, size: Size, cell_width_px: u32, cell_height_px: u32) {
        self.send(Command::Resize {
            size,
            cell_width_px,
            cell_height_px,
        });
    }

    /// Wheel movement of `lines` rows; negative is towards history. Goes to
    /// the application when it tracks the mouse or runs full screen.
    pub fn scroll(&self, lines: isize, mods: Modifiers, px_x: u32, px_y: u32) {
        self.send(Command::Scroll {
            lines,
            mods,
            px_x,
            px_y,
        });
    }

    pub fn mouse(&self, input: MouseInput) {
        self.send(Command::Mouse(input));
    }

    /// Ask for the selected text; it arrives as [`Event::SelectionText`].
    pub fn copy_selection(&self) {
        self.send(Command::CopySelection);
    }

    /// Look for the last Mermaid diagram in the scrollback; the answer
    /// arrives as [`Event::LastDiagram`].
    pub fn find_last_diagram(&self) {
        self.send(Command::FindLastDiagram);
    }

    /// Ask the OS for the foreground process's working directory; it
    /// arrives as [`Event::Cwd`]. For shells without OSC 7.
    pub fn query_cwd(&self) {
        self.send(Command::QueryCwd);
    }

    /// Start, change or (with `None`) end a scrollback search. The newest
    /// match is scrolled into view; results arrive as [`Event::Search`] and
    /// highlights in the frames. The search re-runs as output arrives.
    pub fn search(&self, query: Option<SearchQuery>) {
        self.send(Command::Search(query));
    }

    /// Replace the default colors and cursor style (config reload).
    pub fn set_colors(&self, colors: ColorConfig) {
        self.send(Command::SetColors(colors));
    }

    /// Move to the next older (or newer) match and scroll it into view.
    pub fn search_step(&self, older: bool) {
        self.send(Command::SearchStep { older });
    }

    pub fn scroll_to_bottom(&self) {
        self.send(Command::ScrollToBottom);
    }

    /// Scroll so the last shell prompt sits at the top (bottom when the
    /// shell never marked one).
    pub fn jump_to_last_prompt(&self) {
        self.send(Command::JumpToLastPrompt);
    }

    /// Scroll to the previous (`-1`) or next (`1`) shell prompt.
    pub fn jump_to_prompt(&self, delta: i32) {
        self.send(Command::JumpToPrompt(delta));
    }

    fn send(&self, cmd: Command) {
        // A closed channel means the terminal thread is gone; nothing to do.
        let _ = self.tx.send(Msg::Command(cmd));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Closing our sender does not end the thread while the reader still
        // holds one, so signal explicitly.
        let _ = self.tx.send(Msg::OutputClosed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// What the terminal thread owns besides the VT state.
struct Owned {
    pty: Pty,
    stop: ReaderStop,
    restore: Option<Restore>,
    pending_reader: Option<(PtyReader, Sender<Msg>)>,
    output_tx: Sender<Msg>,
}

/// A detached session's content to show before new output.
struct Restore {
    snapshot: Vec<u8>,
    size: Size,
}

fn spawn_reader(mut reader: PtyReader, tx: Sender<Msg>) {
    let _ = thread::Builder::new()
        .name("chda-pty-read".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(Msg::Output(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = tx.send(Msg::OutputClosed);
        });
}

fn run(
    opts: SessionOptions,
    owned: Owned,
    rx: Receiver<Msg>,
    frame_slot: Arc<Mutex<Arc<Frame>>>,
    events: Sender<Event>,
    wake: impl Fn(),
) {
    let Owned {
        mut pty,
        mut stop,
        restore,
        mut pending_reader,
        output_tx,
    } = owned;
    let mut owns_child = pending_reader.is_none();
    let start_size = restore.as_ref().map_or(opts.size, |r| r.size);
    let mut term = match Terminal::new(start_size, opts.scrollback) {
        Ok(t) => t,
        Err(_) => {
            stop.stop();
            if owns_child {
                let _ = pty.kill();
            }
            return;
        }
    };
    let _ = term.set_colors(&opts.colors);
    if let Some(restore) = restore {
        term.feed(&restore.snapshot);
        // Rebuilding the screen is not news: no replies to the child, no
        // bell or prompt events.
        let _ = term.take_effects();
    }
    let mut detaching = None;
    let emit = |event: Event| {
        let _ = events.send(event);
        wake();
    };

    // Only frames that carried new output count against the frame interval:
    // a keystroke's own frame must not hold back the echo that follows it.
    let mut last_output_frame = Instant::now() - FRAME_INTERVAL;
    let mut output_pending = false;
    let mut dirty = true;
    let mut closed = false;
    let mut replies: Vec<Event> = Vec::new();
    // Output arrived since the active search last ran.
    let mut search_stale = false;
    let mut last_search = Instant::now();

    loop {
        // Block for the first message, then drain whatever else is queued so
        // a burst of output is parsed in one go before we draw.
        let mut timeout = if dirty {
            FRAME_INTERVAL.saturating_sub(last_output_frame.elapsed())
        } else {
            Duration::from_secs(3600)
        };
        if search_stale {
            timeout = timeout.min(SEARCH_INTERVAL.saturating_sub(last_search.elapsed()));
        }
        let first = match rx.recv_timeout(timeout) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        for msg in first.into_iter().chain(rx.try_iter()) {
            match msg {
                Msg::Output(bytes) => {
                    term.feed(&bytes);
                    dirty = true;
                    output_pending = true;
                    search_stale = true;
                }
                Msg::OutputClosed => {
                    closed = true;
                }
                Msg::Command(Command::CommitAdoption) => {
                    if let Some((reader, tx)) = pending_reader.take() {
                        owns_child = true;
                        if start_size != opts.size {
                            let _ = term.resize(opts.size, opts.cell_width_px, opts.cell_height_px);
                            let _ = pty.resize(PtySize {
                                cols: opts.size.cols,
                                rows: opts.size.rows,
                                pixel_width: (opts.cell_width_px * u32::from(opts.size.cols))
                                    as u16,
                                pixel_height: (opts.cell_height_px * u32::from(opts.size.rows))
                                    as u16,
                            });
                            dirty = true;
                        }
                        spawn_reader(reader, tx);
                    }
                }
                Msg::Command(_) if !owns_child => {}
                Msg::Command(Command::Detach(reply)) => {
                    // Reserve recovery resources before stopping the reader.
                    match pty
                        .detached_handle()
                        .and_then(|handle| pty.reader().map(|reader| (handle, reader)))
                    {
                        Ok((handle, reader)) => {
                            stop.stop();
                            detaching = Some((reply, handle, reader));
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                // Once detaching, nothing more reaches the child.
                Msg::Command(_) if detaching.is_some() => {}
                Msg::Command(cmd) => {
                    if matches!(cmd, Command::Search(_) | Command::SearchStep { .. }) {
                        search_stale = false;
                        last_search = Instant::now();
                    }
                    if handle_command(&mut term, &mut pty, cmd, &mut replies) {
                        dirty = true;
                    }
                }
            }
        }

        if search_stale && last_search.elapsed() >= SEARCH_INTERVAL {
            search_stale = false;
            last_search = Instant::now();
            if let Some(status) = term.refresh_search() {
                replies.push(Event::Search(status));
                dirty = true;
            }
        }

        if closed && let Some((reply, handle, reader)) = detaching.take() {
            match pty.try_wait() {
                Ok(None) => {
                    let size = term.size();
                    match term.snapshot().map_err(std::io::Error::other) {
                        Ok(snapshot) => {
                            let _ = reply.send(Ok(DetachedSession {
                                pty: handle,
                                snapshot,
                                size,
                            }));
                            return;
                        }
                        Err(error) => {
                            stop = reader.stopper();
                            spawn_reader(reader, output_tx.clone());
                            closed = false;
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                _ => {
                    let _ = reply.send(Err(std::io::Error::other("the program already exited")));
                }
            }
        }

        let fx = term.take_effects();
        if !fx.pty_output.is_empty() {
            let _ = pty.write_all(&fx.pty_output);
        }
        // Publish the frame before the events so handlers see current state.
        let force_frame = fx.title_changed || fx.pwd_changed || closed;
        let throttled = output_pending && last_output_frame.elapsed() < FRAME_INTERVAL;
        if dirty && (force_frame || !throttled) {
            if let Ok(frame) = term.frame() {
                *frame_slot.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(frame);
                emit(Event::Frame);
            }
            if output_pending {
                last_output_frame = Instant::now();
                output_pending = false;
            }
            dirty = false;
        }
        if fx.bell {
            emit(Event::Bell);
        }
        if fx.title_changed {
            emit(Event::TitleChanged(term.title()));
        }
        if fx.pwd_changed {
            emit(Event::PwdChanged(term.pwd()));
        }
        if let Some(text) = fx.clipboard_write {
            emit(Event::ClipboardWrite(text));
        }
        for reply in replies.drain(..) {
            emit(reply);
        }
        if term.take_effects().prompt_shown {
            emit(Event::PromptShown);
        }

        if closed {
            break;
        }
    }

    if !owns_child {
        stop.stop();
        return;
    }
    let status = match pty.try_wait() {
        Ok(Some(s)) => s,
        _ => {
            let _ = pty.kill();
            pty.wait().ok().flatten()
        }
    };
    // Keep draining while a shell flushes its PTY during exit. Stopping the
    // reader before wait can deadlock macOS tty teardown.
    stop.stop();
    if opts.capture_exit_output {
        emit(Event::FinalOutput(
            term.screen_text().map_err(|e| e.to_string()),
        ));
    }
    emit(Event::Exited(status));
}

/// Apply a command. Returns whether the viewport may have changed.
fn handle_command(
    term: &mut Terminal,
    pty: &mut Pty,
    cmd: Command,
    replies: &mut Vec<Event>,
) -> bool {
    match cmd {
        // The run loop takes this one before commands reach here.
        Command::Detach(_) | Command::CommitAdoption => false,
        Command::Key(input) => {
            if let Ok(bytes) = term.encode_key(&input)
                && !bytes.is_empty()
            {
                term.clear_selection();
                term.scroll_to_bottom();
                let _ = pty.write_all(&bytes);
                return true;
            }
            false
        }
        Command::Paste(text) => {
            if let Ok(bytes) = term.encode_paste(&text) {
                term.clear_selection();
                term.scroll_to_bottom();
                let _ = pty.write_all(&bytes);
                return true;
            }
            false
        }
        Command::Text(text) => {
            if let Ok(bytes) = term.encode_text(&text)
                && !bytes.is_empty()
            {
                term.clear_selection();
                term.scroll_to_bottom();
                let _ = pty.write_all(&bytes);
                return true;
            }
            false
        }
        Command::Bytes(bytes) => {
            let _ = pty.write_all(&bytes);
            false
        }
        Command::Focus(focused) => {
            let bytes = term.encode_focus(focused);
            if !bytes.is_empty() {
                let _ = pty.write_all(&bytes);
            }
            false
        }
        Command::Resize {
            size,
            cell_width_px,
            cell_height_px,
        } => {
            let _ = term.resize(size, cell_width_px, cell_height_px);
            let _ = pty.resize(PtySize {
                cols: size.cols,
                rows: size.rows,
                pixel_width: (cell_width_px * u32::from(size.cols)) as u16,
                pixel_height: (cell_height_px * u32::from(size.rows)) as u16,
            });
            true
        }
        Command::Scroll {
            lines,
            mods,
            px_x,
            px_y,
        } => {
            if lines == 0 {
                return false;
            }
            let up = lines < 0;
            if term.mouse_tracking() && !mods.shift {
                for _ in 0..lines.unsigned_abs() {
                    if let Ok(bytes) = term.encode_wheel(up, mods, px_x, px_y) {
                        let _ = pty.write_all(&bytes);
                    }
                }
                return false;
            }
            if term.alt_scroll() {
                let key = KeyInput {
                    action: crate::input::KeyAction::Press,
                    key: if up {
                        KeyCode::ArrowUp
                    } else {
                        KeyCode::ArrowDown
                    },
                    mods: Modifiers::default(),
                    text: None,
                };
                for _ in 0..lines.unsigned_abs() {
                    if let Ok(bytes) = term.encode_key(&key) {
                        let _ = pty.write_all(&bytes);
                    }
                }
                return false;
            }
            term.scroll(lines);
            true
        }
        Command::ScrollToBottom => {
            term.scroll_to_bottom();
            true
        }
        Command::JumpToPrompt(delta) => term.jump_to_prompt(delta),
        Command::JumpToLastPrompt => {
            term.jump_to_last_prompt();
            true
        }
        Command::Mouse(input) => {
            if term.mouse_tracking() && !input.mods.shift {
                if let Ok(bytes) = term.encode_mouse(&input)
                    && !bytes.is_empty()
                {
                    let _ = pty.write_all(&bytes);
                }
                return false;
            }
            if input.button != crate::input::MouseButton::Left {
                return false;
            }
            let _ = term.select(&input);
            true
        }
        Command::CopySelection => {
            replies.push(Event::SelectionText(term.selection_text().unwrap_or(None)));
            false
        }
        Command::FindLastDiagram => {
            replies.push(Event::LastDiagram(term.last_diagram()));
            false
        }
        Command::QueryCwd => {
            replies.push(Event::Cwd(pty.foreground_cwd()));
            false
        }
        Command::Search(query) => {
            replies.push(Event::Search(term.search(query)));
            true
        }
        Command::SearchStep { older } => {
            replies.push(Event::Search(term.search_step(older)));
            true
        }
        Command::SetColors(colors) => {
            let _ = term.set_colors(&colors);
            true
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::input::{KeyAction, KeyCode, Modifiers};
    use crate::shell_integration::test_shells::{bashes, fish, hostname, temp_home};

    /// Drives a session in tests and remembers every event it consumed.
    struct Probe {
        session: Session,
        events: Receiver<Event>,
        seen: Vec<Event>,
    }

    impl Probe {
        fn spawn(command: &[&str]) -> Self {
            Self::spawn_with(SessionOptions {
                command: Some(command.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            })
        }

        fn spawn_with(opts: SessionOptions) -> Self {
            let (tx, events) = mpsc::channel();
            let session = Session::spawn(
                SessionOptions {
                    size: Size { cols: 40, rows: 6 },
                    ..opts
                },
                tx,
                || {},
            )
            .unwrap();
            Self {
                session,
                events,
                seen: Vec::new(),
            }
        }

        fn wait_for(&mut self, pred: impl Fn(&Event) -> bool) -> Event {
            if let Some(ev) = self.seen.iter().find(|e| pred(e)) {
                return ev.clone();
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let ev = self
                    .events
                    .recv_timeout(left)
                    .unwrap_or_else(|e| panic!("waiting for event: {e}; seen {:?}", self.seen));
                self.seen.push(ev.clone());
                if pred(&ev) {
                    return ev;
                }
            }
        }

        fn wait_for_text(&mut self, needle: &str) -> Arc<Frame> {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let frame = self.session.frame();
                let rows: Vec<String> = (0..frame.size.rows).map(|y| frame.row_text(y)).collect();
                if rows.iter().any(|l| l.contains(needle)) {
                    return frame;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                let ev = self
                    .events
                    .recv_timeout(left)
                    .unwrap_or_else(|e| panic!("waiting for {needle:?}: {e}; rows {rows:?}"));
                self.seen.push(ev);
            }
        }

        fn key(&self, key: KeyCode, text: Option<&str>) {
            self.session.key(KeyInput {
                action: KeyAction::Press,
                key,
                mods: Modifiers::default(),
                text: text.map(str::to_owned),
            });
        }
    }

    #[test]
    fn rejected_image_handoff_keeps_the_terminal_and_child_running() {
        let mut p = Probe::spawn(&[
            "/bin/sh",
            "-c",
            r"printf '\033_Ga=T,f=32,s=1,v=1,q=2;/////w==\033\\'; echo before; read x; echo survived:$x; read y",
        ]);
        p.wait_for_text("before");
        let Err(error) = p
            .session
            .detach()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        else {
            panic!("image handoff must fail");
        };
        assert!(error.to_string().contains("inline images"));
        p.session.write(b"yes\n".to_vec());
        p.wait_for_text("survived:yes");
    }

    #[test]
    fn failed_preparation_keeps_child_output_for_recovery() {
        let mut original = Probe::spawn(&[
            "/bin/sh",
            "-c",
            "echo before; read x; echo recovered:$x; read y",
        ]);
        original.wait_for_text("before");
        let detached = original
            .session
            .detach()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let recovery = DetachedSession {
            pty: detached.pty.try_clone().unwrap(),
            snapshot: detached.snapshot.clone(),
            size: detached.size,
        };
        let (tx, rx) = mpsc::channel();
        let prepared =
            Session::prepare_adoption(SessionOptions::default(), detached, tx, || {}).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Frame
        );
        // Output while a successor is preparing must stay unread in the PTY.
        let mut writer = Pty::adopt(recovery.pty.try_clone().unwrap()).unwrap();
        writer.write_all(b"retained\n").unwrap();
        drop(prepared);
        assert!(
            writer.try_wait().unwrap().is_none(),
            "failed adoption killed the shell"
        );
        let (tx, events) = mpsc::channel();
        let session = Session::adopt(SessionOptions::default(), recovery, tx, || {}).unwrap();
        let mut recovered = Probe {
            session,
            events,
            seen: Vec::new(),
        };
        recovered.wait_for_text("recovered:retained");
        recovered.session.paste("done\n".into());
        recovered.wait_for(|e| matches!(e, Event::Exited(_)));
    }

    /// A detached session continues in a new one: the child never notices,
    /// the old screen is back, and output written meanwhile is not lost.
    #[test]
    fn detach_and_adopt_continue_the_same_child() {
        let mut p = Probe::spawn(&[
            "/bin/sh",
            "-c",
            "echo before; read x; echo after:$x; read y",
        ]);
        p.wait_for_text("before");
        let detached = p
            .session
            .detach()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let pid = detached.pty.pid();
        assert_eq!(p.session.child_pid(), Some(pid));
        drop(p);

        let (tx, events) = mpsc::channel();
        let session = Session::adopt(
            SessionOptions {
                size: Size { cols: 40, rows: 6 },
                ..Default::default()
            },
            detached,
            tx,
            || {},
        )
        .unwrap();
        let mut q = Probe {
            session,
            events,
            seen: Vec::new(),
        };
        q.wait_for_text("before");
        q.session.text("hi\r".into());
        q.wait_for_text("after:hi");
        q.session.text("\r".into());
        assert_eq!(
            q.wait_for(|e| matches!(e, Event::Exited(_))),
            Event::Exited(None)
        );
    }

    #[test]
    fn detaching_an_exited_session_fails() {
        let mut p = Probe::spawn(&["/bin/sh", "-c", "exit 0"]);
        p.wait_for(|e| matches!(e, Event::Exited(_)));
        let reply = p.session.detach().recv_timeout(Duration::from_secs(2));
        assert!(!matches!(reply, Ok(Ok(_))));
    }

    #[test]
    fn opted_in_exit_output_retains_scrollback_before_the_exit_event() {
        let mut p = Probe::spawn_with(SessionOptions {
            command: Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "i=0; while [ $i -lt 30 ]; do echo history-$i; i=$((i+1)); done; exit 7".into(),
            ]),
            capture_exit_output: true,
            ..Default::default()
        });
        p.wait_for(|e| matches!(e, Event::Exited(_)));
        let output = p
            .seen
            .iter()
            .find_map(|e| {
                if let Event::FinalOutput(text) = e {
                    Some(text)
                } else {
                    None
                }
            })
            .unwrap()
            .as_ref()
            .unwrap();
        assert!(
            output.contains("history-0"),
            "retains output above the viewport"
        );
        assert!(output.contains("history-29"), "retains final output");
        assert!(matches!(p.seen.last(),Some(Event::Exited(Some(status))) if status.code==7));
        let mut plain = Probe::spawn(&["/bin/sh", "-c", "echo plain; exit 0"]);
        plain.wait_for(|e| matches!(e, Event::Exited(_)));
        assert!(
            !plain
                .seen
                .iter()
                .any(|e| matches!(e, Event::FinalOutput(_))),
            "ordinary shells do not export history"
        );
    }

    #[test]
    fn shell_output_shows_up_in_frames_and_exit_is_reported() {
        let mut p = Probe::spawn(&["/bin/sh", "-c", "echo marker-1; read x; echo got:$x"]);
        p.wait_for_text("marker-1");
        p.key(KeyCode::Char('h'), Some("h"));
        p.key(KeyCode::Char('i'), Some("i"));
        p.key(KeyCode::Enter, None);
        p.wait_for_text("got:hi");
        let ev = p.wait_for(|e| matches!(e, Event::Exited(_)));
        assert_eq!(
            ev,
            Event::Exited(Some(ExitStatus {
                code: 0,
                signal: None
            }))
        );
    }

    /// Start `shell` the way chda does, with the integration loaded, `home`
    /// as HOME and /private/tmp as the working directory. `args` go before
    /// the integration's own arguments.
    fn integrated_shell(shell: &std::path::Path, args: &[&str], home: &std::path::Path) -> Probe {
        let launch = crate::launch_for(crate::ShellIntegration::Detect, shell, home).unwrap();
        let mut command = vec![shell.to_string_lossy().into_owned()];
        command.extend(args.iter().map(|a| a.to_string()));
        command.extend(launch.args);
        let mut env = SessionOptions::default().env;
        env.push(("HOME".into(), home.to_string_lossy().into_owned()));
        env.push(("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into()));
        env.extend(launch.env);
        Probe::spawn_with(SessionOptions {
            command: Some(command),
            cwd: Some(std::path::PathBuf::from("/private/tmp")),
            env,
            ..Default::default()
        })
    }

    /// The shell reported /private/tmp, and the row showing `prompt` is
    /// marked as a prompt.
    fn assert_integrated(p: &mut Probe, prompt: &str) {
        let ev = p.wait_for(|e| matches!(e, Event::PwdChanged(_)));
        assert_eq!(
            ev,
            Event::PwdChanged(format!("file://{}/private/tmp", hostname()))
        );
        let frame = p.wait_for_text(prompt);
        let row = (0..frame.size.rows)
            .find(|&y| frame.row_text(y).contains(prompt))
            .unwrap();
        assert_eq!(
            frame.rows[row as usize].semantic_prompt,
            crate::SemanticPrompt::Prompt
        );
        p.wait_for(|e| matches!(e, Event::PromptShown));
    }

    #[test]
    fn zsh_integration_reports_cwd_and_prompt_marks() {
        if !std::path::Path::new("/bin/zsh").exists() {
            return;
        }
        let home = temp_home("zsh");
        // A distinctive prompt: a bare "%" also matches zsh's partial-line
        // marker (PROMPT_SP), which is not a prompt row.
        std::fs::write(home.join(".zshrc"), "PROMPT_EOL_MARK=''\nPS1='chda%% '\n").unwrap();
        let mut p = integrated_shell(std::path::Path::new("/bin/zsh"), &["-i"], &home);
        assert_integrated(&mut p, "chda%");
        p.session.text("exit\n".into());
        p.wait_for(|e| matches!(e, Event::Exited(_)));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn bash_integration_reports_cwd_title_and_prompt_marks() {
        for bash in bashes() {
            let home = temp_home("bash");
            std::fs::write(home.join(".bash_profile"), "PS1='chda$ '\n").unwrap();
            let mut p = integrated_shell(&bash, &[], &home);
            assert_integrated(&mut p, "chda$");
            let ev = p.wait_for(|e| matches!(e, Event::TitleChanged(_)));
            assert_eq!(ev, Event::TitleChanged("/private/tmp".into()));
            p.session.text("exit\n".into());
            p.wait_for(|e| matches!(e, Event::Exited(_)));
            std::fs::remove_dir_all(&home).unwrap();
        }
    }

    #[test]
    fn fish_integration_reports_cwd_title_and_prompt_marks() {
        let Some(fish) = fish() else {
            return;
        };
        let home = temp_home("fish");
        let config = home.join(".config/fish");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.fish"),
            "set -g fish_greeting\nfunction fish_prompt; echo -n 'chda> '; end\n",
        )
        .unwrap();
        let mut p = integrated_shell(&fish, &["-l"], &home);
        assert_integrated(&mut p, "chda>");
        p.wait_for(|e| matches!(e, Event::TitleChanged(_)));
        // fish 4 turns on the kitty keyboard protocol, so Enter must be a
        // key press rather than a typed newline.
        p.session.text("exit".into());
        p.key(KeyCode::Enter, None);
        p.wait_for(|e| matches!(e, Event::Exited(_)));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn title_and_resize_round_trip() {
        let mut p = Probe::spawn(&[
            "/bin/sh",
            "-c",
            "printf '\\033]2;hello-title\\007'; read x; stty size",
        ]);
        let ev = p.wait_for(|e| matches!(e, Event::TitleChanged(_)));
        assert_eq!(ev, Event::TitleChanged("hello-title".into()));
        assert_eq!(p.session.frame().title, "hello-title");

        p.session.resize(Size { cols: 50, rows: 8 }, 0, 0);
        p.session.scroll(-1, Modifiers::default(), 0, 0);
        p.session.paste("\n".into());
        let frame = p.wait_for_text("8 50");
        assert_eq!(frame.size, Size { cols: 50, rows: 8 });
        p.wait_for(|e| matches!(e, Event::Exited(_)));
    }

    /// Keystroke-to-echo latency through a real zsh: time from sending text
    /// to the first frame that shows it. Run with
    /// `cargo test --release -p chda-term echo_latency -- --ignored --nocapture`.
    #[test]
    #[ignore = "latency measurement, not a check"]
    fn echo_latency() {
        for size in [
            Size { cols: 80, rows: 24 },
            Size {
                cols: 220,
                rows: 60,
            },
        ] {
            let (tx, rx) = mpsc::channel();
            let mut env = SessionOptions::default().env;
            env.push(("LANG".into(), "en_US.UTF-8".into()));
            let session = Session::spawn(
                SessionOptions {
                    size,
                    command: Some(vec!["/bin/zsh".into(), "-f".into(), "-i".into()]),
                    env,
                    ..Default::default()
                },
                tx,
                || {},
            )
            .unwrap();
            thread::sleep(Duration::from_millis(800));
            while rx.try_recv().is_ok() {}
            let mut ms = Vec::new();
            for ch in ["한", "글", "입", "력"].iter().cycle().take(20) {
                let start = Instant::now();
                session.text((*ch).to_owned());
                loop {
                    if let Event::Frame = rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                        let f = session.frame();
                        let y = f.cursor.map_or(0, |c| c.y);
                        if f.row_text(y).ends_with(ch) {
                            break;
                        }
                    }
                }
                ms.push(start.elapsed().as_secs_f64() * 1000.0);
                thread::sleep(Duration::from_millis(120));
            }
            ms.sort_by(f64::total_cmp);
            eprintln!(
                "{}x{}: echo median {:.2} ms, p90 {:.2} ms",
                size.cols, size.rows, ms[10], ms[18]
            );
        }
    }
}
