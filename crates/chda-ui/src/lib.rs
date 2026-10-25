//! GPUI views, elements and theme. The only crate that may depend on GPUI.

mod diagram;
mod diff_review;
mod environment;
mod external_drop;
mod fonts;
mod git_graph;
mod link_menu;
mod markdown_preview;
mod menus;
mod palette;
mod platform;
mod readonly_text;
#[cfg(test)]
mod scenarios;
mod settings;
mod sidebar_view;
mod status_bar;
mod status_icon;
mod terminal_element;
mod terminal_images;
mod terminal_view;
mod text_input;
mod tooltip;
mod ui_icon;
mod upgrade;
mod window_registry;
pub use upgrade::run_broker;
mod workspace_view;

use gpui::{App, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions, point, px, size};

pub use settings::Settings;
use terminal_view::{
    CloseFind, Copy, Find, FindNext, FindPrevious, JumpToNextPrompt, JumpToPrevPrompt, Paste,
    ToggleFindCase, ToggleFindRegex,
};
use workspace_view::*;

/// Ghostty-style shortcuts, translated by the desktop platform adapter.
fn key_bindings() -> Vec<KeyBinding> {
    let t = Some("Terminal");
    let mut bindings = vec![
        KeyBinding::new(platform::shortcut("cmd-c"), Copy, t),
        KeyBinding::new(platform::shortcut("cmd-v"), Paste, t),
        KeyBinding::new(platform::shortcut("cmd-up"), JumpToPrevPrompt, t),
        KeyBinding::new(platform::shortcut("cmd-down"), JumpToNextPrompt, t),
        KeyBinding::new(platform::shortcut("cmd-f"), Find, t),
        KeyBinding::new(platform::shortcut("cmd-g"), FindNext, t),
        KeyBinding::new(platform::shortcut("cmd-shift-g"), FindPrevious, t),
        KeyBinding::new(platform::shortcut("cmd-q"), Quit, None),
        KeyBinding::new(platform::shortcut("cmd-t"), NewTab, None),
        KeyBinding::new(platform::shortcut("cmd-w"), CloseSurface, None),
        KeyBinding::new(platform::shortcut("cmd-shift-]"), NextTab, None),
        KeyBinding::new(platform::shortcut("ctrl-tab"), NextTab, None),
        KeyBinding::new(platform::shortcut("cmd-shift-["), PrevTab, None),
        KeyBinding::new(platform::shortcut("ctrl-shift-tab"), PrevTab, None),
        KeyBinding::new(platform::shortcut("cmd-1"), GotoSession1, None),
        KeyBinding::new(platform::shortcut("cmd-2"), GotoSession2, None),
        KeyBinding::new(platform::shortcut("cmd-3"), GotoSession3, None),
        KeyBinding::new(platform::shortcut("cmd-4"), GotoSession4, None),
        KeyBinding::new(platform::shortcut("cmd-5"), GotoSession5, None),
        KeyBinding::new(platform::shortcut("cmd-6"), GotoSession6, None),
        KeyBinding::new(platform::shortcut("cmd-7"), GotoSession7, None),
        KeyBinding::new(platform::shortcut("cmd-8"), GotoSession8, None),
        KeyBinding::new(platform::shortcut("cmd-9"), GotoSession9, None),
        KeyBinding::new(platform::shortcut("cmd-0"), GotoSession10, None),
        KeyBinding::new(platform::shortcut("cmd-d"), SplitRight, None),
        KeyBinding::new(platform::shortcut("cmd-shift-d"), SplitDown, None),
        KeyBinding::new(platform::shortcut("cmd-alt-left"), FocusLeft, None),
        KeyBinding::new(platform::shortcut("cmd-alt-right"), FocusRight, None),
        KeyBinding::new(platform::shortcut("cmd-alt-up"), FocusUp, None),
        KeyBinding::new(platform::shortcut("cmd-alt-down"), FocusDown, None),
        KeyBinding::new(platform::shortcut("cmd-]"), NextSplit, None),
        KeyBinding::new(platform::shortcut("cmd-["), PrevSplit, None),
        KeyBinding::new(platform::shortcut("cmd-ctrl-left"), ResizeLeft, None),
        KeyBinding::new(platform::shortcut("cmd-ctrl-right"), ResizeRight, None),
        KeyBinding::new(platform::shortcut("cmd-ctrl-up"), ResizeUp, None),
        KeyBinding::new(platform::shortcut("cmd-ctrl-down"), ResizeDown, None),
        KeyBinding::new(platform::shortcut("cmd-ctrl-="), EqualizeSplits, None),
        KeyBinding::new(platform::shortcut("cmd-shift-enter"), ToggleZoom, None),
        KeyBinding::new(platform::shortcut("cmd-b"), ToggleSidebar, None),
        KeyBinding::new(platform::shortcut("cmd-shift-o"), AddRepo, None),
        KeyBinding::new(platform::shortcut("cmd-shift-n"), NewWindow, None),
        KeyBinding::new(platform::shortcut("cmd-n"), NewWorktree, None),
        KeyBinding::new(platform::shortcut("cmd-shift-p"), TogglePalette, None),
        KeyBinding::new(platform::shortcut("escape"), Dismiss, None),
        KeyBinding::new(platform::shortcut("cmd-="), IncreaseFontSize, None),
        KeyBinding::new(platform::shortcut("cmd-+"), IncreaseFontSize, None),
        KeyBinding::new(platform::shortcut("cmd--"), DecreaseFontSize, None),
        KeyBinding::new(platform::shortcut("cmd-alt-0"), ResetFontSize, None),
        KeyBinding::new(platform::shortcut("cmd-shift-a"), GoToWaitingAgent, None),
        KeyBinding::new(platform::shortcut("cmd-,"), OpenConfig, None),
        KeyBinding::new(platform::shortcut("cmd-shift-,"), ReloadConfig, None),
        KeyBinding::new(platform::shortcut("cmd-m"), Minimize, None),
        KeyBinding::new(platform::shortcut("cmd-v"), Paste, Some("TextInput")),
        KeyBinding::new(platform::shortcut("cmd-c"), Copy, Some("TextInput")),
        KeyBinding::new(
            platform::shortcut("cmd-a"),
            text_input::SelectAll,
            Some("TextInput"),
        ),
        KeyBinding::new("secondary-c", Copy, Some("TextInput")),
        KeyBinding::new("secondary-v", Paste, Some("TextInput")),
        // Global bindings count as matching at the focused element's depth,
        // so these name the focused field and come last: on equal depth the
        // later binding wins over the global escape (Dismiss).
        KeyBinding::new(
            platform::shortcut("enter"),
            FindNext,
            Some("SearchBar > TextInput"),
        ),
        KeyBinding::new(
            platform::shortcut("shift-enter"),
            FindPrevious,
            Some("SearchBar > TextInput"),
        ),
        KeyBinding::new(
            platform::shortcut("escape"),
            CloseFind,
            Some("SearchBar > TextInput"),
        ),
        KeyBinding::new(
            platform::shortcut("alt-c"),
            ToggleFindCase,
            Some("SearchBar > TextInput"),
        ),
        KeyBinding::new(
            platform::shortcut("alt-r"),
            ToggleFindRegex,
            Some("SearchBar > TextInput"),
        ),
    ];
    bindings.extend(platform::hide_key_bindings());
    bindings
}

