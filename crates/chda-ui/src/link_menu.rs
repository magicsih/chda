//! The menu for a right-clicked link: open, reveal, a terminal or `cd`
//! there, copy.

use std::path::{Component, Path, PathBuf};

use chda_core::PaneId;

use crate::platform;
use crate::terminal_view::LinkOpen;
use crate::workspace_view::MenuAction;

/// Menu items for `link`, clicked in `pane` whose shell is in `cwd`.
/// `at_prompt`: the shell waits for a command, so `cd` can be typed.
pub(crate) fn items(
    link: &LinkOpen,
    pane: PaneId,
    cwd: Option<&Path>,
    at_prompt: bool,
) -> Vec<(String, MenuAction)> {
    let (path, line, column, is_dir) = match link {
        LinkOpen::Url(url) => {
            return vec![
                ("Open link".into(), MenuAction::OpenUrl(url.clone())),
                ("Copy link".into(), MenuAction::Copy(url.clone())),
            ];
        }
        LinkOpen::Path {
            path,
            line,
            column,
            is_dir,
        } => (path, *line, *column, *is_dir),
    };
    let folder = if is_dir {
        path.clone()
    } else {
        path.parent()
            .map_or_else(|| path.clone(), Path::to_path_buf)
    };
    let manager = platform::file_manager_name();
    let mut items = vec![
        (
            "Open".into(),
            MenuAction::OpenPath {
                path: path.clone(),
                line,
                column,
            },
        ),
        (
            format!("Reveal in {manager}"),
            MenuAction::RevealPath(path.clone()),
        ),
    ];
    if !is_dir && crate::markdown_preview::is_markdown(path) {
        items.insert(
            1,
            (
                "Preview Markdown".into(),
                MenuAction::PreviewMarkdown(path.clone()),
            ),
        );
    }
    items.extend([
        (
            if is_dir {
                format!("Open folder in {manager}")
            } else {
                format!("Open containing folder in {manager}")
            },
            MenuAction::OpenWithSystem(folder.clone()),
        ),
        (
            "Open a terminal tab here".into(),
            MenuAction::OpenTerminal(folder.clone()),
        ),
    ]);
    if at_prompt {
        items.push((
            "cd here in this pane".into(),
            MenuAction::CdHere { pane, dir: folder },
        ));
    }
    items.push((
        "Copy absolute path".into(),
        MenuAction::Copy(path.to_string_lossy().into_owned()),
    ));
    if let Some(relative) = cwd.map(|cwd| relative_path(path, cwd)) {
        items.push((
            "Copy relative path".into(),
            MenuAction::Copy(relative.to_string_lossy().into_owned()),
        ));
    }
    items
}

/// `path` relative to the directory `base`, with `..` where needed; both
/// absolute.
pub(crate) fn relative_path(path: &Path, base: &Path) -> PathBuf {
    let path: Vec<Component> = path.components().collect();
    let base: Vec<Component> = base.components().collect();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    for c in &path[common..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let rel = |p: &str, b: &str| relative_path(Path::new(p), Path::new(b));
        assert_eq!(rel("/a/b/src/main.rs", "/a/b"), Path::new("src/main.rs"));
        assert_eq!(rel("/a/shared", "/a/b/c"), Path::new("../../shared"));
        assert_eq!(rel("/a/b", "/a/b"), Path::new("."));
        assert_eq!(rel("/x/y", "/a"), Path::new("../x/y"));
    }

    #[test]
    fn file_and_folder_menus() {
        let (_, pane) = chda_core::Workspace::new().new_tab();
        let file = LinkOpen::Path {
            path: "/w/src/main.rs".into(),
            line: Some(12),
            column: Some(5),
            is_dir: false,
        };
        let labels = |items: &[(String, MenuAction)]| {
            items.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>()
        };
        let items = items(&file, pane, Some(Path::new("/w")), false);
        assert!(
            matches!(&items[2].1, MenuAction::OpenWithSystem(p) if p == Path::new("/w/src")),
            "a file's folder is its parent"
        );
        assert!(!labels(&items).iter().any(|l| l.starts_with("cd ")));
        assert!(matches!(&items.last().unwrap().1, MenuAction::Copy(p) if p == "src/main.rs"));

        let folder = LinkOpen::Path {
            path: "/w/build".into(),
            line: None,
            column: None,
            is_dir: true,
        };
        let items = super::items(&folder, pane, None, true);
        assert!(items.iter().any(|(l, a)| l == "cd here in this pane"
            && matches!(a, MenuAction::CdHere { dir, .. } if dir == Path::new("/w/build"))));
        assert!(
            !labels(&items).contains(&"Copy relative path".to_owned()),
            "no relative path without a directory"
        );
    }
}
