//! Durable entry point for hooks, MCP and status-line helpers.
use std::path::PathBuf;

pub fn helper_executable() -> std::io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    {
        let directory = std::env::var_os("APPDIR").map(PathBuf::from);
        let image = std::env::var_os("APPIMAGE").map(PathBuf::from);
        if let Some(image) = appimage::source(&executable, directory.as_deref(), image.as_deref()) {
            let data = crate::hook::data_dir()
                .ok_or_else(|| std::io::Error::other("Missing helper data directory"))?;
            return appimage::launcher(&data, &image);
        }
    }
    Ok(executable)
}

#[cfg(any(target_os = "linux", all(test, unix)))]
mod appimage {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    };
    use std::path::{Path, PathBuf};

    pub(super) fn source(
        executable: &Path,
        directory: Option<&Path>,
        image: Option<&Path>,
    ) -> Option<PathBuf> {
        // An editor launched from another AppImage can pass its environment to
        // chda. Accept APPIMAGE only for this exact packaged binary and marker.
        let (directory, image) = (directory?, image?);
        (directory.join("com.magicsih.chda.desktop").is_file()
            && directory
                .join("usr/bin/chda")
                .canonicalize()
                .ok()?
                .as_path()
                == executable
            && image.is_absolute()
            && crate::ipc::is_executable(image))
        .then(|| image.canonicalize().ok())
        .flatten()
    }

    fn script(image: &Path) -> Vec<u8> {
        let mut text = br#"#!/bin/sh
set -eu
unset TARGET_APPIMAGE
command -v setsid >/dev/null || { echo 'chda: agent helpers require util-linux (setsid)' >&2; exit 1; }
chda_helper_tmp=$(mktemp -d "${TMPDIR:-/tmp}/chda-helper.XXXXXXXX")
trap 'chda_helper_status=$?; rm -rf -- "$chda_helper_tmp" || :; exit "$chda_helper_status"' EXIT
chda_helper_stop() {
    trap '' HUP INT TERM
    kill -TERM "-$chda_helper_pid" 2>/dev/null || kill -TERM "$chda_helper_pid" 2>/dev/null || :
    wait "$chda_helper_pid" 2>/dev/null || :
    exit "$1"
}
exec 3<&0
APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR="$chda_helper_tmp" setsid -- '"#.to_vec();
        for &byte in image.as_os_str().as_bytes() {
            if byte == b'\'' {
                text.extend_from_slice(b"'\\''");
            } else {
                text.push(byte);
            }
        }
        text.extend_from_slice(
            br#"' "$@" <&3 3<&- &
chda_helper_pid=$!
exec 3<&-
trap 'chda_helper_stop 129' HUP
trap 'chda_helper_stop 130' INT
trap 'chda_helper_stop 143' TERM
if wait "$chda_helper_pid"; then exit 0; else exit "$?"; fi
"#,
        );
        text
    }

    pub(super) fn launcher(data: &Path, image: &Path) -> std::io::Result<PathBuf> {
        let directory = data.join("helpers");
        std::fs::create_dir_all(&directory)?;
        let metadata = std::fs::symlink_metadata(&directory)?;
        // SAFETY: geteuid has no arguments or memory access requirements.
        if !metadata.file_type().is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::other(
                "Helper directory is not owned by the current user",
            ));
        }
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        let mut hash = DefaultHasher::new();
        image.hash(&mut hash);
        let path = directory.join(format!("appimage-{:016x}", hash.finish()));
        let text = script(image);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(std::io::Error::other(
                    "Helper launcher is not a regular file",
                ));
            }
            Ok(metadata)
                if metadata.permissions().mode() & 0o777 == 0o700
                    && std::fs::read(&path)? == text =>
            {
                return Ok(path);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        crate::ipc::process::write_private_executable_atomic(&path, &text)?;
        Ok(path)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[cfg(target_os = "linux")]
        use std::{
            io::Write,
            process::{Command, Stdio},
        };

        #[test]
        fn appimage_helpers_keep_the_original_image_after_its_extract_directory_disappears() {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "chda-helper-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let directory = root.join("temporary mount");
            let executable = directory.join("usr/bin/chda");
            let image = root.join("chda package ' 世界.AppImage");
            std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
            std::fs::write(&executable, b"fixture").unwrap();
            std::fs::write(
                &image,
                b"#!/bin/sh\n[ \"${APPIMAGE_EXTRACT_AND_RUN-}\" = 1 ] || exit 42\nprintf '%s\\n' \"$TMPDIR\"\ncat\n",
            )
            .unwrap();
            std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(directory.join("com.magicsih.chda.desktop"), b"fixture").unwrap();
            let executable = executable.canonicalize().unwrap();
            let expected = image.canonicalize().unwrap();
            let source = source(&executable, Some(&directory), Some(&image)).unwrap();
            assert_eq!(source, expected);
            assert!(super::source(&executable, None, None).is_none());
            assert!(
                super::source(&expected, Some(&directory), Some(&image)).is_none(),
                "ignore another application's inherited AppImage environment"
            );
            let helper = launcher(&root.join("data"), &source).unwrap();
            std::fs::remove_dir_all(&directory).unwrap();
            #[cfg(target_os = "linux")]
            let run = || {
                let mut child = Command::new(&helper)
                    .env_remove("APPIMAGE_EXTRACT_AND_RUN")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all("HOOK 世界\n".as_bytes())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(output.status.success());
                let text = String::from_utf8(output.stdout).unwrap();
                let (temporary, payload) = text.split_once('\n').unwrap();
                assert_eq!(payload, "HOOK 世界\n");
                assert!(
                    !Path::new(temporary).exists(),
                    "remove only this helper's temporary directory"
                );
                temporary.to_owned()
            };
            #[cfg(target_os = "linux")]
            assert_ne!(
                run(),
                run(),
                "each invocation needs its own extraction directory"
            );
            assert_eq!(launcher(&root.join("data"), &source).unwrap(), helper);
            assert_eq!(
                std::fs::metadata(&helper).unwrap().permissions().mode() & 0o777,
                0o700
            );
            let metadata = root.join("metadata.json");
            crate::ipc::write_private_atomic(&metadata, b"{}").unwrap();
            assert_eq!(
                std::fs::metadata(metadata).unwrap().permissions().mode() & 0o777,
                0o600
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
