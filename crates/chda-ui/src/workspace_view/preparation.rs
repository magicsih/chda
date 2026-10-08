//! Checkout preparation never opens a terminal from a background completion.
use super::*;
use chda_config::PreparationPlan;
use chda_core::{AuthorizedPreparation, PreparationPreview, PreparationProgress};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) enum PreparationStage {
    Inspecting,
    Review,
    Running(PreparationProgress),
    Complete,
    Failed(String),
    Cancelled,
}
impl PreparationStage {
    fn busy(&self) -> bool {
        matches!(self, Self::Inspecting | Self::Running(_))
    }
    fn label(&self) -> String {
        match self {
            Self::Inspecting => "Checking preparation…".into(),
            Self::Review => "Confirm preparation".into(),
            Self::Running(PreparationProgress::Copying) => "Copying selected files…".into(),
            Self::Running(PreparationProgress::Command { index, total }) => {
                format!("Setup command {} of {total}", index + 1)
            }
            Self::Running(PreparationProgress::Complete) | Self::Complete => {
                "Preparation complete".into()
            }
            Self::Failed(e) => format!("Preparation stopped: {e}"),
            Self::Cancelled => "Cancelled — worktree and existing files kept".into(),
        }
    }
}
pub(crate) struct PreparationJob {
    id: u64,
    repo: PathBuf,
    path: PathBuf,
    plan: PreparationPlan,
    preview: Option<PreparationPreview>,
    pub stage: PreparationStage,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    open: bool,
    agent: Option<AgentId>,
}
impl Drop for PreparationJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
pub(super) struct PreparationEditor {
    repo: PathBuf,
    commands: Vec<(Entity<TextInput>, Subscription)>,
    files: Vec<(Entity<TextInput>, Subscription)>,
    error: Option<String>,
}
enum RunMessage {
    Progress(PreparationProgress),
    Done(Result<(), String>, bool),
}
struct WorkerFinished(Arc<AtomicBool>);
impl Drop for WorkerFinished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl WorkspaceView {
    pub(super) fn stop_preparations(&mut self, cx: &mut Context<Self>) -> Vec<Arc<AtomicBool>> {
        let paths: Vec<_> = self.preparation_jobs.keys().cloned().collect();
        let finished = self
            .preparation_jobs
            .values()
            .map(|job| {
                job.cancelled.store(true, Ordering::Release);
                job.finished.clone()
            })
            .collect();
        self.preparation_jobs.clear();
        self.preparation_open = None;
        self.preparation_editor = None;
        for path in paths {
            self.publish_preparation_guard(&path, false, cx);
        }
        finished
    }
    pub(super) fn require_preparation(
        &mut self,
        cwd: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let pending = self
            .env
            .windows
            .borrow()
            .preparations
            .iter()
            .find(|(root, owner)| cwd.starts_with(root) && owner.upgrade().is_some())
            .map(|(root, owner)| (root.clone(), owner.clone()));
        let Some((root, owner)) = pending else {
            return false;
        };
        if owner == self.self_weak {
            if let Some(path) = self
                .preparation_jobs
                .keys()
                .find(|p| p.canonicalize().ok().as_ref() == Some(&root))
                .cloned()
            {
                self.show_preparation(path, window, cx);
            }
        } else {
            self.notify_error(
                "Finish or explicitly skip this worktree's preparation in its original window"
                    .into(),
            );
            cx.notify();
        }
        true
    }
    pub(super) fn sync_preparation_busy(&mut self, cx: &mut Context<Self>) {
        let roots: Vec<_> = self
            .env
            .windows
            .borrow()
            .preparations
            .iter()
            .filter(|(_, owner)| owner.upgrade().is_some())
            .map(|(p, _)| p.clone())
            .collect();
        self.sidebar.update(cx, |s, cx| {
            for repo in &mut s.model.repos {
                for worktree in &mut repo.worktrees {
                    let path = worktree
                        .path
                        .canonicalize()
                        .unwrap_or_else(|_| worktree.path.clone());
                    if roots.contains(&path) {
                        worktree.busy = Some("preparing".into());
                    } else if worktree.busy.as_deref() == Some("preparing") {
                        worktree.busy = None;
                    }
                }
            }
            cx.notify();
        });
    }
    fn publish_preparation_guard(&mut self, path: &Path, active: bool, cx: &mut Context<Self>) {
        let root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut registry = self.env.windows.borrow_mut();
        if active {
            registry.preparations.insert(root, self.self_weak.clone());
        } else if registry.preparations.get(&root) == Some(&self.self_weak) {
            registry.preparations.remove(&root);
        }
        let entries = registry.entries.clone();
        drop(registry);
        self.sync_preparation_busy(cx);
        let own = self.self_weak.clone();
        cx.defer(move |cx| {
            for entry in entries {
                if entry.view != own {
                    let _ = entry.view.update(cx, |v, cx| v.sync_preparation_busy(cx));
                }
            }
        });
    }
    pub(super) fn edit_preparation(
        &mut self,
        repo: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preparation_editor.is_some() {
            return;
        }
        let plan = self
            .config
            .repo_preparation
            .get(&repo)
            .cloned()
            .unwrap_or_default();
        self.preparation_editor = Some(PreparationEditor {
            repo,
            commands: Vec::new(),
            files: Vec::new(),
            error: None,
        });
        self.preparation_open = None;
        for command in plan.commands {
            self.add_preparation_field(true, &command, window, cx);
        }
        for file in plan.files {
            self.add_preparation_field(false, &file.to_string_lossy(), window, cx);
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
    fn add_preparation_field(
        &mut self,
        command: bool,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let input = cx.new(|cx| {
            let mut input = TextInput::new(
                if command {
                    "setup command"
                } else {
                    "relative/path.env"
                },
                fg,
                blend(bg, fg, 0.12),
                cx,
            );
            // A field is one exact value, including intentional newlines or spaces.
            input.multiline = true;
            input.set_text(text, cx);
            input
        });
        let subscription = cx.subscribe_in(&input, window, |v, _, event, window, cx| {
            if matches!(event, TextInputEvent::Cancel) {
                v.close_preparation(window, cx);
            }
        });
        if let Some(editor) = &mut self.preparation_editor {
            let fields = if command {
                &mut editor.commands
            } else {
                &mut editor.files
            };
            fields.push((input.clone(), subscription));
            window.focus(&input.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }
    fn save_preparation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.preparation_editor else {
            return;
        };
        let plan = PreparationPlan {
            commands: editor
                .commands
                .iter()
                .map(|(i, _)| i.read(cx).text().to_owned())
                .collect(),
            files: editor
                .files
                .iter()
                .map(|(i, _)| PathBuf::from(i.read(cx).text()))
                .collect(),
        };
        let result = (|| {
            if plan
                .commands
                .iter()
                .any(|c| c.trim().is_empty() || c.contains('\0'))
                || plan.files.iter().any(|p| {
                    p.as_os_str().is_empty()
                        || p.components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                })
            {
                return Err("Complete or remove empty fields; files must be relative paths without traversal".to_owned());
            }
            let path = self
                .env
                .config_path
                .as_ref()
                .ok_or("The configuration location is unavailable")?;
            let mut config = self.config.clone();
            if plan.is_empty() {
                config.repo_preparation.remove(&editor.repo);
            } else {
                config.repo_preparation.insert(editor.repo.clone(), plan);
            }
            config.save(path).map_err(|e| e.to_string())?;
            Ok(config)
        })();
        match result {
            Ok(config) => {
                self.config = config;
                self.preparation_editor = None;
                self.focus_active(window, cx);
            }
            Err(e) => self.preparation_editor.as_mut().unwrap().error = Some(e),
        }
        cx.notify();
    }
    fn preparation_tab(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key != "tab" {
            return;
        }
        let Some(editor) = &self.preparation_editor else {
            return;
        };
        let fields: Vec<_> = editor
            .commands
            .iter()
            .chain(&editor.files)
            .map(|(i, _)| i.read(cx).focus_handle(cx))
            .collect();
        if fields.is_empty() {
            return;
        }
        let current = fields.iter().position(|f| f.is_focused(window));
        let next = match current {
            Some(i) if event.keystroke.modifiers.shift => (i + fields.len() - 1) % fields.len(),
            Some(i) => (i + 1) % fields.len(),
            None => 0,
        };
        window.focus(&fields[next], cx);
        cx.stop_propagation();
    }
    pub(super) fn after_worktree_created(
        &mut self,
        repo: PathBuf,
        path: PathBuf,
        launch: (bool, Option<AgentId>),
        navigation: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (open, agent) = launch;
        let plan = self
            .config
            .repo_preparation
            .get(&repo)
            .cloned()
            .unwrap_or_default();
        if plan.is_empty() && navigation.is_none_or(|n| n == self.env.windows.borrow().navigation) {
            if open {
                match agent {
                    Some(agent) => self.run_agent(&path, agent, None, window, cx),
                    None => self.open_tab_at(Some(path), None, window, cx),
                }
            }
            return false;
        }
        let foreground = navigation == Some(self.env.windows.borrow().navigation);
        self.begin_preparation(repo, path.clone(), plan, (open, agent), false, cx);
        if foreground {
            self.preparation_open = Some(path);
        }
        cx.notify();
        true
    }
    fn begin_preparation(
        &mut self,
        repo: PathBuf,
        path: PathBuf,
        plan: PreparationPlan,
        launch: (bool, Option<AgentId>),
        retry: bool,
        cx: &mut Context<Self>,
    ) {
        let (open, agent) = launch;
        if self
            .preparation_jobs
            .get(&path)
            .is_some_and(|j| j.stage.busy())
        {
            return;
        }
        self.preparation_generation = self.preparation_generation.wrapping_add(1);
        let id = self.preparation_generation;
        self.preparation_jobs.insert(
            path.clone(),
            PreparationJob {
                id,
                repo: repo.clone(),
                path: path.clone(),
                plan: plan.clone(),
                preview: None,
                stage: PreparationStage::Inspecting,
                cancelled: Arc::new(AtomicBool::new(false)),
                finished: Arc::new(AtomicBool::new(true)),
                open,
                agent,
            },
        );
        self.publish_preparation_guard(&path, true, cx);
        let data = self.env.data_dir.clone();
        let operation = self.env.windows.borrow().update.operation();
        let work = cx.background_spawn({
            let path = path.clone();
            async move {
                if plan.is_empty() {
                    return Ok((None, None));
                }
                let preview = PreparationPreview::inspect(&repo, &path, plan, retry)
                    .map_err(|e| e.to_string())?;
                let approved = if retry {
                    None
                } else {
                    preview
                        .remembered(
                            data.as_deref()
                                .ok_or("Private approval storage is unavailable")?,
                        )
                        .map_err(|e| e.to_string())?
                };
                Ok::<_, String>((Some(preview), approved))
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                let Some(job) = v.preparation_jobs.get_mut(&path).filter(|j| j.id == id) else {
                    return;
                };
                if job.cancelled.load(Ordering::Acquire) {
                    job.stage = PreparationStage::Cancelled;
                } else {
                    match result {
                        Ok((preview, approved)) => {
                            job.preview = preview;
                            job.stage = if job.preview.is_none() {
                                PreparationStage::Complete
                            } else {
                                PreparationStage::Review
                            };
                            if let Some(approved) = approved {
                                v.run_preparation(path.clone(), id, approved, cx);
                            }
                        }
                        Err(e) => job.stage = PreparationStage::Failed(e),
                    }
                }
                cx.notify();
            });
            drop(operation);
        })
        .detach();
    }
    fn approve_preparation(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preparation_open.clone() else {
            return;
        };
        let Some(job) = self
            .preparation_jobs
            .get_mut(&path)
            .filter(|j| matches!(j.stage, PreparationStage::Review))
        else {
            return;
        };
        let Some(preview) = job.preview.clone() else {
            return;
        };
        let id = job.id;
        job.stage = PreparationStage::Inspecting;
        let data = self.env.data_dir.clone();
        let operation = self.env.windows.borrow().update.operation();
        let work = cx.background_spawn(async move {
            preview
                .authorize(
                    data.as_deref()
                        .ok_or("Private approval storage is unavailable")?,
                )
                .map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                let Some(job) = v.preparation_jobs.get_mut(&path).filter(|j| j.id == id) else {
                    return;
                };
                if job.cancelled.load(Ordering::Acquire) {
                    job.stage = PreparationStage::Cancelled;
                } else {
                    match result {
                        Ok(approved) => v.run_preparation(path.clone(), id, approved, cx),
                        Err(e) => job.stage = PreparationStage::Failed(e),
                    }
                }
                cx.notify();
            });
            drop(operation);
        })
        .detach();
        cx.notify();
    }
    fn run_preparation(
        &mut self,
        path: PathBuf,
        id: u64,
        approved: AuthorizedPreparation,
        cx: &mut Context<Self>,
    ) {
        let Some(job) = self.preparation_jobs.get_mut(&path).filter(|j| j.id == id) else {
            return;
        };
        let cancel = job.cancelled.clone();
        job.finished.store(false, Ordering::Release);
        let finished = WorkerFinished(job.finished.clone());
        job.stage = PreparationStage::Running(PreparationProgress::Copying);
        let shell = self.env.shell.clone();
        let environment = self.env.pane_env.clone();
        let (tx, mut rx) = unbounded::<RunMessage>();
        let background_operation = self.env.windows.borrow().update.operation();
        let completion_operation = self.env.windows.borrow().update.operation();
        let worker = std::thread::Builder::new()
            .name("chda-preparation".into())
            .spawn(move || {
                let _finished = finished;
                let result = shell
                    .ok_or_else(|| "The configured shell is unavailable".to_owned())
                    .and_then(|shell| {
                        approved
                            .run(&shell, &environment, &cancel, |p| {
                                let _ = tx.unbounded_send(RunMessage::Progress(p));
                            })
                            .map_err(|e| e.to_string())
                    });
                let _ = tx.unbounded_send(RunMessage::Done(result, cancel.load(Ordering::Acquire)));
                drop(background_operation);
            });
        if let Err(error) = worker
            && let Some(job) = self.preparation_jobs.get_mut(&path).filter(|j| j.id == id)
        {
            job.stage = PreparationStage::Failed(format!("Could not start preparation: {error}"));
        }
        cx.spawn(async move |this, cx| {
            while let Some(message) = rx.next().await {
                let done = matches!(&message, RunMessage::Done(..));
                if this
                    .update(cx, |v, cx| {
                        let Some(job) = v.preparation_jobs.get_mut(&path).filter(|j| j.id == id)
                        else {
                            return;
                        };
                        match message {
                            RunMessage::Progress(progress) => {
                                job.stage = PreparationStage::Running(progress)
                            }
                            RunMessage::Done(_, true) => job.stage = PreparationStage::Cancelled,
                            RunMessage::Done(_, false) if job.cancelled.load(Ordering::Acquire) => {
                                job.stage = PreparationStage::Cancelled
                            }
                            RunMessage::Done(Ok(()), false) => {
                                job.stage = PreparationStage::Complete
                            }
                            RunMessage::Done(Err(e), false) => {
                                job.stage = PreparationStage::Failed(e)
                            }
                        }
                        // The worktree remains unopened until an explicit Open or Skip.
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                if done {
                    break;
                }
            }
            drop(completion_operation);
        })
        .detach();
    }
    pub(super) fn close_preparation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preparation_editor = None;
        self.preparation_open = None;
        self.focus_active(window, cx);
        cx.notify();
    }
    fn cancel_preparation(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self
            .preparation_open
            .as_ref()
            .and_then(|p| self.preparation_jobs.get_mut(p))
        {
            job.cancelled.store(true, Ordering::Release);
            if !job.stage.busy() {
                job.stage = PreparationStage::Cancelled;
            }
        }
        cx.notify();
    }
    fn retry_preparation(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preparation_open.clone() else {
            return;
        };
        let Some(job) = self.preparation_jobs.get(&path).filter(|j| !j.stage.busy()) else {
            return;
        };
        let (repo, open, agent) = (job.repo.clone(), job.open, job.agent);
        let plan = self
            .config
            .repo_preparation
            .get(&repo)
            .cloned()
            .unwrap_or_default();
        self.begin_preparation(repo, path, plan, (open, agent), true, cx);
        cx.notify();
    }
    fn open_prepared_worktree(&mut self, skip: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.preparation_open.clone() else {
            return;
        };
        let Some(job) = self
            .preparation_jobs
            .get(&path)
            .filter(|j| !j.stage.busy() && j.open)
        else {
            return;
        };
        if !skip && !matches!(job.stage, PreparationStage::Complete) {
            return;
        }
        if self.launch_sheet.is_some() {
            self.notify_error("Finish or cancel the current agent launch first".into());
            return;
        }
        let agent = job.agent;
        if !path.is_dir() {
            self.notify_error("The prepared worktree no longer exists".into());
            return;
        }
        self.preparation_open = None;
        self.preparation_jobs.remove(&path);
        self.publish_preparation_guard(&path, false, cx);
        match agent {
            Some(agent) => self.run_agent(&path, agent, None, window, cx),
            None => self.open_tab_at(Some(path), None, window, cx),
        }
        cx.notify();
    }
    pub(super) fn show_preparation(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.preparation_jobs.contains_key(&path) {
            return;
        }
        self.preparation_open = Some(path);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
    pub(super) fn render_preparation_badge(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.preparation_jobs.is_empty() {
            return None;
        }
        let mut jobs: Vec<_> = self.preparation_jobs.values().collect();
        jobs.sort_by_key(|j| j.id);
        let items = jobs
            .iter()
            .map(|job| {
                (
                    format!(
                        "{} · {}",
                        job.path.file_name().unwrap_or_default().to_string_lossy(),
                        job.stage.label()
                    ),
                    MenuAction::PreparationRun(job.path.clone()),
                )
            })
            .collect::<Vec<_>>();
        Some(
            div()
                .id("preparation-badge")
                .debug_selector(|| "preparation-badge".into())
                .px_2()
                .rounded_md()
                .cursor_pointer()
                .child(format!("Setup · {}", jobs.len()))
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.context_menu = Some(ContextMenu {
                        position: point(px(20.0), px(TITLE_BAR_HEIGHT)),
                        items: items.clone(),
                    });
                    cx.notify();
                }))
                .into_any_element(),
        )
    }
    pub(super) fn render_preparation(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(editor) = &self.preparation_editor {
            let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
            let fields = |command: bool, fields: &Vec<(Entity<TextInput>, Subscription)>| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(fields.iter().enumerate().map(|(index, (input, _))| {
                        let selector = format!(
                            "preparation-{}-{index}",
                            if command { "command" } else { "file" }
                        );
                        div()
                            .debug_selector(move || selector.clone())
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(format!(
                                        "{} {}",
                                        if command { "Command" } else { "File" },
                                        index + 1
                                    ))
                                    .child(
                                        div()
                                            .id((
                                                if command {
                                                    "remove-command"
                                                } else {
                                                    "remove-file"
                                                },
                                                index,
                                            ))
                                            .cursor_pointer()
                                            .child("Remove")
                                            .on_click(cx.listener(move |v, _, _, cx| {
                                                if let Some(e) = &mut v.preparation_editor {
                                                    let fields = if command {
                                                        &mut e.commands
                                                    } else {
                                                        &mut e.files
                                                    };
                                                    if index < fields.len() {
                                                        drop(fields.remove(index));
                                                    }
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            )
                            .child(input.clone())
                    }))
            };
            let body = div().flex().flex_col().gap_3().on_key_down(cx.listener(Self::preparation_tab))
                .child(div().text_sm().child(editor.repo.display().to_string()))
                .child("For new worktrees only. Saving does not run anything.")
                .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Setup commands — in this order"))
                .child(fields(true, &editor.commands))
                .child(div().id("preparation-add-command").debug_selector(|| "preparation-add-command".into()).cursor_pointer().child("+ Add command")
                    .on_click(cx.listener(|v, _, window, cx| v.add_preparation_field(true, "", window, cx))))
                .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Local files — relative to the primary checkout"))
                .child(fields(false, &editor.files))
                .child(div().id("preparation-add-file").debug_selector(|| "preparation-add-file".into()).cursor_pointer().child("+ Add file")
                    .on_click(cx.listener(|v, _, window, cx| v.add_preparation_field(false, "", window, cx))))
                .child(div().text_xs().text_color(fg.opacity(0.65)).child("Only explicitly listed, untracked gitignored regular files. No folders or symlinks."))
                .child(div().flex().justify_end().gap_3()
                    .child(div().id("preparation-settings-cancel").cursor_pointer().child("Cancel").on_click(cx.listener(|v, _, window, cx| v.close_preparation(window, cx))))
                    .child(div().id("preparation-save").debug_selector(|| "preparation-save".into()).px_3().py_2().bg(fg.opacity(0.12)).cursor_pointer().child("Save settings")
                        .on_click(cx.listener(Self::save_preparation_click))));
            return Some(self.sheet_frame(
                "Worktree preparation".into(),
                body,
                editor.error.clone(),
                "Shift ↵ New line   ·   Esc Cancel",
            ));
        }
        let path = self.preparation_open.as_ref()?;
        let job = self.preparation_jobs.get(path)?;
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .debug_selector(|| "preparation-stage".into())
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(job.stage.label()),
            )
            .child(
                div()
                    .text_xs()
                    .child(format!("RUN IN  {}", job.path.display())),
            )
            .child(
                div()
                    .text_xs()
                    .child(format!("COPY FROM  {}", job.repo.display())),
            )
            .child(
                div()
                    .text_xs()
                    .child("Copy files → setup commands → open your terminal or agent"),
            )
            .child(div().text_xs().child("Commands run without terminal input or captured output. Failure keeps the worktree and copied files for inspection."));
        if let Some(preview) = &job.preview {
            for file in &preview.files {
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(format!(
                            "{} · {}",
                            if file.keep_existing {
                                "Keep existing"
                            } else {
                                "Copy file"
                            },
                            file.relative.display()
                        ))
                        .child(div().text_xs().child(format!(
                            "{} → {}",
                            preview.identity.primary.join(&file.relative).display(),
                            preview.destination.join(&file.relative).display()
                        ))),
                );
            }
            if preview.retry {
                body = body.child("Retry keeps existing regular files and runs all setup commands again from the beginning.");
            }
        }
        for (index, command) in job.plan.commands.iter().enumerate() {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(format!("Command {}", index + 1))
                    .child(div().p_2().bg(fg.opacity(0.06)).child(command.clone())),
            );
        }
        let mut buttons = div().flex().flex_wrap().gap_3().items_center();
        if matches!(job.stage, PreparationStage::Review) {
            buttons = buttons.child(
                div()
                    .id("preparation-start")
                    .debug_selector(|| "preparation-start".into())
                    .px_3()
                    .py_2()
                    .bg(fg.opacity(0.15))
                    .cursor_pointer()
                    .child("Confirm and prepare")
                    .on_click(cx.listener(|v, _, _, cx| v.approve_preparation(cx))),
            );
        }
        if job.stage.busy() {
            buttons = buttons.child(
                div()
                    .id("preparation-cancel")
                    .debug_selector(|| "preparation-cancel".into())
                    .cursor_pointer()
                    .child("Cancel preparation")
                    .on_click(cx.listener(|v, _, _, cx| v.cancel_preparation(cx))),
            );
        } else {
            if matches!(job.stage, PreparationStage::Complete) && job.open {
                let label = job
                    .agent
                    .map(|a| format!("Open {}…", a.as_str()))
                    .unwrap_or_else(|| "Open terminal".into());
                buttons = buttons.child(
                    div()
                        .id("preparation-open")
                        .debug_selector(|| "preparation-open".into())
                        .px_3()
                        .py_2()
                        .bg(fg.opacity(0.15))
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(|v, _, window, cx| {
                            v.open_prepared_worktree(false, window, cx)
                        })),
                );
            }
            if matches!(
                job.stage,
                PreparationStage::Failed(_) | PreparationStage::Cancelled
            ) {
                buttons = buttons.child(
                    div()
                        .id("preparation-retry")
                        .debug_selector(|| "preparation-retry".into())
                        .cursor_pointer()
                        .child("Review retry")
                        .on_click(cx.listener(|v, _, _, cx| v.retry_preparation(cx))),
                );
                let repo = job.repo.clone();
                buttons = buttons.child(
                    div()
                        .id("preparation-edit")
                        .cursor_pointer()
                        .child("Edit settings")
                        .on_click(cx.listener(move |v, _, window, cx| {
                            v.edit_preparation(repo.clone(), window, cx)
                        })),
                );
            }
            if !matches!(job.stage, PreparationStage::Complete) && job.open {
                buttons = buttons.child(
                    div()
                        .id("preparation-skip")
                        .debug_selector(|| "preparation-skip".into())
                        .cursor_pointer()
                        .child("Skip preparation and open…")
                        .on_click(cx.listener(|v, _, window, cx| {
                            v.open_prepared_worktree(true, window, cx)
                        })),
                );
            }
            buttons = buttons.child(
                div()
                    .id("preparation-finish")
                    .debug_selector(|| "preparation-finish".into())
                    .cursor_pointer()
                    .child("Keep worktree and close")
                    .on_click(cx.listener(|v, _, window, cx| {
                        if let Some(path) = v.preparation_open.take() {
                            v.preparation_jobs.remove(&path);
                            v.publish_preparation_guard(&path, false, cx);
                        }
                        v.focus_active(window, cx);
                        cx.notify();
                    })),
            );
        }
        buttons = buttons.child(
            div()
                .id("preparation-hide")
                .debug_selector(|| "preparation-hide".into())
                .cursor_pointer()
                .child("Continue working")
                .on_click(cx.listener(|v, _, window, cx| v.close_preparation(window, cx))),
        );
        body = body.child(buttons);
        Some(self.sheet_frame(
            "Prepare new worktree".into(),
            body,
            None,
            "Esc Continue working   ·   Find this run in Setup",
        ))
    }
    fn save_preparation_click(
        &mut self,
        _: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_preparation(window, cx);
    }
}
