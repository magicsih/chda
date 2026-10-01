# chda

**Checkout · Hack · Deliver · Again.**

A terminal with a git worktree side panel and a live status board for LLM coding agents.
Terminal core by [libghostty-vt](https://github.com/ghostty-org/ghostty), app in Rust, UI on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
macOS first; Linux and Windows build in CI.

Status: M1 in progress. The app runs your shell in one window: output, colors, wide glyphs, keyboard and IME input, mouse selection and copy, paste, mouse reporting and scrollback work, and it reads font, colors and padding from your Ghostty config.

See [CONTRIBUTING.md](CONTRIBUTING.md) for build prerequisites and [docs/architecture.md](docs/architecture.md) for the crate layout.

## Why

One worktree, one task, one agent. chda shows which worktree is running which agent and what it is waiting for, and lets you create, merge, and prune worktrees from the side panel without leaving the terminal.

## License

MIT
