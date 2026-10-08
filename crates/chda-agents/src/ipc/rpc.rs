//! WebSocket JSON-RPC over the installed CLI's existing-daemon byte proxy.
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    process::{ChildStdin, ChildStdout, Command, Stdio},
    time::Duration,
};
use tungstenite::{Message, WebSocket, client::client_with_config, protocol::WebSocketConfig};

struct Pipe {
    input: ChildStdin,
    output: ChildStdout,
    read: usize,
}
impl Read for Pipe {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.read >= 33_554_432 {
            return Err(io::Error::other(
                "Local provider response exceeds its bound",
            ));
        }
        let limit = bytes.len().min(33_554_432 - self.read);
        let n = self.output.read(&mut bytes[..limit])?;
        self.read += n;
        Ok(n)
    }
}
impl Write for Pipe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.input.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.input.flush()
    }
}

pub struct LocalRpc {
    socket: WebSocket<Pipe>,
    next: u64,
}
fn unavailable() -> io::Error {
    io::Error::other("Local provider transport unavailable")
}
impl LocalRpc {
    pub fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.next += 1;
        let id = self.next;
        self.socket
            .send(Message::Text(
                json!({"id":id,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .map_err(|_| unavailable())?;
        loop {
            let message = self.socket.read().map_err(|_| unavailable())?;
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(result) = value.get("result") {
                return Ok(result.clone());
            }
            // Error messages may include input. Keep them out of UI and logs.
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Local provider query rejected",
            ));
        }
    }
    fn initialize(&mut self) -> io::Result<()> {
        self.request("initialize",json!({"clientInfo":{"name":"chda","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}))?;
        self.socket
            .send(Message::Text(
                json!({"method":"initialized"}).to_string().into(),
            ))
            .map_err(|_| unavailable())
    }
}

/// A deadline includes the handshake and all requests. The outer process
/// guard closes the pipes and reaps only our proxy on success or failure.
pub fn with_local_rpc<T: Send + 'static>(
    command: &mut Command,
    timeout: Duration,
    exchange: impl FnOnce(&mut LocalRpc) -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    super::with_process(command, |child| {
        let pipe = Pipe {
            input: child.stdin.take().unwrap(),
            output: child.stdout.take().unwrap(),
            read: 0,
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| {
                let config = WebSocketConfig::default()
                    .max_message_size(Some(1_048_576))
                    .max_frame_size(Some(1_048_576))
                    .max_write_buffer_size(2_097_152);
                let (socket, _) = client_with_config("ws://localhost/", pipe, Some(config))
                    .map_err(|_| unavailable())?;
                let mut rpc = LocalRpc { socket, next: 0 };
                rpc.initialize()?;
                exchange(&mut rpc)
            })();
            let _ = tx.send(result);
        });
        rx.recv_timeout(timeout).map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Local provider query deadline exceeded",
            )
        })?
    })
}
