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

## Consequences

- Tools act on the window the user sees and follow `config.toml`
  (worktree path template, default action); the sidebar updates at once.
- `report_status` produces the same events as hooks, so status, the Dock
  badge and notifications need no new code. The session id is per `chda mcp`
  process, which clients start per session.
- Windows needs a named-pipe transport for both hooks and requests (M5).
