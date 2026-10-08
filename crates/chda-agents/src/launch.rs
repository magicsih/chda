//! Explicit launch choices, independent from status hooks and global config.
use crate::{AgentId, SessionId};

/// Captured launches remain attached to their exact provider conversation,
/// even after its pane closes or a preset changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchHistory {
    contexts: std::collections::BTreeMap<String, AgentLaunchContext>,
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    version: u32,
    history: LaunchHistory,
}

impl LaunchHistory {
    pub fn load(data_dir: &std::path::Path) -> std::io::Result<Self> {
        match std::fs::read(data_dir.join("agent-launches.json")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            result => {
                let file: HistoryFile = serde_json::from_slice(&result?)?;
                if file.version != 1 {
                    return Err(std::io::Error::other(
                        "Unsupported agent launch history version",
                    ));
                }
                Ok(file.history)
            }
        }
    }
    pub fn save(&self, data_dir: &std::path::Path) -> std::io::Result<()> {
        crate::ipc::write_private_atomic(
            &data_dir.join("agent-launches.json"),
            &serde_json::to_vec(&HistoryFile {
                version: 1,
                history: self.clone(),
            })?,
        )
    }
    pub fn record(&mut self, context: AgentLaunchContext) {
        if context.managed
            && let Some(session) = &context.session
            && !session.0.is_empty()
        {
            self.contexts
                .insert(format!("{}:{}", context.agent.as_str(), session.0), context);
        }
    }
    pub fn get(&self, agent: AgentId, session: &SessionId) -> Option<&AgentLaunchContext> {
        self.contexts
            .get(&format!("{}:{}", agent.as_str(), session.0))
    }
}
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicy {
    #[default]
    Inherit,
    Bypass,
}

impl PermissionPolicy {
    pub fn label(self, agent: AgentId) -> &'static str {
        match self {
            Self::Inherit => "CLI configuration — effective policy unobserved",
            Self::Bypass if agent == AgentId::Codex => "Bypass — approvals and sandbox disabled",
            Self::Bypass => "Bypass — permission checks disabled",
        }
    }
}

/// Only chda-managed launches have a trustworthy command/options record.
/// Hook/status-line options are regenerated separately on exact resume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLaunchContext {
    pub agent: AgentId,
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub options: Vec<String>,
    pub policy: PermissionPolicy,
    pub session: Option<SessionId>,
    pub managed: bool,
}

