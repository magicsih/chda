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
        match chda_session::Client::connect(data) {
            Ok(client) => return Ok(Some(client)),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) => {}
            Err(e) => return Err(e),
        }
        let executable = std::env::current_exe()?.with_file_name("chda-session");
        let retained = retain(data, &executable)?;
        chda_session::Client::connect_or_start(data, &retained).map(Some)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data;
        Ok(None)
    }
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
    chda_core::agents::ipc::write_private_atomic(&directory.join("chda-session"), &bytes)?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let target = directory.join("chda-session");
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
    Ok(target)
}

pub(crate) fn helper(client: Option<&chda_session::Client>) -> io::Result<PathBuf> {
    if let Some(client) = client {
        return client.helper_executable();
    }
    chda_core::agents::ipc::helper_executable()
}
