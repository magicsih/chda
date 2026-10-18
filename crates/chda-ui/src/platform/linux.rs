//! XDG application discovery. GLib's launcher handles desktop Exec fields and
//! D-Bus activation, rather than interpreting application commands as a shell.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use freedesktop_desktop_entry::{DesktopEntry, Iter};

use super::FolderApp;

fn available(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &Path| {
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        executable(Path::new(program))
    } else {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .any(|path| executable(&path.join(program)))
    }
}

fn can_open_folders(entry: &DesktopEntry) -> bool {
    let desktops = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    can_open_folders_on(entry, &desktops)
}

fn can_open_folders_on(entry: &DesktopEntry, desktops: &[String]) -> bool {
    const EDITORS: &[&str] = &[
        "code",
        "code-oss",
        "codium",
        "cursor",
        "windsurf",
        "dev.zed.Zed",
        "sublime_text",
        "jetbrains-idea",
        "jetbrains-idea-ce",
        "jetbrains-pycharm",
    ];
    let only = entry.only_show_in();
    let excluded = entry.not_show_in();
    let visible = desktops
        .iter()
        .filter(|desktop| !desktop.is_empty())
        .find_map(|desktop| {
            if only
                .as_ref()
                .is_some_and(|names| names.contains(&desktop.as_str()))
            {
                Some(true)
            } else if excluded
                .as_ref()
                .is_some_and(|names| names.contains(&desktop.as_str()))
            {
                Some(false)
            } else {
                None
            }
        })
        .unwrap_or(only.is_none());
    entry.type_() == Some("Application")
        && !entry.hidden()
        && !entry.no_display()
        && !entry.terminal()
        && (entry.exec().is_some() || entry.dbus_activatable())
        && visible
        && entry.desktop_entry("TryExec").is_none_or(available)
        && (entry
            .mime_type()
            .is_some_and(|types| types.contains(&"inode/directory"))
            || EDITORS.contains(&entry.id()))
}

fn entries() -> Vec<DesktopEntry> {
    let locales = freedesktop_desktop_entry::get_languages_from_env();
    let mut seen = HashSet::new();
    Iter::new(freedesktop_desktop_entry::default_paths())
        .entries(Some(&locales))
        // A hidden user entry masks a system entry with the same id.
        .filter(|entry| seen.insert(entry.id().to_owned()))
        .filter(can_open_folders)
        .collect()
}

fn icon(name: &str) -> Option<Arc<gpui::Image>> {
    let mut candidates = Vec::new();
    if Path::new(name).is_absolute() {
        candidates.push(PathBuf::from(name));
    } else if !name.contains('/') {
        for root in freedesktop_desktop_entry::default_paths()
            .filter_map(|p| p.parent().map(Path::to_path_buf))
        {
            for theme in ["hicolor", "Adwaita", "breeze"] {
                for size in ["64x64", "48x48", "256x256", "scalable"] {
                    for extension in ["png", "svg"] {
                        candidates.push(
                            root.join(format!("icons/{theme}/{size}/apps/{name}.{extension}")),
                        );
                    }
                }
            }
            candidates.push(root.join(format!("pixmaps/{name}.png")));
        }
    }
    candidates.into_iter().find_map(|path| {
        if path.metadata().ok()?.len() > 1_048_576 {
            return None;
        }
        let format = match path.extension()?.to_str()? {
            "png" => gpui::ImageFormat::Png,
            "svg" => gpui::ImageFormat::Svg,
            _ => return None,
        };
        Some(Arc::new(gpui::Image::from_bytes(
            format,
            std::fs::read(path).ok()?,
        )))
    })
}

pub(super) fn folder_apps() -> Vec<FolderApp> {
    let mut apps = Vec::new();
    if available("xdg-open") {
        apps.push(FolderApp {
            id: "xdg-default".into(),
            name: "File Manager".into(),
            icon: None,
        });
    }
    if available("gio") {
        let locales = freedesktop_desktop_entry::get_languages_from_env();
        let mut installed: Vec<_> = entries()
            .into_iter()
            .filter_map(|entry| {
                Some(FolderApp {
                    id: format!("desktop:{}", entry.id()),
                    name: entry.name(&locales)?.into_owned(),
                    icon: entry.icon().and_then(icon),
                })
            })
            .collect();
        installed.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        apps.extend(installed);
    }
    apps
}

