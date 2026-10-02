# Roadmap

Milestones are cut at "usable" boundaries. Dates are not promised.

| Milestone | Scope | State |
|---|---|---|
| M0 | Workspace skeleton, CI on macOS, Linux and Windows | done |
| M1 | Terminal: libghostty-vt core, GPUI renderer, input, IME, selection, Ghostty config | done |
| M2 | Tabs, splits, zsh shell integration, working-directory tracking | done |
| M3 | Worktree sidebar, git badges, Claude Code and Codex status, sessions | done |
| M4 | Merge-and-clean, pull request badges, command palette, signed macOS release, Homebrew | done (v0.1.x) |
| M5 | Linux (Wayland/X11) and Windows (ConPTY) as first-class platforms | next |

Done since v0.1.3: scrollback search, clickable links, font size shortcuts,
live config reload, session restore, bash and fish integration, per-pane agent
status (tabs, ACTIVE list, Dock badge), notification click to the agent's pane
and jumping to the waiting agent.

Done since v0.1.6: Gemini CLI, GitHub Copilot CLI and OpenCode adapters, an
MCP server (`chda mcp`) so agents can create worktrees, open tabs and report
status, GitLab and Gitea pull request status, ligatures and Kitty graphics.

Decisions that are hard to reverse are summarized in `docs/decisions/`.