/// Preserve preset arguments and reject explicit policy conflicts. Inherit
/// adds no flags and does not claim to observe config/profile effective policy.
pub fn permission_arguments(
    agent: AgentId,
    policy: PermissionPolicy,
    preset: &[String],
) -> Result<Vec<String>, String> {
    let bypass = match agent {
        AgentId::Claude => "--dangerously-skip-permissions",
        AgentId::Codex => "--dangerously-bypass-approvals-and-sandbox",
        _ if policy == PermissionPolicy::Inherit => return Ok(preset.to_vec()),
        _ => return Err("Permission selection is available for Claude Code and Codex".into()),
    };
    let mut has_bypass = false;
    let mut has_bypass_flag = false;
    let mut conflicting = Vec::new();
    let mut i = 0;
    while i < preset.len() {
        let arg = &preset[i];
        if arg == "--" {
            break;
        }
        let (mut flag, mut inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(f, v)| (f, Some(v)));
        if agent == AgentId::Codex
            && !arg.starts_with("--")
            && let Some(short) = ["-a", "-s", "-c"]
                .into_iter()
                .find(|short| arg.starts_with(short) && arg.len() > short.len())
        {
            flag = short;
            inline = Some(arg[short.len()..].trim_start_matches('='));
        }
        let mut value = inline;
        let takes_policy_value = match agent {
            AgentId::Claude => flag == "--permission-mode",
            AgentId::Codex => matches!(
                flag,
                "-a" | "--ask-for-approval" | "-s" | "--sandbox" | "-c" | "--config"
            ),
            _ => false,
        };
        if takes_policy_value && value.is_none() {
            i += 1;
            value = preset.get(i).map(String::as_str);
        }
        let explicit_bypass = flag == bypass
            || (agent == AgentId::Codex && flag == "--yolo")
            || (agent == AgentId::Claude
                && flag == "--permission-mode"
                && value == Some("bypassPermissions"));
        if explicit_bypass {
            has_bypass = true;
        }
        has_bypass_flag |= flag == bypass || (agent == AgentId::Codex && flag == "--yolo");
        let override_flag = match agent {
            AgentId::Claude => {
                matches!(flag, "--permission-mode" | "--restricted") && !explicit_bypass
            }
            AgentId::Codex if matches!(flag, "-c" | "--config") => value.is_some_and(|v| {
                v.split_once('=').is_some_and(|(key, _)| {
                    matches!(key.trim(), "approval_policy" | "sandbox_mode")
                })
            }),
            AgentId::Codex => matches!(
                flag,
                "-a" | "--ask-for-approval"
                    | "-s"
                    | "--sandbox"
                    | "--approve-for-me"
                    | "--not-so-yolo"
                    | "--full-auto"
            ),
            _ => false,
        };
        if override_flag && policy == PermissionPolicy::Bypass {
            conflicting.push(flag.to_owned());
        }
        // A scalar option value is data even when it resembles another flag.
        if inline.is_none()
            && matches!(
                flag,
                "--model"
                    | "-m"
                    | "--settings"
                    | "--append-system-prompt"
                    | "--system-prompt"
                    | "--resume"
                    | "-r"
                    | "--profile"
                    | "-p"
                    | "--effort"
                    | "--session-id"
                    | "--cd"
                    | "-C"
            )
        {
            i += 1;
        }
        i += 1;
    }
    if policy == PermissionPolicy::Inherit && has_bypass {
        return Err(format!(
            "Preset requests bypass. Select Bypass explicitly, or remove {bypass} / the bypass permission mode from the preset."
        ));
    }
    if !conflicting.is_empty() {
        return Err(format!(
            "Bypass conflicts with preset options: {}. Use CLI configuration or edit those preset options; chda will not remove them.",
            conflicting.join(", ")
        ));
    }
    let mut args = preset.to_vec();
    if policy == PermissionPolicy::Bypass && !has_bypass_flag {
        let end = args
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(args.len());
        args.insert(end, bypass.into());
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn default_policy_preserves_arguments_and_bypass_is_explicit() {
        for (agent, flag) in [
            (AgentId::Claude, "--dangerously-skip-permissions"),
            (AgentId::Codex, "--dangerously-bypass-approvals-and-sandbox"),
        ] {
            let preset = args(&["--model", "custom-model"]);
            assert_eq!(
                permission_arguments(agent, PermissionPolicy::Inherit, &preset).unwrap(),
                preset
            );
            assert_eq!(
                permission_arguments(agent, PermissionPolicy::Bypass, &preset).unwrap(),
                args(&["--model", "custom-model", flag])
            );
            assert!(
                permission_arguments(agent, PermissionPolicy::Inherit, &args(&[flag])).is_err()
            );
            assert_eq!(
                permission_arguments(agent, PermissionPolicy::Bypass, &args(&[flag])).unwrap(),
                args(&[flag])
            );
        }
    }
    #[test]
    fn conflicting_preset_modes_are_rejected_without_rewriting_the_preset() {
        for (agent, options) in [
            (AgentId::Claude, args(&["--permission-mode", "plan"])),
            (AgentId::Claude, args(&["--permission-mode=acceptEdits"])),
            (AgentId::Claude, args(&["--restricted"])),
            (AgentId::Codex, args(&["-a", "on-request"])),
            (AgentId::Codex, args(&["--sandbox=read-only"])),
            (AgentId::Codex, args(&["--approve-for-me"])),
            (AgentId::Codex, args(&["-aon-request"])),
            (AgentId::Codex, args(&["-sread-only"])),
            (AgentId::Codex, args(&["-capproval_policy='on-request'"])),
            (
                AgentId::Codex,
                args(&["-c", "approval_policy = 'on-request'"]),
            ),
            (
                AgentId::Codex,
                args(&["--config=sandbox_mode='workspace-write'"]),
            ),
        ] {
            assert!(
                permission_arguments(agent, PermissionPolicy::Bypass, &options).is_err(),
                "{options:?}"
            );
            assert_eq!(
                permission_arguments(agent, PermissionPolicy::Inherit, &options).unwrap(),
                options
            );
        }
        assert!(
            permission_arguments(
                AgentId::Claude,
                PermissionPolicy::Inherit,
                &args(&["--permission-mode=bypassPermissions"])
            )
            .is_err()
        );
        assert!(
            permission_arguments(
                AgentId::Codex,
                PermissionPolicy::Inherit,
                &args(&["--yolo"])
            )
            .is_err()
        );
    }
    #[test]
    fn positional_text_and_status_config_are_not_permission_options() {
        let options = args(&[
            "-c",
            "tui.terminal_title=['app-name']",
            "--",
            "--sandbox=read-only",
        ]);
        let result =
            permission_arguments(AgentId::Codex, PermissionPolicy::Bypass, &options).unwrap();
        assert_eq!(
            result,
            args(&[
                "-c",
                "tui.terminal_title=['app-name']",
                "--dangerously-bypass-approvals-and-sandbox",
                "--",
                "--sandbox=read-only"
            ])
        );
        let options = args(&["--append-system-prompt", "--dangerously-skip-permissions"]);
        assert_eq!(
            permission_arguments(AgentId::Claude, PermissionPolicy::Inherit, &options).unwrap(),
            options
        );
        assert!(permission_arguments(AgentId::Gemini, PermissionPolicy::Bypass, &[]).is_err());
    }
    #[test]
    fn recorded_options_and_policy_round_trip_for_the_exact_session() {
        let context = AgentLaunchContext {
            agent: AgentId::Codex,
            executable: "/test/bin/codex".into(),
            cwd: "/test/work tree".into(),
            options: args(&["--model", "saved-model"]),
            policy: PermissionPolicy::Bypass,
            session: Some(SessionId("exact-session".into())),
            managed: true,
        };
        assert_eq!(
            serde_json::from_str::<AgentLaunchContext>(&serde_json::to_string(&context).unwrap())
                .unwrap(),
            context
        );
        let dir = std::env::temp_dir().join(format!("chda-launch-history-{}", std::process::id()));
        let mut history = LaunchHistory::default();
        history.record(context.clone());
        history.save(&dir).unwrap();
        let loaded = LaunchHistory::load(&dir).unwrap();
        let session = context.session.as_ref().unwrap();
        assert_eq!(loaded.get(AgentId::Codex, session), Some(&context));
        assert!(loaded.get(AgentId::Claude, session).is_none());
        assert!(
            loaded
                .get(AgentId::Codex, &SessionId("session".into()))
                .is_none()
        );
        std::fs::write(dir.join("agent-launches.json"), "broken").unwrap();
        assert!(LaunchHistory::load(&dir).is_err());
        std::fs::write(
            dir.join("agent-launches.json"),
            r#"{"version":2,"history":{"contexts":{}}}"#,
        )
        .unwrap();
        assert!(LaunchHistory::load(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bypass_mode_alias_gets_the_explicit_claude_flag() {
        let preset = args(&["--permission-mode", "bypassPermissions"]);
        assert_eq!(
            permission_arguments(AgentId::Claude, PermissionPolicy::Bypass, &preset).unwrap(),
            args(&[
                "--permission-mode",
                "bypassPermissions",
                "--dangerously-skip-permissions"
            ])
        );
    }
}
