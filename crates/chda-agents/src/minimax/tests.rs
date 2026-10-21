use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chda-mm-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn executable(&self, version: &str) -> PathBuf {
        let path = self.0.join("mcode");
        fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // The native CLI makes its own hook-cache directories read-only.
        // Only visit this freshly created fixture; never follow its symlinks.
        fn writable_dirs(path: &Path) {
            if !fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
                return;
            }
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    writable_dirs(&entry.path());
                }
            }
        }
        writable_dirs(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn startup_does_not_install_and_lookup_uses_mcode_not_provider_id() {
    let f = Fixture::new();
    let report = MiniMaxAdapter
        .install_hooks(&f.0, Path::new("/chda"))
        .unwrap();
    assert!(report.file.is_none());
    assert!(fs::read_dir(&f.0).unwrap().next().is_none());
    assert!(MiniMaxAdapter.executable(&f.0, f.0.as_os_str()).is_none());
    let executable = f.executable("0.6.12");
    assert_eq!(
        MiniMaxAdapter.executable(&f.0, f.0.as_os_str()),
        Some(executable.canonicalize().unwrap())
    );
}

#[test]
fn installation_preserves_user_files_and_detects_collisions_and_partial_updates() {
    let f = Fixture::new();
    let provider = f.0.join("provider");
    let data = f.0.join("data");
    fs::create_dir_all(&provider).unwrap();
    fs::write(provider.join("config.yaml"), "keep").unwrap();
    fs::write(provider.join("disabled-state.db"), "disabled").unwrap();
    let root = install_plugin(&provider, &data).unwrap();
    fs::write(root.join("user-note.txt"), "keep this too").unwrap();
    install_plugin(&provider, &data).unwrap();
    let mut old = package();
    old.insert("report.cjs".into(), b"// earlier chda helper\n".to_vec());
    fs::write(root.join("report.cjs"), &old["report.cjs"]).unwrap();
    fs::write(root.join(OWNER), serde_json::to_vec(&old).unwrap()).unwrap();
    install_plugin(&provider, &data).unwrap();
    let backup = fs::read_dir(data.join("hooks/minimax/plugin-backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let saved: BTreeMap<String, Vec<u8>> =
        serde_json::from_slice(&fs::read(backup).unwrap()).unwrap();
    assert_eq!(saved, old, "recovery contains the exact previous bytes");
    fs::write(root.join("report.cjs"), "user changed or interrupted").unwrap();
    assert!(install_plugin(&provider, &data).is_err());
    assert_eq!(
        fs::read_to_string(root.join("report.cjs")).unwrap(),
        "user changed or interrupted"
    );
    assert_eq!(
        fs::read_to_string(root.join("user-note.txt")).unwrap(),
        "keep this too"
    );
    assert_eq!(
        fs::read_to_string(provider.join("config.yaml")).unwrap(),
        "keep"
    );
    assert_eq!(
        fs::read_to_string(provider.join("disabled-state.db")).unwrap(),
        "disabled"
    );
    let other = f.0.join("other/plugins/chda-status");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("user"), "original").unwrap();
    assert!(install_plugin(&f.0.join("other"), &data).is_err());
    assert_eq!(fs::read_to_string(other.join("user")).unwrap(), "original");
    let linked = f.0.join("linked/plugins");
    fs::create_dir_all(&linked).unwrap();
    symlink(&root, linked.join("chda-status")).unwrap();
    assert!(install_plugin(&f.0.join("linked"), &data).is_err());
}

#[test]
fn preparation_honors_data_overrides_and_isolates_and_removes_each_pane() {
    let f = Fixture::new();
    let exe = f.executable("0.6.12");
    let data = f.0.join("data");
    let provider = f.0.join("custom provider");
    let mut environment = vec![
        (
            "MINIMAX_DATA_DIR".into(),
            provider.to_string_lossy().into_owned(),
        ),
        ("MAVIS_DATA_DIR".into(), String::new()),
        ("__MAVIS_RUNTIME_PROFILE".into(), String::new()),
    ];
    let path = std::env::var_os("PATH").unwrap();
    let prepare = |pane, environment: &[(String, String)]| {
        prepare_launch(&LaunchPreparation {
            home: &f.0,
            cwd: &f.0,
            data_dir: &data,
            hook_bin: &exe,
            executable: &exe,
            pane,
            search_path: &path,
            environment,
        })
    };
    let first = prepare(11, &environment).unwrap();
    let second = prepare(12, &environment).unwrap();
    assert_eq!(
        std::env::split_paths(&first).next(),
        Some(context_dir(&data, 11))
    );
    assert_ne!(first, second);
    assert!(provider.join("plugins/chda-status/hooks.json").is_file());
    assert!(!f.0.join(".minimax").exists());
    remove_context(&data, 11);
    assert!(!context_dir(&data, 11).exists());
    assert!(context_dir(&data, 12).exists());
    environment[0].1.clear();
    environment[1].1 = "legacy-selected".into();
    prepare(13, &environment).unwrap();
    assert!(
        f.0.join("legacy-selected/plugins/chda-status/hooks.json")
            .is_file()
    );
    environment[1].1.clear();
    prepare(14, &environment).unwrap();
    assert!(
        f.0.join(".minimax/plugins/chda-status/hooks.json")
            .is_file()
    );
    environment[2].1 = "opaque-profile".into();
    assert!(prepare(15, &environment).is_err());
    assert!(!context_dir(&data, 15).exists());
    environment[2].1.clear();
    f.executable("0.6.11");
    assert!(prepare(16, &environment).is_err());
    assert!(!context_dir(&data, 16).exists());
}

#[test]
fn permission_failure_preserves_the_existing_provider_configuration() {
    let f = Fixture::new();
    let provider = f.0.join("provider");
    let plugins = provider.join("plugins");
    fs::create_dir_all(&plugins).unwrap();
    fs::write(provider.join("config.yaml"), "original settings").unwrap();
    fs::set_permissions(&plugins, fs::Permissions::from_mode(0o500)).unwrap();
    let result = install_plugin(&provider, &f.0.join("data"));
    fs::set_permissions(&plugins, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(provider.join("config.yaml")).unwrap(),
        "original settings"
    );
    assert!(!plugins.join("chda-status").exists());
}

#[test]
fn native_payload_mapping_rejects_children_and_does_not_invent_completion() {
    use crate::hook::{HookKind, minimax_event};
    let mut input =
        json!({"session_id":"root", "cwd":"/work/app", "hook_event_name":"SessionStart"});
    for (event, expected) in [
        ("SessionStart", HookKind::SessionStart),
        ("UserPromptSubmit", HookKind::PromptSubmitted),
        ("PermissionRequest", HookKind::WaitingInput),
        ("PostToolUse", HookKind::PromptSubmitted),
        ("Stop", HookKind::Stopped),
        ("SessionEnd", HookKind::SessionEnd),
    ] {
        input["hook_event_name"] = event.into();
        let report = minimax_event(&input).unwrap();
        assert_eq!(
            (
                report.agent.as_str(),
                report.session_id.as_str(),
                report.kind
            ),
            ("minimax", "root", expected)
        );
    }
    input["hook_event_name"] = "Cancelled".into();
    assert!(minimax_event(&input).is_none());
    input["hook_event_name"] = "Stop".into();
    input["agent_id"] = "child".into();
    assert!(minimax_event(&input).is_none());
    input.as_object_mut().unwrap().remove("agent_id");
    input["session_id"] = "".into();
    assert!(minimax_event(&input).is_none());
}

#[test]
#[ignore = "requires an explicitly selected installed mcode; uses a fresh isolated HOME and provider data directory"]
fn installed_cli_discovers_the_plugin_and_keeps_it_disabled() {
    let executable = std::env::var_os("CHDA_TEST_MCODE").expect("select CHDA_TEST_MCODE");
    let f = Fixture::new();
    let provider = f.0.join("provider");
    let data = f.0.join("data");
    install_plugin(&provider, &data).unwrap();
    let run = |args: &[&str]| {
        let mut command = Command::new(&executable);
        command
            .args(args)
            .env_clear()
            .env("HOME", &f.0)
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("MINIMAX_DATA_DIR", &provider)
            .current_dir(&f.0);
        let out = ipc::capture_command(
            &mut command,
            None,
            std::time::Duration::from_secs(20),
            1024 * 1024,
        )
        .unwrap();
        serde_json::from_slice::<serde_json::Value>(&out).unwrap_or_else(|_| {
            panic!("CLI did not return JSON: {}", String::from_utf8_lossy(&out))
        })
    };
    let first = run(&[
        "plugin",
        "list",
        "--available",
        "--marketplace",
        "local",
        "--json",
    ]);
    assert!(first.to_string().contains("chda-status"), "{first}");
    eprintln!("Isolated local discovery: {first}");
    let disabled = run(&["plugin", "disable", "chda-status@local", "--json"]);
    eprintln!("Isolated disable result: {disabled}");
    install_plugin(&provider, &data).unwrap();
    let second = run(&["plugin", "list", "--marketplace", "local", "--json"]);
    eprintln!("After repeated installation: {second}");
    assert!(second.to_string().contains("\"enabled\":false"), "{second}");
}
