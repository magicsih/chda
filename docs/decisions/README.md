# Architecture decision records

Summaries of decisions that are hard to reverse. Propose a change by adding a
new record rather than editing an accepted one.

| ADR | Decision |
|---|---|
| 0001 | Terminal core is `libghostty-vt` through C FFI, isolated in `chda-term` |
| 0002 | GUI framework is GPUI, pinned to a Zed git commit |
| 0003 | Worktree backend is in-app: gix for reads, `git` subprocess for writes |
| 0004 | macOS first; Linux and Windows must keep building in CI |
| 0005 | Name `chda`, MIT license, personal public repository |
| 0006 | `chda mcp` is a stateless MCP server relaying to the running app over the hook socket |
| 0007 | Proposed: directly typed Codex reports per-pane activity through runtime OSC titles |
| 0008 | Terminal and read-only Git graph tabs have explicit content types; history is queried in the background and rendered incrementally |