pub(super) fn open_folder_in(id: &str, folder: &Path) -> std::io::Result<()> {
    let folder = folder.canonicalize()?;
    if !folder.is_dir() {
        return Err(std::io::Error::other("The folder no longer exists"));
    }
    let mut command = if id == "xdg-default" {
        let mut command = Command::new("xdg-open");
        command.arg(&folder);
        command
    } else {
        let entry = entries()
            .into_iter()
            .find(|e| Some(e.id()) == id.strip_prefix("desktop:"))
            .ok_or_else(|| {
                std::io::Error::other("The selected application is no longer installed")
            })?;
        let mut command = Command::new("gio");
        command.arg("launch").arg(&entry.path).arg(&folder);
        command
    };
    // Editors can remain open for hours. Reap off the UI thread and report
    // launch failure without blocking input or retaining zombies.
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        if let Ok(status) = child.wait()
            && !status.success()
        {
            eprintln!("chda: the selected application could not open the folder");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(extra: &str) -> DesktopEntry {
        DesktopEntry::from_str(
            Path::new("/fixture/code.desktop"),
            &format!("[Desktop Entry]\nType=Application\nName=Editor\nExec=code %F\n{extra}"),
            None::<&[&str]>,
        )
        .unwrap()
    }

    #[test]
    fn only_folder_capable_visible_gui_applications_are_offered() {
        assert!(can_open_folders(&entry("")));
        for fields in [
            "Hidden=true\n",
            "NoDisplay=true\n",
            "Terminal=true\n",
            "TryExec=/missing/chda-test-app\n",
        ] {
            assert!(!can_open_folders(&entry(fields)), "{fields}");
        }
        let manager = DesktopEntry::from_str(
            Path::new("/fixture/manager.desktop"),
            "[Desktop Entry]\nType=Application\nName=Files\nExec=files %U\nMimeType=inode/directory;\n",
            None::<&[&str]>,
        ).unwrap();
        assert!(can_open_folders(&manager));
        let unrelated = DesktopEntry::from_str(
            Path::new("/fixture/player.desktop"),
            "[Desktop Entry]\nType=Application\nName=Player\nExec=player %U\nMimeType=video/mp4;\n",
            None::<&[&str]>,
        )
        .unwrap();
        assert!(!can_open_folders(&unrelated));
    }

    #[test]
    fn desktop_visibility_and_dbus_only_applications_follow_the_specification() {
        let gnome = vec!["GNOME".to_owned()];
        let kde = vec!["KDE".to_owned()];
        let restricted = entry("OnlyShowIn=KDE;\n");
        assert!(!can_open_folders_on(&restricted, &gnome));
        assert!(!can_open_folders_on(&restricted, &[]));
        assert!(can_open_folders_on(&restricted, &kde));
        let excluded = entry("NotShowIn=GNOME;\n");
        assert!(!can_open_folders_on(&excluded, &gnome));
        assert!(can_open_folders_on(&excluded, &kde));
        let activated = DesktopEntry::from_str(
            Path::new("/fixture/org.example.Files.desktop"),
            "[Desktop Entry]\nType=Application\nName=Files\nDBusActivatable=true\nMimeType=inode/directory;\n",
            None::<&[&str]>,
        ).unwrap();
        assert!(can_open_folders_on(&activated, &gnome));
        let ordered = entry("OnlyShowIn=KDE;\nNotShowIn=GNOME;\n");
        assert!(can_open_folders_on(
            &ordered,
            &["KDE".into(), "GNOME".into()]
        ));
        assert!(!can_open_folders_on(
            &ordered,
            &["GNOME".into(), "KDE".into()]
        ));
    }
}
