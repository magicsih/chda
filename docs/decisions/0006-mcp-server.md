# 0006: `chda mcp` relays to the running app over the hook socket

Status: accepted (2026-10-02)

## Context

Agents should be able to ask chda for a new worktree, open a tab or report
their own status without hooks (#22). The Model Context Protocol is how
coding agents call external tools, and every agent chda supports can start a
local MCP server over stdio. The window owns the state such requests touch:
the sidebar's repositories, the worktree path template, the tabs.

## Decision

- `chda mcp` is a stdio MCP server in `chda-agents`. It keeps no state: each
  tool call becomes a request to the running app over the existing hook
  socket, and the app does the work with its own config, as if the user had
  used the sidebar. Without a running app the tools return an error.
- The socket now carries two kinds of connections: hook events (one JSON
  line each, unchanged) and one request per connection, framed as
  `{"request": {...}}`, after which the client closes its writing half and
  reads one reply line. Each connection is served on its own thread, so a
  slow request does not delay hook events.
- The server speaks both protocol eras: revision 2026-07-28 (version in each
  request's `_meta`, `server/discover`) and the `initialize` handshake of
  2025-11-25 back to 2024-11-05, since the agents' clients move at
  different speeds.
- JSON-RPC is implemented directly on `serde_json`: four tools over stdio do
  not justify an async runtime and an SDK dependency.
- chda does not register itself in agents' MCP configs; the user adds it
  once per agent (`claude mcp add chda -- chda mcp`, ...).

## Read-only coordination (#249)

`list_sessions`, `read_output` and `wait_for_status` let a coordinating agent
follow the agents it started without the user relaying:

- They answer only for the repository the caller's directory is in. A pane in
  another repository is refused; wider access needs a new decision.
- Status comes from the same hooks, titles and provider-confirmed child
  activity as the sidebar (ADR 0015); a pane without a live agent is `ended`
  or `terminal`, and unknown child states stay counted as unknown.
- `read_output` never returns what the user is still typing. A live Claude
  Code or Codex session is read from its exact conversation's transcript
  (the latest answers, as in ADR 0016); a shell is read from its visible
  screen up to the current OSC 133 prompt. Other live agents and full-screen
  programs are refused, since their screen may hold an input box. Output is
  capped at 500 lines and 64 KB.
- `wait_for_status` answers at once when the state already matches, else
  when it is reached, the pane closes or the timeout (at most 55 seconds,
  below the socket's reply timeout) passes. The owning window keeps the wait.
- None of them changes focus, rows or seen state. Sending text to another
  agent remains out of scope ([ADR 0020](0020-session-addressed-prompt-delivery.md) proposes it).

## Consequences

- Tools act on the window the user sees and follow `config.toml`
  (worktree path template, default action); the sidebar updates at once.
- `report_status` produces the same events as hooks, so status, the Dock
  badge and notifications need no new code. The session id is per `chda mcp`
  process, which clients start per session.
- Windows needs a named-pipe transport for both hooks and requests (M5).
