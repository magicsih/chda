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
use crate::platform::{FolderApp, NotificationTarget};
use crate::terminal_view::TerminalView;
use crate::workspace_view::WorkspaceView;

/// What the window asked the OS to do.
#[derive(Default)]
pub struct Recorded {
    pub notifications: Vec<(String, String, NotificationTarget)>,
    pub badge: usize,
    pub beeps: usize,
    pub click: Option<Box<dyn Fn(NotificationTarget)>>,
    pub opened_files: Vec<PathBuf>,
    pub revealed: Vec<PathBuf>,
    /// Folders opened from the title bar, with the app's id.
    pub opened_in: Vec<(String, PathBuf)>,
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

    fn open_file(&self, path: &Path, _: &App) {
        self.0.borrow_mut().opened_files.push(path.to_path_buf());
    }

    fn reveal_path(&self, path: &Path, _: &App) {
        self.0.borrow_mut().revealed.push(path.to_path_buf());
    }

    /// Two apps, as if installed.
    fn folder_apps(&self) -> Vec<FolderApp> {
        [("finder", "Finder"), ("vscode", "VS Code")]
            .into_iter()
            .map(|(id, name)| FolderApp {
                id: id.into(),
                name: name.into(),
                icon: None,
            })
            .collect()
    }

    fn open_folder_in(&self, id: &str, folder: &Path) -> std::io::Result<()> {
        self.0
            .borrow_mut()
            .opened_in
            .push((id.into(), folder.to_path_buf()));
        Ok(())
    }
}

/// A real adapter that reads transcripts only from the test home: Claude
/// Code's from a test folder, the others' not at all. Claude Code counts as
/// installed when the home has a fake `claude` script ([`Home::fake_claude`]),
/// which then runs in its place. Codex can use a stand-in on the pane PATH;
/// the other agents are never installed.
struct TestAdapter {
    inner: Box<dyn AgentAdapter>,
    projects: Option<PathBuf>,
    bin: Option<PathBuf>,
}

