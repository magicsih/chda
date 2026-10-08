//! `chda mcp`: a Model Context Protocol server over stdio, so a coding agent
//! can ask the running chda window for a worktree, open a tab or report its
//! own status without hooks.
//!
//! The server holds no state of its own: every tool call becomes a
//! [`Request`] (or a hook event) sent over the hook socket, and the app does
//! the work with its own config. It speaks both protocol eras: revision
//! 2026-07-28, where each request carries its version in `_meta`, and the
//! `initialize` handshake of 2025-11-25 and earlier.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::control::{Reply, Request};
use crate::hook::{HookEvent, HookKind, PANE_ENV, data_dir, now_ms};
use crate::{AgentId, ipc};

/// Revisions served statelessly, with the version in each request's `_meta`.
pub const MODERN_VERSIONS: [&str; 1] = ["2026-07-28"];
/// Revisions served after an `initialize` handshake, newest first.
pub const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_KEY: &str = "io.modelcontextprotocol/clientInfo";

const INVALID_PARAMS: i64 = -32602;
const METHOD_NOT_FOUND: i64 = -32601;
const PARSE_ERROR: i64 = -32700;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const INSTRUCTIONS: &str = "chda is the terminal this agent runs in. It shows git worktrees \
in a sidebar with the status of each coding agent. Use create_worktree to start separate \
work on its own branch and folder, open_tab to open a terminal tab, list_worktrees to see \
the repository's worktrees, and report_status to tell the user whether you are working, \
waiting for their input or done.";

/// Entry point for `chda mcp`. Serves stdin until it closes.
pub fn mcp_main(_args: &[String]) -> i32 {
    let Some(dir) = data_dir() else {
        eprintln!("chda mcp: no home directory");
        return 1;
    };
    let mut server = Server {
        socket: ipc::socket_path(&dir),
        cwd: std::env::current_dir().unwrap_or_default(),
        pane: std::env::var(PANE_ENV).ok().and_then(|v| v.parse().ok()),
        client: None,
    };
    let stdout = io::stdout();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if let Some(out) = server.handle_line(&line) {
            let mut out_lock = stdout.lock();
            if writeln!(out_lock, "{out}")
                .and_then(|()| out_lock.flush())
                .is_err()
            {
                break;
            }
        }
    }
    0
}

/// One `chda mcp` process: where the app listens and where the agent runs.
pub struct Server {
    pub socket: PathBuf,
    /// The agent's working directory, the default for paths.
    pub cwd: PathBuf,
    /// The chda pane the agent runs in.
    pub pane: Option<u64>,
    /// The client's self-reported name, e.g. `claude-code`.
    pub client: Option<String>,
}

struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl Server {
    /// Answer one JSON-RPC message; `None` for notifications and responses.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        if line.trim().is_empty() {
            return None;
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error_response(
                &Value::Null,
                RpcError::new(PARSE_ERROR, "Parse error"),
            ));
        };
        let id = message.get("id")?.clone();
        let method = message.get("method")?.as_str()?.to_owned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(match self.dispatch(&method, &params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
            Err(e) => error_response(&id, e),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, RpcError> {
        let meta = params.get("_meta");
        let modern = match meta.and_then(|m| m.get(VERSION_KEY)) {
            Some(version) => {
                let requested = version.as_str().unwrap_or_default();
                if !MODERN_VERSIONS.contains(&requested) {
                    return Err(RpcError {
                        code: UNSUPPORTED_PROTOCOL_VERSION,
                        message: "Unsupported protocol version".into(),
                        data: Some(json!({
                            "supported": MODERN_VERSIONS.iter().chain(&LEGACY_VERSIONS).collect::<Vec<_>>(),
                            "requested": requested,
                        })),
                    });
                }
                if let Some(name) = meta
                    .and_then(|m| m.pointer(&format!("/{}/name", CLIENT_KEY.replace('/', "~1"))))
                    .and_then(Value::as_str)
                {
                    self.client = Some(name.to_owned());
                }
                true
            }
            None => false,
        };
        let mut result = match method {
            "initialize" => {
                if let Some(name) = params.pointer("/clientInfo/name").and_then(Value::as_str) {
                    self.client = Some(name.to_owned());
                }
                let requested = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let version = LEGACY_VERSIONS
                    .iter()
                    .find(|v| **v == requested)
                    .unwrap_or(&LEGACY_VERSIONS[0]);
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": server_info(),
                    "instructions": INSTRUCTIONS,
                })
            }
            "server/discover" => json!({
                "supportedVersions": MODERN_VERSIONS,
                "capabilities": { "tools": {} },
                "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
                "instructions": INSTRUCTIONS,
            }),
            "ping" => json!({}),
            "tools/list" => {
                let mut list = json!({ "tools": tools() });
                if modern {
                    list["ttlMs"] = json!(3_600_000);
                    list["cacheScope"] = json!("public");
                }
                list
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::new(INVALID_PARAMS, "missing tool name"))?;
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let outcome = self.call(name, &args)?;
                tool_result(outcome)
            }
            _ => return Err(RpcError::new(METHOD_NOT_FOUND, "Method not found")),
        };
        if modern {
            result["resultType"] = json!("complete");
        }
        Ok(result)
    }

    /// Run a tool. `Err` is a protocol error (unknown tool); a failed
    /// action is an `Ok` reply with `ok: false`, which the model sees.
    fn call(&self, name: &str, args: &Value) -> Result<Reply, RpcError> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let agent = match text("agent") {
            Some(id) if AgentId::parse(&id).is_none() => {
                return Ok(Reply::err(format!("unknown agent {id}")));
            }
            other => other,
        };
        let request = match name {
            "create_worktree" => Request::CreateWorktree {
                cwd: self.cwd.clone(),
                branch: text("branch"),
                base: text("base"),
                note: text("note"),
                open: args.get("open").and_then(Value::as_bool).unwrap_or(true),
                agent,
            },
            "open_tab" => Request::OpenTab {
                path: text("path").map_or_else(|| self.cwd.clone(), |p| self.cwd.join(p)),
                agent,
            },
            "list_worktrees" => Request::ListWorktrees {
                cwd: self.cwd.clone(),
            },
            "report_status" => return Ok(self.report_status(args, agent)),
            _ => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!("Unknown tool: {name}"),
                ));
            }
        };
        Ok(ipc::request(&self.socket, &request).unwrap_or_else(|e| not_running(&e)))
    }

    fn report_status(&self, args: &Value, agent: Option<String>) -> Reply {
        let kind = match args.get("status").and_then(Value::as_str) {
            Some("working") => HookKind::PromptSubmitted,
            Some("waiting_for_input") => HookKind::WaitingInput,
            Some("done") => HookKind::Stopped,
            Some("ended") => HookKind::SessionEnd,
            other => return Reply::err(format!("unknown status {other:?}")),
        };
        let Some(agent) = agent.or_else(|| self.client.as_deref().and_then(agent_of_client)) else {
            return Reply::err("say which agent you are with `agent`");
        };
        let event = HookEvent {
            child: None,
            agent,
            // One `chda mcp` process serves one agent session.
            session_id: format!("mcp-{}", std::process::id()),
            cwd: self.cwd.clone(),
            kind,
            timestamp: now_ms(),
            pane: self.pane,
        };
        match ipc::send(&self.socket, &event) {
            Ok(()) => Reply::ok("Status reported to chda", Value::Null),
            Err(e) => not_running(&e),
        }
    }
}

fn not_running(e: &io::Error) -> Reply {
    Reply::err(format!(
        "chda is not running or did not answer ({e}). Start chda and try again."
    ))
}

/// The agent id a client name belongs to, e.g. `claude-code` → `claude`.
fn agent_of_client(name: &str) -> Option<String> {
    let name = name.to_lowercase();
    AgentId::ALL
        .into_iter()
        .find(|id| name.contains(id.as_str()))
        .map(|id| id.as_str().to_owned())
}