/// Start the application and open the main window with the user's Ghostty
/// config.
pub fn run() {
    run_inner(None);
}

pub fn run_adopt(fd: i32) -> std::io::Result<()> {
    let inherited = upgrade::inherited(fd)?;
    run_inner(Some(inherited));
    Ok(())
}

fn run_inner(inherited: Option<(chda_core::handoff::Handoff, chda_term::handoff::Inherited)>) {
    platform::init_graphics_diagnostics();
    let env = std::rc::Rc::new(environment::Environment::for_user());
    let mut reply = None;
    let mut handoff_session = None;
    let mut completed_update = None;
    if let Some((state, inherited)) = inherited {
        if state.recovery_error.is_none() {
            completed_update = state.update.clone();
        }
        let mut registry = env.windows.borrow_mut();
        registry.started_at = Some(state.started_at);
        registry.update.adopting = true;
        registry.inherited_notifications = state.notifications.into_iter().collect();
        registry.children = state.children;
        registry.update.frozen = true;
        registry.update.progress = chda_core::self_update::UpdateProgress::Reconnecting;
        if let Some(update) = &state.update {
            registry.update.target = Some(update.target_exe.clone());
            registry.update.journal = Some(update.directory.join("events.jsonl"));
            if let Ok(text) = std::fs::read_to_string(update.directory.join("events.jsonl")) {
                for line in text.lines() {
                    if let Ok(q) =
                        serde_json::from_str::<chda_core::agents::quota::QuotaSnapshot>(line)
                    {
                        registry
                            .replay
                            .push(chda_core::agents::control::Incoming::Quota(q));
                    } else if let Ok(e) = serde_json::from_str::<chda_core::agents::HookEvent>(line)
                    {
                        registry
                            .replay
                            .push(chda_core::agents::control::Incoming::Event(e));
                    }
                }
            }
        }
        if let Some(message) = state.recovery_error {
            registry.update.progress = chda_core::self_update::UpdateProgress::Failed { message };
        }
        for (pane, pty) in state.panes.into_iter().zip(inherited.ptys) {
            let session = chda_term::DetachedSession {
                pty,
                snapshot: pane.snapshot.clone(),
                size: chda_term::Size {
                    cols: pane.cols,
                    rows: pane.rows,
                },
            };
            registry.update.inherited.insert(pane.pane, (pane, session));
        }
        handoff_session = Some(state.session);
        reply = Some(inherited.reply);
    }
    let app = gpui_platform::application();
    let reopen_env = env.clone();
    app.on_reopen(move |cx| reopen_workspace(reopen_env.clone(), cx));
    app.run(move |cx: &mut App| {
        cx.set_app_identity("com.magicsih.chda", "chda");
        fonts::register(cx);
        cx.bind_keys(key_bindings());
        let quitting_env = env.clone();
        cx.on_action(move |_: &Quit, cx| {
            if !quitting_env.windows.borrow().update.frozen {
                cx.quit();
            }
        });
        menus::install(cx);

        let config = env.load_config();
        let saved = handoff_session.take().or_else(|| {
            config
                .restore_session
                .then(|| {
                    env.data_dir
                        .as_deref()
                        .and_then(chda_core::SavedSession::load)
                })
                .flatten()
        });
        let active = saved.as_ref().map_or(0, |s| s.active_window);
        let windows = saved
            .map(|s| s.windows.into_iter().map(Some).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![None]);
        env.windows.borrow_mut().restoring = true;
        let mut opened = Vec::new();
        for saved in windows {
            opened.push(open_workspace_window(saved, env.clone(), cx));
        }
        env.windows.borrow_mut().restoring = false;
        if let Some(window) = opened.get(active).or(opened.first()) {
            window
                .update(cx, |view, window, cx| {
                    window.activate_window();
                    view.focus(window, cx);
                })
                .expect("main window vanished");
        }
        cx.activate(true);
        if let Some(reply) = reply.take() {
            let env = env.clone();
            let completed_update = completed_update.take();
            cx.spawn(async move |cx| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
                loop {
                    let ready = cx.update(|cx| {
                        let registry = env.windows.borrow();
                        registry.update.inherited.is_empty()
                            && registry.entries.iter().all(|e| {
                                e.view
                                    .upgrade()
                                    .is_some_and(|v| v.read(cx).adoption_ready(cx))
                            })
                    });
                    if ready {
                        break;
                    }
                    if std::time::Instant::now() > deadline {
                        std::process::exit(1);
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(10))
                        .await;
                }
                let committed = cx
                    .background_spawn(
                        async move { reply.prepared(std::time::Duration::from_secs(30)) },
                    )
                    .await;
                if committed.is_err() {
                    std::process::exit(1);
                }
                cx.update(|cx| {
                    {
                        let mut registry = env.windows.borrow_mut();
                        if let Some(server) = &registry.ipc_server {
                            let _ = server.journal_to(None);
                        }
                        registry.update.frozen = false;
                        registry.update.adopting = false;
                        if !matches!(
                            registry.update.progress,
                            chda_core::self_update::UpdateProgress::Failed { .. }
                        ) {
                            registry.update.progress =
                                chda_core::self_update::UpdateProgress::Installed;
                        }
                    }
                    let entries = env.windows.borrow().entries.clone();
                    for e in entries {
                        let _ = e.window.update(cx, |_, window, cx| {
                            let _ = e
                                .view
                                .update(cx, |view, cx| view.finish_adoption(window, cx));
                        });
                    }
                });
                if let Some(handoff) = completed_update {
                    platform::update::discard(platform::update::UpdateJob { handoff });
                }
            })
            .detach();
        }
    });
}

