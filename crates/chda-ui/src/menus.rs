//! The menu bar. Shortcuts shown next to items come from the key bindings;
//! items whose action nothing in focus handles are greyed out.

use gpui::{App, Menu, MenuItem, SystemMenuType, actions};

use crate::platform;
use crate::terminal_view::{
    Copy, Find, FindNext, FindPrevious, JumpToNextPrompt, JumpToPrevPrompt, Paste,
};
use crate::workspace_view::*;

actions!(
    app,
    [
        About,
        Hide,
        HideOthers,
        ShowAll,
        OpenRepository,
        ReportIssue,
        ReleaseNotes,
    ]
);

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Register the app-level menu actions and set the menu bar.
pub fn install(cx: &mut App) {
    cx.on_action(|_: &About, _| platform::show_about());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &OpenRepository, cx| cx.open_url(REPOSITORY));
    cx.on_action(|_: &ReportIssue, cx| cx.open_url(&format!("{REPOSITORY}/issues/new")));
    cx.on_action(|_: &ReleaseNotes, cx| cx.open_url(&format!("{REPOSITORY}/releases")));
    cx.set_menus(menus());
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("chda").items([
            MenuItem::action("About chda", About),
            MenuItem::separator(),
            MenuItem::action("Settings...", OpenConfig),
            MenuItem::action("Open Ghostty Config", OpenGhosttyConfig),
            MenuItem::action("Reload Config", ReloadConfig),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide chda", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit chda", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("New Worktree...", NewWorktree),
            MenuItem::action("Add Repository...", AddRepo),
            MenuItem::separator(),
            MenuItem::action("Split Right", SplitRight),
            MenuItem::action("Split Down", SplitDown),
            MenuItem::separator(),
            MenuItem::action("Close", CloseSurface),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Copy", Copy),
            MenuItem::action("Paste", Paste),
            MenuItem::separator(),
            MenuItem::action("Find...", Find),
            MenuItem::action("Find Next", FindNext),
            MenuItem::action("Find Previous", FindPrevious),
            MenuItem::separator(),
            MenuItem::action("Jump to Previous Prompt", JumpToPrevPrompt),
            MenuItem::action("Jump to Next Prompt", JumpToNextPrompt),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Command Palette", TogglePalette),
            MenuItem::separator(),
            MenuItem::action("Zoom Split", ToggleZoom),
            MenuItem::separator(),
            MenuItem::action("Increase Font Size", IncreaseFontSize),
            MenuItem::action("Decrease Font Size", DecreaseFontSize),
            MenuItem::action("Reset Font Size", ResetFontSize),
            MenuItem::separator(),
            MenuItem::action("Select Theme...", SelectTheme),
        ]),
        Menu::new("Agents").items([
            MenuItem::action("Go to Waiting Agent", GoToWaitingAgent),
            MenuItem::separator(),
            MenuItem::action("Run Claude Code Here", RunClaude),
            MenuItem::action("Run Codex Here", RunCodex),
            MenuItem::action("Resume Session...", ResumeSession),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("Show Next Tab", NextTab),
            MenuItem::action("Show Previous Tab", PrevTab),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Select Split").items([
                MenuItem::action("Next", NextSplit),
                MenuItem::action("Previous", PrevSplit),
                MenuItem::separator(),
                MenuItem::action("Left", FocusLeft),
                MenuItem::action("Right", FocusRight),
                MenuItem::action("Above", FocusUp),
                MenuItem::action("Below", FocusDown),
            ])),
            MenuItem::submenu(Menu::new("Resize Split").items([
                MenuItem::action("Move Divider Left", ResizeLeft),
                MenuItem::action("Move Divider Right", ResizeRight),
                MenuItem::action("Move Divider Up", ResizeUp),
                MenuItem::action("Move Divider Down", ResizeDown),
                MenuItem::separator(),
                MenuItem::action("Equalize Splits", EqualizeSplits),
            ])),
            MenuItem::separator(),
        ]),
        Menu::new("Help").items([
            MenuItem::action("chda on GitHub", OpenRepository),
            MenuItem::action("Report an Issue", ReportIssue),
            MenuItem::action("Release Notes", ReleaseNotes),
        ]),
    ]
}
