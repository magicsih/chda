//! Native-shaped hook payloads pass through the generated Node plugin and the
//! real `chda hook minimax` process, without a model or a user's provider data.
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chda_agents::minimax::{LaunchPreparation, install_plugin, prepare_launch, remove_context};
use chda_agents::{HookEvent, HookKind, ipc};
use serde_json::json;

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn send(script: &Path, path: &std::ffi::OsStr, home: &Path, payload: serde_json::Value) {
    // Match MiniMax's safeHookEnvironment: custom CHDA_* and XDG_* are absent.
    let plugin = script.parent().unwrap();
    let hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(plugin.join("hooks.json")).unwrap()).unwrap();
    // The native runner expands ${PLUGIN_ROOT} before starting the shell.
    let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .replace("${PLUGIN_ROOT}", &plugin.to_string_lossy());
    let mut child = Command::new("/bin/sh")
        .args(["-c", &command])
        .current_dir(home)
        .env_clear()
        .env("PATH", path)
        .env("HOME", home)
        .env("PLUGIN_ROOT", plugin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty() || out.stdout == b"{}\n");
}

#[test]
fn managed_plugin_routes_only_root_metadata_to_the_real_hook() {
    let root = Root(std::env::temp_dir().join(format!("chda-mm-hook-{}", std::process::id())));
    fs::create_dir(&root.0).unwrap();
    let data = root.0.join("custom chda data");
    let provider = root
        .0
        .join("provider 'quoted' $(touch unexpected-shell-file)");
    let exe = root.0.join("mcode");
    fs::write(&exe, "#!/bin/sh\necho 0.6.12\n").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    let environment = vec![
        (
            "MINIMAX_DATA_DIR".into(),
            provider.to_string_lossy().into_owned(),
        ),
        ("__MAVIS_RUNTIME_PROFILE".into(), String::new()),
    ];
    let normal_path = std::env::var_os("PATH").unwrap();
    let prepare = |pane| {
        prepare_launch(&LaunchPreparation {
            home: &root.0,
            cwd: &root.0,
            data_dir: &data,
            hook_bin: Path::new(env!("CARGO_BIN_EXE_chda")),
            pane,
            executable: &exe,
            search_path: &normal_path,
            environment: &environment,
        })
        .unwrap()
    };
    let path1 = prepare(11);
    let path2 = prepare(12);
    let script = install_plugin(&provider, &data).unwrap().join("report.cjs");
    let payload = |event: &str, session: &str| {
        json!({
            "hook_event_name": event, "session_id": session, "cwd": root.0,
            "prompt": "private prompt", "tool_input": {"secret":"private tool input"},
            "transcript_path": "/private/transcript", "permission_mode":"auto",
        })
    };
    send(
        &script,
        &normal_path,
        &root.0,
        payload("SessionStart", "outside"),
    );
    assert!(ipc::take_fallback(&data).unwrap().is_empty());
    send(&script, &path1, &root.0, payload("SessionStart", "root-1"));
    send(&script, &path2, &root.0, payload("SessionStart", "root-2"));
    send(
        &script,
        &path1,
        &root.0,
        payload("UserPromptSubmit", "child-session"),
    );
    send(
        &script,
        &path1,
        &root.0,
        payload("SessionStart", "child-session"),
    );
    let mut child_start = payload("SessionStart", "child-session");
    child_start["agent_id"] = "child".into();
    send(&script, &path1, &root.0, child_start);
    for event in [
        "UserPromptSubmit",
        "PermissionRequest",
        "PostToolUse",
        "Stop",
        "SessionEnd",
    ] {
        send(&script, &path1, &root.0, payload(event, "root-1"));
    }
    send(&script, &path1, &root.0, payload("Cancelled", "root-1"));
    let journal = ipc::take_fallback(&data).unwrap();
    for private in [
        "private prompt",
        "private tool input",
        "/private/transcript",
        "\"tool_input\"",
    ] {
        assert!(
            !journal.contains(private),
            "the actual hook log contains metadata only"
        );
    }
    let events: Vec<HookEvent> = journal
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events.len(),
        7,
        "unmanaged, child and cancellation payloads report nothing: {events:?}"
    );
    assert_eq!(events[0].pane, Some(11));
    assert_eq!(
        (events[1].pane, events[1].session_id.as_str()),
        (Some(12), "root-2")
    );
    assert_eq!(
        events.iter().skip(2).map(|e| e.kind).collect::<Vec<_>>(),
        [
            HookKind::PromptSubmitted,
            HookKind::WaitingInput,
            HookKind::PromptSubmitted,
            HookKind::Stopped,
            HookKind::SessionEnd,
        ]
    );
    assert!(
        events
            .iter()
            .skip(2)
            .all(|e| e.pane == Some(11) && e.session_id == "root-1" && e.agent == "minimax")
    );
    // Pane numbers can repeat in a new app process. A fresh managed launch
    // must admit its new root even when the previous app left routing state.
    let fresh_path = prepare(11);
    let mut fresh_start = payload("SessionStart", "fresh-process-root");
    fresh_start["source"] = "startup".into();
    send(&script, &fresh_path, &root.0, fresh_start);
    send(
        &script,
        &fresh_path,
        &root.0,
        payload("UserPromptSubmit", "fresh-process-root"),
    );
    let fresh_events: Vec<HookEvent> = ipc::take_fallback(&data)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        fresh_events.len(),
        2,
        "a new managed launch must replace the earlier process's root binding"
    );
    assert!(
        fresh_events
            .iter()
            .all(|event| { event.pane == Some(11) && event.session_id == "fresh-process-root" })
    );
    remove_context(&data, 11);
    send(&script, &path1, &root.0, payload("Stop", "root-1"));
    assert!(ipc::take_fallback(&data).unwrap().is_empty());
    assert!(
        !root.0.join("unexpected-shell-file").exists(),
        "the native command must not evaluate the provider path"
    );
}