impl AgentAdapter for TestAdapter {
    fn id(&self) -> AgentId {
        self.inner.id()
    }
    fn display_name(&self) -> &str {
        self.inner.display_name()
    }
    fn short_label(&self) -> String {
        self.inner.short_label()
    }
    fn is_installed(&self, path: &std::ffi::OsStr) -> bool {
        self.bin.as_ref().is_some_and(|b| b.is_file())
            && (self.inner.id() != AgentId::Codex || self.inner.is_installed(path))
    }
    fn launch_command(
        &self,
        cwd: &Path,
        resume: Option<&SessionId>,
        hook_bin: &Path,
    ) -> std::process::Command {
        let real = self.inner.launch_command(cwd, resume, hook_bin);
        let Some(bin) = &self.bin else {
            return real;
        };
        let program = if self.inner.id() == AgentId::Codex {
            Path::new("codex")
        } else {
            bin
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(real.get_args()).current_dir(cwd);
        cmd
    }
    fn session_roots(&self) -> Vec<PathBuf> {
        self.projects.clone().into_iter().collect()
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
    /// Where [`Home::fake_claude`] puts its script.
    pub claude_bin: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub ghostty: PathBuf,
    /// Where the setup may write stand-ins for `gh`, `glab`, `tea` and `codex`;
    /// without them pull request badges are off.
    pub bin: PathBuf,
    /// What "GitHub" answers for the latest release; absent: offline.
    /// Each request adds a line to [`Home::release_requests`].
    pub latest_release: PathBuf,
}

impl Home {
    /// How many times the window asked for the latest release.
    pub fn release_requests(&self) -> usize {
        std::fs::read_to_string(self.latest_release.with_extension("requests"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

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

    /// Install a stand-in `claude` that prints its arguments and waits.
    pub fn fake_claude(&self) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(self.claude_bin.parent().unwrap()).unwrap();
        std::fs::write(
            &self.claude_bin,
            "#!/bin/sh\necho \"fake-claude $*\"\nexec cat >/dev/null\n",
        )
        .unwrap();
        std::fs::set_permissions(&self.claude_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
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
            claude_bin: root.0.join("bin/claude"),
            config: root.0.join("home/.config/chda/config.toml"),
            data: root.0.join("data"),
            ghostty: root.0.join("home/.config/ghostty/config"),
            bin: root.0.join("bin"),
            latest_release: root.0.join("latest-release.json"),
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
        // Ghostty's file names and order under the temporary home; no theme
        // directories, so only bundled themes resolve.
        let paths = Paths {
            config_files: Paths::under(Some(home.home.join(".config")), Some(home.home.clone()))
                .config_files,
            theme_dirs: Vec::new(),
        };
        let mut pane_env = vec![
            ("HOME".into(), home.home.to_string_lossy().into_owned()),
            ("ZDOTDIR".into(), home.home.to_string_lossy().into_owned()),
        ];
        // Tests may model a Dock launch without mutating the test process PATH.
        if let Ok(path) = std::fs::read_to_string(home.home.join("gui-path")) {
            pane_env.push(("PATH".into(), path));
            pane_env.extend(crate::environment::shell_path_env(
                Path::new("/bin/zsh"),
                &home.home,
                &pane_env,
            ));
        }
        let env = Rc::new(Environment {
            config_path: Some(home.config.clone()),
            data_dir: Some(home.data.clone()),
            ghostty: paths.clone(),
            shell: Some(PathBuf::from("/bin/zsh")),
            pane_env,
            adapters: Arc::new(
                adapters()
                    .into_iter()
                    .map(|inner| {
                        let claude = inner.id() == AgentId::Claude;
                        let codex = inner.id() == AgentId::Codex;
                        Box::new(TestAdapter {
                            inner,
                            projects: claude.then(|| home.claude_projects.clone()),
                            bin: if claude {
                                Some(home.claude_bin.clone())
                            } else if codex {
                                Some(home.bin.join("codex"))
                            } else {
                                None
                            },
                        }) as Box<dyn AgentAdapter>
                    })
                    .collect(),
            ),
            forge_clis: chda_core::ForgeClis {
                gh: home.bin.join("gh"),
                glab: home.bin.join("glab"),
                tea: home.bin.join("tea"),
            },
            system,
            latest_release: {
                let file = home.latest_release.clone();
                Arc::new(move || {
                    use std::io::Write;
                    let mut log = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(file.with_extension("requests"))
                        .unwrap();
                    writeln!(log, "request").unwrap();
                    std::fs::read_to_string(&file).ok()
                })
            },
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
        wait_until(&mut self.cx, &self.view, what, done);
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
        self.hook_session(pane, cwd, kind, "s");
    }

    /// [`Harness::hook`] for a given Claude Code session id.
    pub fn hook_session(&self, pane: Option<u64>, cwd: &Path, kind: HookKind, session: &str) {
        self.hook_from("claude", pane, cwd, kind, session);
    }

    /// [`Harness::hook`] for the agent with id `agent` and a session id.
    pub fn hook_from(
        &self,
        agent: &str,
        pane: Option<u64>,
        cwd: &Path,
        kind: HookKind,
        session: &str,
    ) {
        let event = HookEvent {
            agent: agent.into(),
            session_id: session.into(),
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

/// Run a window until `done` holds, letting shells and watchers work.
pub fn wait_until(
    cx: &mut VisualTestContext,
    view: &Entity<WorkspaceView>,
    what: &str,
    done: impl Fn(&WorkspaceView, &App) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |v, cx| done(v, cx)) {
            return;
        }
        if Instant::now() >= deadline {
            let screen = view.read_with(cx, |v, cx| v.focused_text(cx));
            panic!("timed out waiting for {what}; focused pane shows:\n{screen}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
