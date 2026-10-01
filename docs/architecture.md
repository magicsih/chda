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

Decisions that are hard to reverse are recorded under `docs/decisions/`.
