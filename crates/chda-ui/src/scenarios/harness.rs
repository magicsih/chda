//! Opens a window over a temporary home and waits for real shells.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chda_config::{GhosttyConfig, Paths};
use chda_core::SavedWindow;
use chda_core::agents::{AgentAdapter, AgentId, HookEvent, HookKind, SessionId, adapters, ipc};
use gpui::{App, AppContext, Entity, TestAppContext, VisualTestContext, WindowOptions};

use crate::environment::{Environment, System};
use crate::platform::NotificationTarget;
use crate::terminal_view::TerminalView;
use crate::workspace_view::WorkspaceView;

/// What the window asked the OS to do.
#[derive(Default)]
pub struct Recorded {
    pub notifications: Vec<(String, String, NotificationTarget)>,
    pub badge: usize,
    pub beeps: usize,
    pub click: Option<Box<dyn Fn(NotificationTarget)>>,
}

#[derive(Default)]
pub struct RecordingSystem(pub RefCell<Recorded>);

impl System for RecordingSystem {
    fn notify(&self, title: &str, body: &str, target: &NotificationTarget) {
        self.0
            .borrow_mut()
            .notifications
            .push((title.into(), body.into(), target.clone()));
    }

    fn set_badge(&self, count: usize) {
        self.0.borrow_mut().badge = count;
    }

    fn beep(&self) {
        self.0.borrow_mut().beeps += 1;
    }

    fn restore_windows(&self) {}

    fn pane_locale(&self) -> Option<String> {
        Some("en_US.UTF-8".into())
    }

    fn on_notification_click(&self, handler: Box<dyn Fn(NotificationTarget)>) {
        self.0.borrow_mut().click = Some(handler);
    }
}

/// The real Claude Code adapter with transcripts read from a test folder.
struct TestClaude {
    inner: Box<dyn AgentAdapter>,
    projects: PathBuf,
}

impl AgentAdapter for TestClaude {
    fn id(&self) -> AgentId {
        self.inner.id()
    }
    fn display_name(&self) -> &str {
        self.inner.display_name()
    }
    fn is_installed(&self) -> bool {
        false
    }
    fn launch_command(
        &self,
        cwd: &Path,
        resume: Option<&SessionId>,
        hook_bin: &Path,
    ) -> std::process::Command {
        self.inner.launch_command(cwd, resume, hook_bin)
    }
    fn session_roots(&self) -> Vec<PathBuf> {
        vec![self.projects.clone()]
    }
    fn parse_session(&self, file: &Path) -> Option<chda_core::agents::AgentSession> {
        self.inner.parse_session(file)
    }
    fn install_hooks(
        &self,
        data_dir: &Path,
        hook_bin: &Path,
    ) -> std::io::Result<chda_core::agents::HookInstallReport> {
        self.inner.install_hooks(data_dir, hook_bin)
    }
}

/// A temporary directory removed when the test ends. Kept short: the hook
/// socket inside it must fit macOS's 104-byte socket path limit.
pub struct TempRoot(pub PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cs-{name}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.canonicalize().unwrap())
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Harness {
    pub cx: VisualTestContext,
    pub view: Entity<WorkspaceView>,
    pub system: Rc<RecordingSystem>,
    pub home: Home,
    app: TestAppContext,
    _root: TempRoot,
}

/// Paths of the temporary home, available before the window opens.
pub struct Home {
    pub home: PathBuf,
    /// Claude Code transcripts the window indexes (`~/.claude/projects`).
    pub claude_projects: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub ghostty: PathBuf,
}

impl Home {
    /// Write a Claude Code transcript for a session that ran in `cwd`.
    pub fn claude_session(&self, cwd: &Path, id: &str, prompt: &str) {
        let dir = self.claude_projects.join("project");
        std::fs::create_dir_all(&dir).unwrap();
        let line = serde_json::json!({
            "type": "user",
            "sessionId": id,
            "cwd": cwd,
            "timestamp": "2026-10-01T10:00:00.000Z",
            "message": {"role": "user", "content": prompt},
        });
        std::fs::write(dir.join(format!("{id}.jsonl")), format!("{line}\n")).unwrap();
    }

    /// Create a git repository with one commit under the home.
    pub fn repo(&self, name: &str) -> PathBuf {
        let repo = self.home.join("src").join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        repo
    }
}

pub fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

impl Harness {
    /// Open a window over a fresh temporary home; `setup` can write config
    /// files or repositories first.
    pub fn open(cx: &mut TestAppContext, name: &str, setup: impl FnOnce(&Home)) -> Self {
        let root = TempRoot::new(name);
        let home = Home {
            home: root.0.join("home"),
            claude_projects: root.0.join("claude-projects"),
            config: root.0.join("home/.config/chda/config.toml"),
            data: root.0.join("data"),
            ghostty: root.0.join("home/.config/ghostty/config"),
        };
        std::fs::create_dir_all(home.home.join(".config/chda")).unwrap();
        std::fs::create_dir_all(home.home.join(".config/ghostty")).unwrap();
        std::fs::write(
            home.home.join(".zshrc"),
            "PS1='test%# '\nPROMPT_EOL_MARK=''\n",
        )
        .unwrap();
        setup(&home);
        Self::open_home(cx, root, home)
    }

