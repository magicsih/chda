//! OpenCode: `opencode` CLI, status through a plugin loaded per launch from
//! `OPENCODE_CONFIG_DIR`.
//!
//! OpenCode keeps its sessions in a SQLite database, so chda does not list
//! them; a session can still be resumed by id with `opencode --session`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{
    AgentAdapter, AgentId, AgentSession, HookInstallReport, SessionId, which, write_if_changed,
};

pub struct OpenCodeAdapter;

const CONFIG_DIR_ENV: &str = "OPENCODE_CONFIG_DIR";

/// The directory chda points `OPENCODE_CONFIG_DIR` at.
pub fn config_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks").join("opencode")
}

/// The plugin: it reduces OpenCode's bus events to chda's hook kinds and
/// pipes each to `chda hook opencode` as JSON. Sessions that subagents start
/// (those with a parent) are left out.
pub fn plugin_source(hook_bin: &Path) -> String {
    let bin = serde_json::to_string(&hook_bin.to_string_lossy()).unwrap_or_default();
    format!(
        r#"// Written by chda: reports session status to the chda window this
// OpenCode runs in. Regenerated on every chda start.
import {{ spawn }} from "node:child_process";

const HOOK_BIN = {bin};

export const ChdaStatus = async ({{ directory }}) => {{
  const subagents = new Set();
  // One hook at a time, so chda sees the events in the order they happened.
  let queue = Promise.resolve();
  const report = (payload) =>
    new Promise((resolve) => {{
      try {{
        const child = spawn(HOOK_BIN, ["hook", "opencode"], {{
          stdio: ["pipe", "ignore", "ignore"],
        }});
        child.on("error", resolve);
        child.on("close", resolve);
        child.stdin.on("error", () => {{}});
        child.stdin.end(payload);
      }} catch {{
        resolve();
      }}
    }});
  const send = (kind, sessionID) => {{
    if (!sessionID || subagents.has(sessionID)) return;
    const payload = JSON.stringify({{ kind, session_id: sessionID, cwd: directory }});
    queue = queue.then(() => report(payload));
  }};
  return {{
    event: async ({{ event }}) => {{
      const p = event.properties ?? {{}};
      switch (event.type) {{
        case "session.created":
          if (p.info?.parentID) subagents.add(p.info.id);
          else send("session_start", p.info?.id);
          break;
        case "session.status":
          if (p.status?.type === "busy") send("prompt_submitted", p.sessionID);
          else if (p.status?.type === "idle") send("stopped", p.sessionID);
          break;
        case "permission.asked":
        case "question.asked":
          send("waiting_input", p.sessionID);
          break;
        case "session.deleted":
          send("session_end", p.info?.id);
          break;
      }}
    }},
  }};
}};
"#
    )
}

impl AgentAdapter for OpenCodeAdapter {
    fn id(&self) -> AgentId {
        AgentId::OpenCode
    }

    fn display_name(&self) -> &str {
        "OpenCode"
    }

    fn short_label(&self) -> String {
        "OC".into()
    }

    fn is_installed(&self) -> bool {
        which("opencode").is_some()
    }

    fn launch_command(&self, cwd: &Path, resume: Option<&SessionId>, _: &Path) -> Command {
        let mut cmd = Command::new("opencode");
        cmd.current_dir(cwd);
        // A config directory the user set already is theirs to keep.
        if std::env::var_os(CONFIG_DIR_ENV).is_none()
            && let Some(data_dir) = crate::hook::data_dir()
        {
            cmd.env(CONFIG_DIR_ENV, config_dir(&data_dir));
        }
        if let Some(id) = resume {
            cmd.arg("--session").arg(&id.0);
        }
        cmd
    }

    fn session_roots(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn parse_session(&self, _: &Path) -> Option<AgentSession> {
        None
    }

    fn install_hooks(&self, data_dir: &Path, hook_bin: &Path) -> io::Result<HookInstallReport> {
        let path = config_dir(data_dir).join("plugins").join("chda.js");
        write_if_changed(&path, &plugin_source(hook_bin))?;
        let note = std::env::var_os(CONFIG_DIR_ENV).map(|_| {
            format!("{CONFIG_DIR_ENV} is set, so OpenCode does not load chda's status plugin")
        });
        Ok(HookInstallReport {
            file: Some(path),
            note,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_plugin_and_resumes_by_session() {
        let dir = std::env::temp_dir().join(format!("chda-opencode-{}", std::process::id()));
        let report = OpenCodeAdapter
            .install_hooks(&dir, Path::new("/opt/chda \"bin\"/chda"))
            .unwrap();
        let file = report.file.unwrap();
        assert!(file.ends_with("hooks/opencode/plugins/chda.js"));
        let source = std::fs::read_to_string(file).unwrap();
        assert!(
            source.contains(r#"const HOOK_BIN = "/opt/chda \"bin\"/chda";"#),
            "{source}"
        );
        assert!(source.contains("export const ChdaStatus"));
        // Every kind the plugin sends is one chda's hook understands.
        for kind in [
            "session_start",
            "prompt_submitted",
            "stopped",
            "waiting_input",
            "session_end",
        ] {
            assert!(source.contains(&format!("send(\"{kind}\"")), "{kind}");
            assert!(serde_json::from_value::<crate::HookKind>(kind.into()).is_ok());
        }

        let cmd = OpenCodeAdapter.launch_command(
            Path::new("/work/app"),
            Some(&SessionId("ses_1".into())),
            Path::new("/usr/local/bin/chda"),
        );
        let argv = crate::command_argv(&cmd);
        assert_eq!(argv[argv.len() - 3..], ["opencode", "--session", "ses_1"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
