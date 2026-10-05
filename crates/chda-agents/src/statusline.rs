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

fn existing_status_line(home: &Path, cwd: &Path) -> Option<Value> {
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
            (status.get("type")?.as_str()? == "command" && status.get("command")?.is_string())
                .then(|| status.clone())
        })
        .next_back()
}

pub fn launch_settings(
    home: &Path,
    cwd: &Path,
    dir: &Path,
    hook_bin: &Path,
) -> std::io::Result<PathBuf> {
    let mut settings = crate::claude::hooks_settings(hook_bin);
    let mut status = existing_status_line(home, cwd).unwrap_or_else(|| json!({"type":"command"}));
    let mut command = format!("{} statusline", shell_quote(hook_bin));
    if let Some(existing) = status.get("command").and_then(Value::as_str) {
        command.push_str(&format!(" --then {}", shell_quote(Path::new(existing))));
    }
    status["command"] = command.into();
    settings["statusLine"] = status;
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
        if let Ok(output) = crate::ipc::capture_command(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_preserves_project_status_line_options_without_changing_user_files() {
        let root = std::env::temp_dir().join(format!("chda-statusline-{}", std::process::id()));
        let project = root.join("project");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(project.join(".claude")).unwrap();
        std::fs::create_dir_all(project.join("src")).unwrap();
        let config = project.join(".claude/settings.local.json");
        let original = r#"{"statusLine":{"type":"command","command":"printf user-status","padding":2,"refreshInterval":1000}}"#;
        std::fs::write(&config, original).unwrap();
        let path = launch_settings(
            &root.join("home"),
            &project.join("src"),
            &root.join("data"),
            Path::new("/opt/chda/chda"),
        )
        .unwrap();
        let settings: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let line = &settings["statusLine"];
        assert_eq!(line["padding"], 2);
        assert_eq!(line["refreshInterval"], 1000);
        assert_eq!(
            line["command"],
            "/opt/chda/chda statusline --then 'printf user-status'"
        );
        assert!(settings.get("hooks").is_some());
        assert_eq!(std::fs::read_to_string(config).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }
}
