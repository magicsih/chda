//! GUI-independent terminal ownership and restart checkpoints.
mod client;
mod protocol;
mod server;
mod store;
pub use client::{Client, RemoteSession, Session, Watch};
pub use protocol::{Action, Identity, PROTOCOL_VERSION, SessionRecord, State};
pub use server::serve;
pub use store::Checkpoint;

pub fn prepare_profile(data: &std::path::Path) -> std::io::Result<()> {
    chda_pty::local::private_directory(&data.join("session-service"))
}
pub fn startup_lease(
    data: &std::path::Path,
) -> std::io::Result<Option<chda_pty::local::FileLease>> {
    prepare_profile(data)?;
    chda_pty::local::file_lease(&data.join("session-service/startup.lock"), true)
}
pub fn executable_lease(
    executable: &std::path::Path,
    wait: bool,
) -> std::io::Result<Option<chda_pty::local::FileLease>> {
    chda_pty::local::file_lease(&executable.with_file_name("server.lock"), wait)
}

pub fn socket_path(data: &std::path::Path) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let data = data.canonicalize().unwrap_or_else(|_| data.to_owned());
    let digest = format!("{:x}", Sha256::digest(data.as_os_str().as_encoded_bytes()));
    chda_pty::local::socket_root().join(format!("{}.sock", &digest[..32]))
}