    fn open_home(cx: &mut TestAppContext, root: TempRoot, home: Home) -> Self {
        cx.executor().allow_parking();
        cx.update(|cx| cx.bind_keys(crate::key_bindings()));
        let system = Rc::new(RecordingSystem::default());
        let (cx2, view) = open_window(cx, &home, system.clone(), None);
        Self {
            cx: cx2,
            view,
            system,
            home,
            app: cx.clone(),
            _root: root,
        }
    }

    /// Open a second window from the session the first one saved, as the
    /// next launch would (sharing the same home).
    pub fn reopen(&mut self) -> (VisualTestContext, Entity<WorkspaceView>) {
        let saved = SavedWindow::load(&self.home.data);
        assert!(saved.is_some(), "no saved session");
        let system = Rc::new(RecordingSystem::default());
        open_window(&mut self.app, &self.home, system, saved)
    }
}

fn open_window(
    cx: &mut TestAppContext,
    home: &Home,
    system: Rc<RecordingSystem>,
    saved: Option<SavedWindow>,
) -> (VisualTestContext, Entity<WorkspaceView>) {
    {
        let paths = Paths {
            config_files: vec![home.ghostty.clone()],
            theme_dirs: Vec::new(),
        };
        let env = Rc::new(Environment {
            config_path: Some(home.config.clone()),
            data_dir: Some(home.data.clone()),
            ghostty: paths.clone(),
            shell: Some(PathBuf::from("/bin/zsh")),
            pane_env: vec![
                ("HOME".into(), home.home.to_string_lossy().into_owned()),
                ("ZDOTDIR".into(), home.home.to_string_lossy().into_owned()),
            ],
            adapters: Arc::new(vec![Box::new(TestClaude {
                inner: adapters()
                    .into_iter()
                    .find(|a| a.id() == AgentId::Claude)
                    .unwrap(),
                projects: home.claude_projects.clone(),
            }) as Box<dyn AgentAdapter>]),
            use_gh: false,
            system,
        });
        let ghostty: GhosttyConfig = chda_config::load(&paths, None);
        let window = cx.update(|cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                cx.new(|cx| WorkspaceView::new(ghostty, saved, env, window, cx))
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let view = window.root(&mut cx).unwrap();
        cx.update(|window, cx| view.update(cx, |v, cx| v.focus(window, cx)));
        (cx, view)
    }
}

impl Harness {
    /// Run the window until `done` holds, letting shells and watchers work.
    pub fn wait_for(&mut self, what: &str, done: impl Fn(&WorkspaceView, &App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.cx.run_until_parked();
            if self.view.read_with(&self.cx, |v, cx| done(v, cx)) {
                return;
            }
            if Instant::now() >= deadline {
                let screen = self.view.read_with(&self.cx, |v, cx| v.focused_text(cx));
                panic!("timed out waiting for {what}; focused pane shows:\n{screen}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn read<R>(&self, f: impl FnOnce(&WorkspaceView, &App) -> R) -> R {
        self.view.read_with(&self.cx, f)
    }

    /// Type key bindings, e.g. `"cmd-t"` or `"cmd-shift-a"`.
    pub fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
        self.cx.run_until_parked();
    }

    /// Type text into whatever has focus, as committed input.
    pub fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(text);
        self.cx.run_until_parked();
    }

    /// Wait for the focused pane's shell prompt.
    pub fn wait_prompt(&mut self) {
        self.wait_for("a shell prompt", |v, cx| {
            v.focused_text(cx).contains("test%")
        });
    }

    /// Run a command in the focused pane and wait until `expect` shows.
    pub fn run(&mut self, command: &str, expect: &str) {
        self.type_text(command);
        self.keys("enter");
        let expect = expect.to_owned();
        self.wait_for(&format!("{expect:?} on screen"), move |v, cx| {
            v.focused_text(cx).contains(&expect)
        });
    }

    pub fn focused_terminal(&self) -> Entity<TerminalView> {
        self.read(|v, _| {
            let pane = v.ws.focused_pane().unwrap();
            v.panes[&pane].0.clone()
        })
    }

    /// Send an agent hook event through the app's socket, as `chda hook`
    /// would from inside a pane.
    pub fn hook(&self, pane: Option<u64>, cwd: &Path, kind: HookKind) {
        let event = HookEvent {
            agent: "claude".into(),
            session_id: "s".into(),
            cwd: cwd.to_path_buf(),
            kind,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            pane,
        };
        ipc::send(&ipc::socket_path(&self.home.data), &event).unwrap();
    }
}
