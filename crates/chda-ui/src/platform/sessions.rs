//! macOS service startup, retained executable integrity and platform selection.
use std::{
    io,
    path::{Path, PathBuf},
};

pub(crate) fn connect(data: Option<&Path>) -> io::Result<Option<chda_session::Client>> {
    #[cfg(target_os = "macos")]
    {
        let data = data.ok_or_else(|| io::Error::other("No session data directory"))?;
        std::fs::create_dir_all(data)?;
        chda_session::prepare_profile(data)?;
        let _startup = chda_session::startup_lease(data)?;
        let existing = match chda_session::Client::connect(data) {
            Ok(client) => Some(client),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                None
            }
            Err(e) => return Err(e),
        };
        let executable = std::env::current_exe()?.with_file_name("chda-session");
        let retained = retain(data, &executable)?;
        let client = match existing {
            Some(client) => client,
            None => chda_session::Client::connect_or_start(data, &retained)?,
        };
        client.set_pending_executable(retained.clone());
        client.upgrade_if_idle()?;
        cleanup(data, &[client.helper_executable()?, retained]);
        Ok(Some(client))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data;
        Ok(None)
    }
}

pub(crate) fn upgrade_if_idle(client: &chda_session::Client) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        if !client.upgrade_pending() {
            return Ok(());
        }
        let _startup = chda_session::startup_lease(client.data_directory())?;
        if client.upgrade_if_idle()? {
            cleanup(client.data_directory(), &[client.helper_executable()?]);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = client;
    Ok(())
}

#[cfg(target_os = "macos")]
fn retain(data: &Path, source: &Path) -> io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let bytes = std::fs::read(source).map_err(|_| {
        io::Error::other("Build or install chda-session beside chda before opening a macOS window")
    })?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let directory = data.join("session-service/bin").join(hash);
    // The retained tree is outside the replaced application bundle. Its
    // content hash binds helpers to the exact source executable.
    let target = directory.join("chda-session");
    if target.exists() {
        if std::fs::read(&target)? != bytes {
            return Err(io::Error::other(
                "The retained session executable no longer matches its content hash",
            ));
        }
    } else {
        chda_core::agents::ipc::write_private_atomic(&target, &bytes)?;
    }
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))?;
    let signature = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict"])
        .arg(&target)
        .output()?;
    if !signature.status.success() {
        return Err(io::Error::other(
            "The retained session executable failed code signature verification",
        ));
    }
    if Sha256::digest(std::fs::read(&target)?) != Sha256::digest(&bytes) {
        return Err(io::Error::other(
            "The retained session executable changed during preparation",
        ));
    }
    target.canonicalize()
}

#[cfg(target_os = "macos")]
fn cleanup(data: &Path, keep: &[PathBuf]) {
    use sha2::{Digest, Sha256};
    let root = data.join("session-service/bin");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() != 64
            || !name.bytes().all(|b| b.is_ascii_hexdigit())
            || !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir())
        {
            continue;
        }
        let executable = path.join("chda-session");
        if keep
            .iter()
            .any(|p| executable.canonicalize().ok().as_ref() == Some(p))
        {
            continue;
        }
        let Ok(Some(lease)) = chda_session::executable_lease(&executable, false) else {
            continue;
        };
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        if files
            .flatten()
            .any(|e| !matches!(e.file_name().to_str(), Some("chda-session" | "server.lock")))
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(&executable) else {
            continue;
        };
        if format!("{:x}", Sha256::digest(&bytes)) != name {
            continue;
        }
        if std::fs::remove_file(&executable).is_err() {
            continue;
        }
        drop(lease);
        let _ = std::fs::remove_file(path.join("server.lock"));
        let _ = std::fs::remove_dir(path);
    }
}

pub(crate) fn helper(client: Option<&chda_session::Client>) -> io::Result<PathBuf> {
    if let Some(client) = client {
        return client.helper_executable();
    }
    chda_core::agents::ipc::helper_executable()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn retention_never_replaces_or_cleans_a_live_or_unknown_executable() {
        let data = std::env::temp_dir().join(format!(
            "chda-retain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&data).unwrap();
        chda_session::prepare_profile(&data).unwrap();
        let first = retain(&data, Path::new("/usr/bin/true")).unwrap();
        let second = retain(&data, Path::new("/usr/bin/false")).unwrap();
        let unknown = first.with_file_name("user-file");
        std::fs::write(&unknown, b"preserve").unwrap();
        cleanup(&data, std::slice::from_ref(&second));
        assert!(first.exists(), "unknown files prevent directory cleanup");
        std::fs::remove_file(unknown).unwrap();
        let lease = chda_session::executable_lease(&first, false)
            .unwrap()
            .unwrap();
        cleanup(&data, std::slice::from_ref(&second));
        assert!(first.exists(), "running executable lease prevents cleanup");
        drop(lease);
        cleanup(&data, std::slice::from_ref(&second));
        assert!(!first.exists());
        assert!(second.exists());
        let original = std::fs::read(&second).unwrap();
        std::fs::write(&second, b"corrupted").unwrap();
        assert!(retain(&data, Path::new("/usr/bin/false")).is_err());
        assert_eq!(
            std::fs::read(&second).unwrap(),
            b"corrupted",
            "a mismatching retained executable is not overwritten"
        );
        std::fs::write(second, original).unwrap();
        std::fs::remove_dir_all(data).unwrap();
    }
}
