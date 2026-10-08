//! Bounded local helper processes for agent discovery and telemetry.

/// Protect a newly created local file before its first private byte is written.
pub fn protect_file(_file: &std::fs::File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        _file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Atomically replace app-owned metadata; temporary files are private on Unix.
pub fn write_private_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Missing metadata directory"))?;
    std::fs::create_dir_all(parent)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut prepared = None;
    for _ in 0..128 {
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = parent.join(format!(".chda-metadata-{}-{id}", std::process::id()));
        match options.open(&temporary) {
            Ok(file) => {
                prepared = Some((temporary, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let (temporary, mut file) = prepared.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "Could not reserve a private metadata file",
        )
    })?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// Follow symlinks and reject directories, broken links and non-executable files.
pub fn is_executable(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Capture bounded stdout without letting shell startup or tools block the UI.
/// A private process group is terminated even when descendants keep stdout open.
pub fn capture_command(
    command: &mut std::process::Command,
    input: Option<&[u8]>,
    timeout: std::time::Duration,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    use std::{
        io::{Read, Write},
        process::Stdio,
        sync::mpsc,
        time::Instant,
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    struct Probe(std::process::Child);
    impl Drop for Probe {
        fn drop(&mut self) {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = Probe(command.spawn()?);
    let stdout = child.0.stdout.take().unwrap();
    let stdin = child.0.stdin.take();
    let bytes = input.map(<[u8]>::to_vec);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let (Some(mut stdin), Some(bytes)) = (stdin, bytes) {
            let _ = stdin.write_all(&bytes);
        }
        let mut output = Vec::new();
        let result = stdout.take(max_bytes as u64 + 1).read_to_end(&mut output);
        let result = result.and_then(|_| {
            if output.len() > max_bytes {
                Err(std::io::Error::other("tool output exceeded its bound"))
            } else {
                Ok(output)
            }
        });
        let _ = tx.send(result);
    });
    let deadline = Instant::now() + timeout;
    let output = rx.recv_timeout(timeout).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "tool did not return within its deadline",
        )
    })??;
    loop {
        if let Some(status) = child.0.try_wait()? {
            return if status.success() {
                Ok(output)
            } else {
                Err(std::io::Error::other("tool exited unsuccessfully"))
            };
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "tool did not exit within its deadline",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Run a streaming helper in a private process group and always reap it.
pub fn with_process<T>(
    command: &mut std::process::Command,
    exchange: impl FnOnce(&mut std::process::Child) -> std::io::Result<T>,
) -> std::io::Result<T> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    struct Guard(std::process::Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Guard(command.spawn()?);
    exchange(&mut child.0)
}

#[cfg(all(test, unix))]
mod quota_tests {
    use crate::quota::{CodexQuotaError, read_codex_with_timeout};
    use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

    struct Cli {
        root: PathBuf,
        bin: PathBuf,
    }

    impl Cli {
        fn new(account: &str, quota_action: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!("chda-quota-{}-{id}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            let bin = root.join("codex");
            let script = format!(
                r#"#!/bin/sh
cd "$(dirname "$0")"
echo $$ > pid
while IFS= read -r line; do
    printf '%s\n' "$line" >> requests
    case "$line" in
        *'"method":"initialize"'*) printf '%s\n' '{{"id":1,"result":{{}}}}' ;;
        *'"method":"account/read"'*) printf '%s\n' '{{"id":2,"result":{account}}}' ;;
        *'"method":"account/rateLimits/read"'*) {quota_action} ;;
    esac
done
"#
            );
            std::fs::write(&bin, script).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self { root, bin }
        }

        fn query(&self) -> Result<crate::quota::QuotaSnapshot, CodexQuotaError> {
            read_codex_with_timeout(&self.bin, 100, Duration::from_secs(5))
        }
    }

    impl Drop for Cli {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const ACCOUNT: &str = r#"{"account":{"type":"chatgpt","email":"test@example.test"}}"#;

    #[test]
    fn metadata_replacement_is_private_and_leaves_no_partial_file() {
        let cli = Cli::new(ACCOUNT, "exit 0");
        let path = cli.root.join("record.json");
        super::write_private_atomic(&path, b"first").unwrap();
        super::write_private_atomic(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!std::fs::read_dir(&cli.root).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".chda-metadata-")
        }));
    }

    #[test]
    fn quota_probe_uses_only_the_account_protocol_and_reaps_the_cli() {
        let cli = Cli::new(
            ACCOUNT,
            r#"printf '%s\n' '{"id":3,"result":{"rateLimits":{"primary":{"usedPercent":25,"windowDurationMins":300}}}}'"#,
        );
        let report = cli.query().unwrap();
        assert_eq!(report.scope, "test@example.test");
        assert_eq!(report.representative().unwrap().used_percent, 25.0);
        let requests: Vec<serde_json::Value> = std::fs::read_to_string(cli.root.join("requests"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let methods: Vec<_> = requests
            .iter()
            .map(|r| r["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "account/read",
                "account/rateLimits/read"
            ]
        );
        assert_eq!(requests[2]["params"]["refreshToken"], false);
        let pid: i32 = std::fs::read_to_string(cli.root.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "probe process was reaped"
        );
    }

    #[test]
    fn signed_out_and_api_key_accounts_do_not_request_subscription_quota() {
        for (account, expected) in [
            (r#"{"account":null}"#, CodexQuotaError::SignInRequired),
            (
                r#"{"account":{"type":"apiKey"}}"#,
                CodexQuotaError::Unsupported,
            ),
            (
                r#"{"account":{"type":"chatgpt","email":null}}"#,
                CodexQuotaError::Unsupported,
            ),
        ] {
            let cli = Cli::new(account, "exit 9");
            assert_eq!(cli.query().unwrap_err(), expected);
            assert!(
                !std::fs::read_to_string(cli.root.join("requests"))
                    .unwrap()
                    .contains("rateLimits")
            );
        }
    }

    #[test]
    fn transport_failure_retains_verified_scope_but_rpc_rejection_does_not() {
        let cli = Cli::new(ACCOUNT, "exit 0");
        let error = cli.query().unwrap_err();
        assert_eq!(error.scope(), Some("test@example.test"));
        assert!(error.details().contains("connection closed"));
        let cli = Cli::new(
            ACCOUNT,
            r#"printf '%s\n' '{"id":3,"error":{"code":-32000,"message":"must not echo provider errors"}}'"#,
        );
        let error = cli.query().unwrap_err();
        assert_eq!(error.scope(), None);
        assert!(!error.details().contains("must not echo"));
        let cli = Cli::new(
            ACCOUNT,
            r#"printf '%s\n' '{"id":3,"error":{"code":-32601}}'"#,
        );
        assert_eq!(cli.query().unwrap_err(), CodexQuotaError::Unsupported);
    }

    #[test]
    fn slow_quota_query_has_a_deadline_and_reaps_its_process() {
        let cli = Cli::new(ACCOUNT, "exec sleep 30");
        let error = read_codex_with_timeout(&cli.bin, 100, Duration::from_secs(5)).unwrap_err();
        assert_eq!(error.label(), "Retrying");
        assert!(error.details().contains("timed out"));
        let pid: i32 = std::fs::read_to_string(cli.root.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "timed-out helper was reaped"
        );
    }
}