fn reopen_workspace(env: std::rc::Rc<environment::Environment>, cx: &mut App) {
    let existing = {
        let registry = env.windows.borrow();
        if registry.quitting || registry.update.frozen {
            return;
        }
        registry
            .entries
            .iter()
            .find(|entry| Some(entry.window.window_id()) == registry.active)
            .or_else(|| registry.entries.first())
            .cloned()
    };
    env.system.restore_windows();
    if let Some(entry) = existing {
        let _ = entry.window.update(cx, |_, window, cx| {
            window.activate_window();
            let _ = entry.view.update(cx, |view, cx| view.focus(window, cx));
        });
    } else {
        let window = open_workspace_window(None, env, cx);
        let _ = window.update(cx, |view, window, cx| {
            window.activate_window();
            view.focus(window, cx);
        });
    }
    cx.activate(true);
}

pub(crate) fn open_workspace_window(
    saved: Option<chda_core::SavedWindow>,
    env: std::rc::Rc<environment::Environment>,
    cx: &mut App,
) -> gpui::WindowHandle<WorkspaceView> {
    let bounds = match saved.as_ref().and_then(|s| s.bounds) {
        Some(b) => Bounds::new(point(px(b.x), px(b.y)), size(px(b.width), px(b.height))),
        None => Bounds::centered(None, size(px(960.0), px(640.0)), cx),
    };
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            app_id: Some("com.magicsih.chda".into()),
            titlebar: Some(platform::titlebar("chda")),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| WorkspaceView::new(saved, env, window, cx)),
    )
    .expect("failed to open workspace window")
}
