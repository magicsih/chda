# chda

**Checkout · Hack · Deliver · Again.**

A terminal with a git worktree side panel and a live status board for LLM coding agents.
Terminal core by [libghostty-vt](https://github.com/ghostty-org/ghostty), app in Rust, UI on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
macOS first; Linux and Windows build in CI.

Status: M2 in progress. Tabs and splits with Ghostty's default shortcuts, zsh shell integration (prompt marks, working directory, title), and a terminal that handles colors, wide glyphs, keyboard and IME input, mouse selection, mouse reporting and scrollback. Font, colors and padding come from your Ghostty config.

See [CONTRIBUTING.md](CONTRIBUTING.md) for build prerequisites and [docs/architecture.md](docs/architecture.md) for the crate layout.

## Why

One worktree, one task, one agent. chda shows which worktree is running which agent and what it is waiting for, and lets you create, merge, and prune worktrees from the side panel without leaving the terminal.

## License

MIT
