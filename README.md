# chda

**Checkout · Hack · Deliver · Again.**

A terminal with a git worktree side panel and a live status board for LLM coding agents.
Terminal core by [libghostty-vt](https://github.com/ghostty-org/ghostty), app in Rust, UI on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
macOS first; Linux and Windows build in CI.

Status: M4 in progress. Merge-and-clean, bulk cleanup of merged worktrees, pull request badges via `gh`, a command palette (cmd-shift-p) and a macOS app bundle (`scripts/bundle-macos.sh`). A worktree sidebar (git badges, create and delete worktrees, agent status from Claude Code and Codex hooks, session resume), tabs and splits with Ghostty's default shortcuts, zsh shell integration, and a terminal that handles colors, wide glyphs, keyboard and IME input, mouse selection, mouse reporting and scrollback. Font, colors and padding come from your Ghostty config; chda's own settings live in `~/.config/chda/config.toml`.

See [CONTRIBUTING.md](CONTRIBUTING.md) for build prerequisites and [docs/architecture.md](docs/architecture.md) for the crate layout.

## Why

One worktree, one task, one agent. chda shows which worktree is running which agent and what it is waiting for, and lets you create, merge, and prune worktrees from the side panel without leaving the terminal.

## License

MIT
