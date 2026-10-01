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

Candidates after M5, in no order: session restore, bash and fish integration,
an MCP server so agents can open worktrees and tabs, more agents (OpenCode,
Gemini CLI, Copilot CLI), GitLab and Gitea pull request status, ligatures and
Kitty graphics.

Decisions that are hard to reverse are summarized in `docs/decisions/`.
