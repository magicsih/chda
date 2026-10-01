# Architecture

chda is a Cargo workspace of eight crates. Arrows only point downward; the
direction is enforced by `deny.toml` (`cargo deny check bans`).

| Crate | Role | May depend on |
|---|---|---|
| `chda` (bin) | Entry point. GUI, and CLI subcommands such as `chda hook` | `chda-ui`, `chda-core`, `chda-agents`, `chda-config` |
| `chda-ui` | GPUI views, elements, theme. The only crate that knows GPUI | `chda-core`, `chda-term`, `chda-config`, `gpui` |
| `chda-core` | Domain hub: workspace, worktrees, agents, event bus | `chda-term`, `chda-git`, `chda-agents`, `chda-config` |
| `chda-term` | `libghostty-vt` wrapper. PTY bytes in, plain-struct frames out | `chda-pty`, `libghostty-vt` |
| `chda-pty` | PTY create/read/write/resize over `portable-pty` | `portable-pty` |
| `chda-git` | `GitBackend` trait; gix for reads, `git` subprocess for writes | `gix` |
| `chda-agents` | `AgentAdapter` trait, hook receiver, session indexer | — |
| `chda-config` | Ghostty config subset and chda's own TOML config | — |

Rules:

- Only `chda-ui` depends on GPUI. Only `chda-term` depends on `libghostty-vt`;
  no FFI type leaves that crate. `chda-core` knows nothing about GPUI,
  libghostty or gix.
- `#[cfg(target_os = ...)]` is allowed only in `chda-pty`, `chda-ui/platform`
  and `chda-agents/ipc`.
- GPUI is a git dependency pinned to a Zed commit in the workspace
  `Cargo.toml`. It is bumped once per milestone in its own pull request.

## Terminal data flow

`libghostty-vt` handles are `!Send`, so each session runs two threads:

- a reader thread that blocks on the PTY and forwards byte chunks;
- a terminal thread that owns the VT state, parses output, encodes input
  (keys, paste, focus, resize, scroll) and writes to the PTY.

The terminal thread publishes `Arc<Frame>` snapshots (plain structs: cells,
rows, cursor, colors, scrollbar, title, pwd) at most every 8 ms while output
streams, and sends `Event`s (frame, title, pwd, bell, clipboard, exit) over a
channel plus a wake callback. The UI never touches the VT state; it reads the
latest frame when it paints.

## Workspace model

`chda-core` holds the window model: tabs, each with a binary split tree of
pane ids, the focused pane, and per-pane info (title, cwd, bell). It does
geometry (layout in the unit square, directional focus, resize, zoom) but
knows nothing about views. `chda-ui`'s `WorkspaceView` maps pane ids to
`TerminalView` entities, renders the tab bar and split tree, and turns
terminal events (exit, title, cwd, bell, focus) into model updates.

## Shell integration

`chda-term` ships its own zsh scripts (`shell-integration/zsh`) that emit
OSC 133 prompt marks, OSC 7 working-directory reports and an OSC 2 title.
They are written to the per-user data directory on first use and loaded by
pointing `ZDOTDIR` at them, which then restores the user's own `ZDOTDIR`.
Shells without integration fall back to asking the OS for the foreground
process's directory (`chda-pty`, macOS only so far).

Decisions that are hard to reverse are recorded under `docs/decisions/`.