fn server_info() -> Value {
    json!({ "name": "chda", "title": "chda", "version": env!("CARGO_PKG_VERSION") })
}

fn tool_result(reply: Reply) -> Value {
    let mut result = json!({
        "content": [{ "type": "text", "text": reply.message }],
        "isError": !reply.ok,
    });
    if reply.data.is_object() {
        result["structuredContent"] = reply.data;
    }
    result
}

fn error_response(id: &Value, e: RpcError) -> String {
    let mut error = json!({ "code": e.code, "message": e.message });
    if let Some(data) = e.data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string()
}

fn tools() -> Value {
    let agents: Vec<&str> = AgentId::ALL.iter().map(|a| a.as_str()).collect();
    json!([
        {
            "name": "create_worktree",
            "title": "Create a worktree",
            "description": "Create a git worktree with a new branch in the repository you are in, \
                and open a chda tab in it. Use it to start a separate task without touching the \
                current checkout. Returns the worktree's path and branch.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "branch": { "type": "string", "description": "New branch name. Omit for a random readable name." },
                    "base": { "type": "string", "description": "Branch or commit the new branch starts from. Default: the current HEAD." },
                    "note": { "type": "string", "description": "What the task is, shown in chda's sidebar (stored as the git branch description)." },
                    "open": { "type": "boolean", "description": "Open a tab in the new worktree. Default true." },
                    "agent": { "type": "string", "enum": agents, "description": "Coding agent to start in the new tab. Default: the user's default action." }
                }
            },
            "annotations": { "destructiveHint": false, "idempotentHint": false }
        },
        {
            "name": "open_tab",
            "title": "Open a tab",
            "description": "Open a new chda tab in a directory, with a shell or a coding agent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory, absolute or relative to yours. Default: your working directory." },
                    "agent": { "type": "string", "enum": agents, "description": "Coding agent to start. Default: a shell." }
                }
            },
            "annotations": { "destructiveHint": false }
        },
        {
            "name": "list_worktrees",
            "title": "List worktrees",
            "description": "List the worktrees of the repository you are in: branch, path, note, \
                uncommitted and unpushed changes, pull request and agent status.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "report_status",
            "title": "Report status",
            "description": "Tell the user, through chda's sidebar, tab dot and notifications, what \
                you are doing: working, waiting_for_input (you need an answer or approval), done \
                (the turn is finished) or ended (the session is over).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["working", "waiting_for_input", "done", "ended"] },
                    "agent": { "type": "string", "enum": agents, "description": "Which agent you are, when chda cannot tell from the MCP client name." }
                },
                "required": ["status"]
            },
            "annotations": { "destructiveHint": false, "idempotentHint": true }
        }
    ])
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::control::Incoming;
    use std::sync::mpsc;
    use std::time::Duration;

    fn server(name: &str) -> (Server, mpsc::Receiver<Incoming>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("chda-mcp-{name}-{}", std::process::id()));
        let socket = ipc::socket_path(&dir);
        let (tx, rx) = mpsc::channel();
        ipc::serve(&socket, tx, || {}).unwrap();
        let server = Server {
            socket,
            cwd: "/work/app".into(),
            pane: Some(4),
            client: None,
        };
        (server, rx, dir)
    }

    fn call(server: &mut Server, line: Value) -> Value {
        serde_json::from_str(&server.handle_line(&line.to_string()).unwrap()).unwrap()
    }

    fn modern(method: &str, params: Value) -> Value {
        let mut params = params;
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": { "name": "claude-code", "version": "2.1" },
            "io.modelcontextprotocol/clientCapabilities": {}
        });
        json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })
    }

    #[test]
    fn speaks_the_initialize_handshake_and_the_stateless_revision() {
        let (mut s, _rx, dir) = server("eras");
        let init = call(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "codex-mcp-client", "version": "1"}
            }}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "chda");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert_eq!(s.client.as_deref(), Some("codex-mcp-client"));
        let newer = call(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "2099-01-01"}}),
        );
        assert_eq!(newer["result"]["protocolVersion"], LEGACY_VERSIONS[0]);
        assert_eq!(
            s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
        );
        let list = call(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        );
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "create_worktree",
                "open_tab",
                "list_worktrees",
                "report_status"
            ]
        );
        assert!(
            list["result"].get("resultType").is_none(),
            "legacy results have none"
        );

        let discover = call(&mut s, modern("server/discover", json!({})));
        assert_eq!(
            discover["result"]["supportedVersions"],
            json!(MODERN_VERSIONS)
        );
        assert_eq!(discover["result"]["resultType"], "complete");
        let list = call(&mut s, modern("tools/list", json!({})));
        assert_eq!(list["result"]["resultType"], "complete");
        assert_eq!(list["result"]["ttlMs"], 3_600_000);
        assert_eq!(s.client.as_deref(), Some("claude-code"));

        let mut old = modern("tools/list", json!({}));
        old["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
        let rejected = call(&mut s, old);
        assert_eq!(rejected["error"]["code"], UNSUPPORTED_PROTOCOL_VERSION);
        assert_eq!(rejected["error"]["data"]["requested"], "1900-01-01");
        assert!(
            rejected["error"]["data"]["supported"]
                .as_array()
                .unwrap()
                .contains(&json!("2026-07-28"))
        );

        let unknown = call(&mut s, modern("tools/call", json!({"name": "rm_rf"})));
        assert_eq!(unknown["error"]["code"], INVALID_PARAMS);
        let missing = call(&mut s, modern("resources/list", json!({})));
        assert_eq!(missing["error"]["code"], METHOD_NOT_FOUND);
        let garbage = s.handle_line("{not json").unwrap();
        assert!(garbage.contains(&PARSE_ERROR.to_string()), "{garbage}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tools_relay_to_the_app_and_report_its_answer() {
        let (mut s, rx, dir) = server("tools");
        let app = std::thread::spawn(move || {
            let Ok(Incoming::Request(req, reply)) = rx.recv_timeout(Duration::from_secs(5)) else {
                panic!("no request");
            };
            assert_eq!(
                req,
                Request::CreateWorktree {
                    cwd: "/work/app".into(),
                    branch: Some("feat/x".into()),
                    base: None,
                    note: Some("Fix login".into()),
                    open: true,
                    agent: Some("codex".into()),
                }
            );
            reply
                .send(Reply::ok(
                    "Created feat/x",
                    json!({"path": "/work/app.worktrees/feat-x", "branch": "feat/x"}),
                ))
                .unwrap();
            let Ok(Incoming::Event(event)) = rx.recv_timeout(Duration::from_secs(5)) else {
                panic!("no event");
            };
            event
        });
        let created = call(
            &mut s,
            modern(
                "tools/call",
                json!({"name": "create_worktree", "arguments": {
                    "branch": "feat/x", "note": "Fix login", "agent": "codex"
                }}),
            ),
        );
        let result = &created["result"];
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"], "Created feat/x");
        assert_eq!(result["structuredContent"]["branch"], "feat/x");

        let reported = call(
            &mut s,
            modern(
                "tools/call",
                json!({"name": "report_status", "arguments": {"status": "waiting_for_input"}}),
            ),
        );
        assert_eq!(reported["result"]["isError"], false);
        let event = app.join().unwrap();
        assert_eq!(
            (event.agent.as_str(), event.kind, event.pane),
            ("claude", HookKind::WaitingInput, Some(4)),
            "the agent comes from the client's name"
        );

        let bad = call(
            &mut s,
            modern(
                "tools/call",
                json!({"name": "open_tab", "arguments": {"agent": "vim"}}),
            ),
        );
        assert_eq!(bad["result"]["isError"], true);

        std::fs::remove_dir_all(&dir).unwrap();
        let offline = call(
            &mut s,
            modern("tools/call", json!({"name": "list_worktrees"})),
        );
        assert_eq!(offline["result"]["isError"], true);
        assert!(
            offline["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("not running")
        );
    }
}
