//! Per-launch Claude status line bridge, preserving an existing status line.
use crate::{
    hook::{PANE_ENV, data_dir},
    ipc,
    quota::claude_quota,
    shell_quote,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

fn existing_command(home: &Path, cwd: &Path) -> Option<String> {
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let mut files = vec![config.join("settings.json")];
    // Project settings live at the repository root, not necessarily the pane cwd.
    let root = cwd
        .ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(cwd);
    files.extend([
        root.join(".claude/settings.json"),
        root.join(".claude/settings.local.json"),
    ]);
    files
        .into_iter()
        .filter_map(|p| {
            let value: Value = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
            let status = value.get("statusLine")?;
            (status.get("type")?.as_str()? == "command")
                .then(|| status.get("command")?.as_str().map(str::to_owned))
                .flatten()
        })
        .last()
}

pub fn launch_settings(
    home: &Path,
    cwd: &Path,
    dir: &Path,
    hook_bin: &Path,
) -> std::io::Result<PathBuf> {
    let mut settings = crate::claude::hooks_settings(hook_bin);
    let existing = existing_command(home, cwd);
    let mut command = format!("{} statusline", shell_quote(hook_bin));
    if let Some(existing) = existing {
        command.push_str(&format!(" --then {}", shell_quote(Path::new(&existing))));
    }
    settings["statusLine"] = json!({"type":"command", "command":command});
    // A per-cwd file avoids changing another live session's command on launch.
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut hash);
    let path = dir
        .join("hooks")
        .join(format!("claude-{:x}.json", hash.finish()));
    crate::write_if_changed(&path, &serde_json::to_string(&settings)?)?;
    Ok(path)
}

pub fn main(args: &[String]) -> i32 {
    let mut input = Vec::new();
    if std::io::stdin()
        .take(1_048_577)
        .read_to_end(&mut input)
        .is_err()
        || input.len() > 1_048_576
    {
        return 1;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(&input) {
        let pane = std::env::var(PANE_ENV).ok().and_then(|v| v.parse().ok());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if let Some(report) = claude_quota(&value, pane, now)
            && let Some(dir) = data_dir()
        {
            let _ = ipc::send_quota(&ipc::socket_path(&dir), &report);
        }
    }
    // Only an existing user command produces terminal status line output.
    if args.first().map(String::as_str) == Some("--then")
        && let Some(command) = args.get(1)
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        if let Ok(output) = chda_pty::capture_command(
            &mut cmd,
            Some(&input),
            std::time::Duration::from_secs(2),
            1_048_576,
        ) {
            let _ = std::io::stdout().write_all(&output);
        }
    }
    0
}
