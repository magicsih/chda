//! User-defined `[[commands]]` (#251): run a command line through the login
//! shell for a worktree, in a new tab, a split or a floating terminal, from
//! the palette, a worktree's menu or a shortcut. Commands never run without
//! one of those explicit actions.
use super::*;
use chda_config::{CommandTarget, UserCommand};

/// Run the `[[commands]]` entry called `name` for the focused worktree.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub(crate) struct RunUserCommand {
    pub name: String,
}

/// A command running in a terminal over the window.
pub(crate) struct Floating {
    pub pane: PaneId,
    pub name: String,
    /// Exit status once the command ended unsuccessfully; the terminal stays.
    pub exited: Option<String>,
    /// cmd-w was pressed once while it still runs; the next one stops it.
    pub stop_armed: bool,
}

/// The `key` entries chda can bind: parsed, not taken by a built-in
/// shortcut or an earlier command. The rest come back as problems.
pub(crate) fn command_keys(commands: &[UserCommand]) -> (Vec<(String, String)>, Vec<String>) {
    let builtin: Vec<String> = crate::key_bindings()
        .iter()
        .map(|b| {
            b.keystrokes()
                .iter()
                .map(|k| k.unparse())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let mut bound: Vec<(String, String)> = Vec::new();
    let mut problems = Vec::new();
    for command in commands {
        let Some(key) = command
            .key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        else {
            continue;
        };
        let parsed: Result<Vec<_>, _> =
            key.split_whitespace().map(gpui::Keystroke::parse).collect();
        let Ok(parsed) = parsed else {
            problems.push(format!(
                "command {:?} has a key chda cannot read: {key}",
                command.name
            ));
            continue;
        };
        let key = parsed
            .iter()
            .map(|k| k.unparse())
            .collect::<Vec<_>>()
            .join(" ");
        if builtin.contains(&key) {
            problems.push(format!(
                "command {:?}: {key} is a built-in shortcut and keeps it",
                command.name
            ));
        } else if let Some((_, other)) = bound.iter().find(|(k, _)| *k == key) {
            problems.push(format!(
                "command {:?}: {key} is already the shortcut of {other:?}",
                command.name
            ));
        } else {
            bound.push((key, command.name.clone()));
        }
    }
    (bound, problems)
}

/// Bind the built-in shortcuts and the commands' keys, replacing earlier
/// command keys.
pub(crate) fn bind_command_keys(keys: &[(String, String)], cx: &mut App) {
    cx.clear_key_bindings();
    cx.bind_keys(crate::key_bindings());
    cx.bind_keys(keys.iter().map(|(key, name)| {
        gpui::KeyBinding::new(key, RunUserCommand { name: name.clone() }, None)
    }));
}

impl WorkspaceView {
    /// Commands that can run, and every problem in their entries.
    pub(super) fn user_commands(&self) -> (Vec<UserCommand>, Vec<String>) {
        let (valid, mut problems) = self.config.valid_commands();
        let (_, key_problems) = command_keys(&valid);
        problems.extend(key_problems);
        (valid, problems)
    }

    /// Rebind command shortcuts after `config.toml` changed them.
    pub(super) fn apply_command_keys(&self, cx: &mut App) {
        let (valid, _) = self.config.valid_commands();
        let (keys, _) = command_keys(&valid);
        bind_command_keys(&keys, cx);
    }

    /// The worktree a command runs in: the focused pane's.
    fn command_cwd(&self, cx: &App) -> Option<PathBuf> {
        let cwd = self.focused_cwd()?;
        Some(
            self.sidebar
                .read(cx)
                .model
                .worktree_for_path(&cwd)
                .map(|(_, w)| w.path.clone())
                .unwrap_or(cwd),
        )
    }

    pub(super) fn run_user_command_action(
        &mut self,
        action: &RunUserCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_user_command(&action.name, None, window, cx);
    }

    /// Run the command `name` in `at`, else in the focused pane's worktree.
    pub(super) fn run_user_command(
        &mut self,
        name: &str,
        at: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (valid, _) = self.config.valid_commands();
        let Some(command) = valid.into_iter().find(|c| c.name.trim() == name) else {
            self.notify_error(format!("No command named {name:?} in config.toml"));
            cx.notify();
            return;
        };
        let Some(cwd) = at.or_else(|| self.command_cwd(cx)) else {
            self.notify_error(format!("{name}: no worktree to run it in"));
            cx.notify();
            return;
        };
        let shell = self
            .env
            .shell
            .clone()
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));
        let argv = vec![
            shell.to_string_lossy().into_owned(),
            "-l".into(),
            "-c".into(),
            command.run.clone(),
        ];
        match command.target().unwrap_or(CommandTarget::Tab) {
            CommandTarget::Tab => {
                let (_, pane) = self.ws.new_tab();
                self.open_pane(pane, Some(cwd), Some(argv), window, cx);
            }
            CommandTarget::Split => {
                let pane = match self.ws.split(chda_core::Axis::Horizontal) {
                    Some(pane) => pane,
                    None => self.ws.new_tab().1,
                };
                self.open_pane(pane, Some(cwd), Some(argv), window, cx);
            }
            CommandTarget::Floating => {
                self.close_floating(cx);
                let pane = self.ws.add_floating_pane(chda_core::PaneInfo::default());
                self.open_pane(pane, Some(cwd), Some(argv), window, cx);
                self.floating = Some(Floating {
                    pane,
                    name: command.name.trim().to_owned(),
                    exited: None,
                    stop_armed: false,
                });
                if let Some((view, _)) = self.panes.get(&pane) {
                    window.focus(&view.read(cx).focus_handle(cx), cx);
                }
                cx.notify();
                return;
            }
        }
        self.focus_active(window, cx);
    }

    /// The floating command ended: close on success, else keep its last
    /// screen with the exit status until dismissed.
    pub(super) fn floating_exited(
        &mut self,
        status: Option<&chda_term::ExitStatus>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let success = status.is_some_and(|s| s.code == 0 && s.signal.is_none());
        if success {
            self.close_floating(cx);
            self.focus_active(window, cx);
            return;
        }
        if let Some(floating) = &mut self.floating {
            floating.exited = Some(match status {
                Some(s) => match &s.signal {
                    Some(signal) => format!("Exited — signal {signal}"),
                    None => format!("Exited — code {}", s.code),
                },
                None => "Exited — status unavailable".into(),
            });
        }
        // Keys now reach chda, so Escape or cmd-w close it.
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// cmd-w or Escape on the floating terminal. A running command stops
    /// only on a second cmd-w; Escape belongs to the program.
    pub(super) fn dismiss_floating(
        &mut self,
        stop: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(floating) = &mut self.floating else {
            return false;
        };
        if floating.exited.is_none() {
            if !stop {
                return false;
            }
            if !floating.stop_armed {
                floating.stop_armed = true;
                cx.notify();
                return true;
            }
        }
        self.close_floating(cx);
        self.focus_active(window, cx);
        true
    }

    /// Remove the floating terminal; a running command is stopped with it.
    pub(super) fn close_floating(&mut self, cx: &mut Context<Self>) {
        if let Some(floating) = self.floating.take() {
            self.panes.remove(&floating.pane);
            self.ws.remove_floating_pane(floating.pane);
            cx.notify();
        }
    }

    pub(super) fn render_floating(&self) -> Option<AnyElement> {
        let floating = self.floating.as_ref()?;
        let (view, _) = self.panes.get(&floating.pane)?;
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let status = match (&floating.exited, floating.stop_armed) {
            (Some(exit), _) => format!("{exit} · Escape or cmd-w closes"),
            (None, true) => "Running · press cmd-w again to stop it".to_owned(),
            (None, false) => "Running · closes when it succeeds · cmd-w twice stops it".to_owned(),
        };
        Some(
            deferred(
                div()
                    .absolute()
                    .size_full()
                    .top_0()
                    .left_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::black().opacity(0.3))
                    .occlude()
                    .child(
                        div()
                            .id("floating-command")
                            .debug_selector(|| "floating-command".into())
                            .w(relative(0.8))
                            .h(relative(0.8))
                            .flex()
                            .flex_col()
                            .rounded_md()
                            .overflow_hidden()
                            .bg(bg)
                            .border_1()
                            .border_color(fg.opacity(0.25))
                            .shadow_lg()
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .gap_2()
                                    .px_3()
                                    .py_1()
                                    .text_xs()
                                    .text_color(fg.opacity(0.8))
                                    .bg(blend(bg, fg, 0.08))
                                    .child(floating.name.clone())
                                    .child(
                                        div()
                                            .debug_selector(|| "floating-status".into())
                                            .child(status),
                                    ),
                            )
                            .child(div().flex_1().min_h_0().child(AnyView::from(view.clone()))),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
