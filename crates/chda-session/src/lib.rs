//! GUI-independent terminal ownership and restart checkpoints.
mod client;
mod protocol;
mod server;
mod store;
pub use client::{Client, RemoteSession, Session};
pub use protocol::{Action, Identity, PROTOCOL_VERSION, SessionRecord, State};
pub use server::serve;
pub use store::Checkpoint;

pub fn socket_path(data: &std::path::Path) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let data = data.canonicalize().unwrap_or_else(|_| data.to_owned());
    let digest = format!("{:x}", Sha256::digest(data.as_os_str().as_encoded_bytes()));
    chda_pty::local::socket_root().join(format!("{}.sock", &digest[..32]))
}
