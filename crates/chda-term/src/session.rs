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

use chda_pty::{ExitStatus, Pty, PtySize, SpawnOptions};

use crate::frame::{ColorConfig, Frame, Size};
use crate::input::{KeyCode, KeyInput, Modifiers, MouseInput};
use crate::vt::Terminal;

/// Default scrollback kept per session, in bytes (Ghostty's default).
///
/// libghostty's header calls `max_scrollback` a line count, but Ghostty
/// passes its byte-sized `scrollback-limit` straight through and the screen
/// treats it as bytes.
pub const DEFAULT_SCROLLBACK: usize = 50_000_000;

/// Shortest interval between two frame rebuilds while output is streaming.
const FRAME_INTERVAL: Duration = Duration::from_millis(8);

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
    /// The child exited; the session is finished.
    Exited(ExitStatus),
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
    Mouse(MouseInput),
    CopySelection,
    QueryCwd,
}

enum Msg {
    Output(Vec<u8>),
    OutputClosed,
    Command(Command),
}

/// Handle to a running session. Dropping it ends the child process.
pub struct Session {
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
        let reader = pty.reader()?;

        let (tx, rx) = mpsc::channel();
        let frame = Arc::new(Mutex::new(Arc::new(Frame::default())));

        spawn_reader(reader, tx.clone());
        let thread = {
            let frame = Arc::clone(&frame);
            thread::Builder::new()
                .name("chda-term".into())
                .spawn(move || run(opts, pty, rx, frame, events, wake))?
        };

        Ok(Self {
            tx,
            frame,
            thread: Some(thread),
        })
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

    /// Ask the OS for the foreground process's working directory; it
    /// arrives as [`Event::Cwd`]. For shells without OSC 7.
    pub fn query_cwd(&self) {
        self.send(Command::QueryCwd);
    }

    pub fn scroll_to_bottom(&self) {
        self.send(Command::ScrollToBottom);
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

fn spawn_reader(mut reader: Box<dyn Read + Send>, tx: Sender<Msg>) {
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
    mut pty: Pty,
    rx: Receiver<Msg>,
    frame_slot: Arc<Mutex<Arc<Frame>>>,
    events: Sender<Event>,
    wake: impl Fn(),
) {
    let mut term = match Terminal::new(opts.size, opts.scrollback) {
        Ok(t) => t,
        Err(_) => {
            let _ = pty.kill();
            return;
        }
    };
    let _ = term.set_colors(&opts.colors);
    let emit = |event: Event| {
        let _ = events.send(event);
        wake();
    };

    let mut last_frame = Instant::now() - FRAME_INTERVAL;
    let mut dirty = true;
    let mut closed = false;
    let mut replies: Vec<Event> = Vec::new();

    loop {
        // Block for the first message, then drain whatever else is queued so
        // a burst of output is parsed in one go before we draw.
        let timeout = if dirty {
            FRAME_INTERVAL.saturating_sub(last_frame.elapsed())
        } else {
            Duration::from_secs(3600)
        };
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
                }
                Msg::OutputClosed => {
                    closed = true;
                }
                Msg::Command(cmd) => {
                    if handle_command(&mut term, &mut pty, cmd, &mut replies) {
                        dirty = true;
                    }
                }
            }
        }

        let fx = term.take_effects();
        if !fx.pty_output.is_empty() {
            let _ = pty.write_all(&fx.pty_output);
        }
        // Publish the frame before the events so handlers see current state.
        let force_frame = fx.title_changed || fx.pwd_changed || closed;
        if dirty && (force_frame || last_frame.elapsed() >= FRAME_INTERVAL) {
            if let Ok(frame) = term.frame() {
                *frame_slot.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(frame);
                emit(Event::Frame);
            }
            last_frame = Instant::now();
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

        if closed {
            break;
        }
    }

    let status = match pty.try_wait() {
        Ok(Some(s)) => s,
        _ => {
            let _ = pty.kill();
            pty.wait().unwrap_or(ExitStatus {
                code: 0,
                signal: None,
            })
        }
    };
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
        Command::QueryCwd => {
            replies.push(Event::Cwd(pty.foreground_cwd()));
            false
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::input::{KeyAction, KeyCode, Modifiers};

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
            Event::Exited(ExitStatus {
                code: 0,
                signal: None
            })
        );
    }

    #[test]
    fn zsh_integration_reports_cwd_and_prompt_marks() {
        if !std::path::Path::new("/bin/zsh").exists() {
            return;
        }
        let home = std::env::temp_dir().join(format!("chda-zsh-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let mut env = SessionOptions::default().env;
        env.push(("HOME".into(), home.to_string_lossy().into_owned()));
        env.extend(
            crate::env_for(
                crate::ShellIntegration::Detect,
                std::path::Path::new("/bin/zsh"),
                &home,
            )
            .unwrap(),
        );
        let mut p = Probe::spawn_with(SessionOptions {
            command: Some(vec!["/bin/zsh".into(), "-i".into()]),
            cwd: Some(std::path::PathBuf::from("/private/tmp")),
            env,
            ..Default::default()
        });
        let ev = p.wait_for(|e| matches!(e, Event::PwdChanged(_)));
        assert_eq!(
            ev,
            Event::PwdChanged(format!("file://{}/private/tmp", hostname()))
        );
        let frame = p.wait_for_text("%");
        assert_eq!(frame.rows[0].semantic_prompt, crate::SemanticPrompt::Prompt);
        p.session.text("exit\n".into());
        p.wait_for(|e| matches!(e, Event::Exited(_)));
        std::fs::remove_dir_all(&home).unwrap();
    }

    fn hostname() -> String {
        String::from_utf8(
            std::process::Command::new("hostname")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned()
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
}
