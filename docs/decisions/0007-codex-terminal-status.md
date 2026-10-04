# 0007: Track directly typed Codex through its runtime terminal title

Status: proposed (2026-10-04)

## Context

A user selecting a branch and typing `codex` at its shell prompt received no
status tracking. The notify override was attached only by menu launches and
reports completion rather than the start of each turn. Native lifecycle
hooks require separate trust review, and installing them in the user's Codex
configuration would affect sessions outside chda.

## Decision

- In interactive zsh, bash and fish panes, supply a shell-local function for
  `codex`, preserving existing user aliases/functions. It forwards arguments
  and return status to `command codex` with a per-launch title override.
- Request `tui.terminal_title=["activity","app-name","run-state"]` for both
  direct shell and menu launches. No user dotfile, Codex config, authentication,
  hook trust or model setting changes.
- Parse only the requested layout in `chda-agents`; route terminal titles
  through the existing status path in `chda-ui`, explicitly naming the pane.
  Ready at startup is idle. Thinking, Working and Waiting mean work; Codex's
  Action Required title means user input. Ready after work completes a turn.
  Spinner frames and action-required blinking do not create repeated events.
- Keep the menu-launch notify integration for the actual session ID used by
  restore. Titles supply no session ID, so they never save a synthetic one.
  Completion arriving by both paths must announce the turn only once.
- Clear activity on a shell prompt, title clear or pane exit. This introduces
  no polling of transcripts, process scanning or new dependency.

## Evidence and boundaries

The public [configuration reference](https://developers.openai.com/codex/config-reference/)
defines `tui.terminal_title`. Layout, activity frames, state words and the
Action Required override were checked against the installed CLI's
[Codex 0.160.0 source](https://github.com/openai/codex/tree/rust-v0.160.0/codex-rs/tui/src):
`chatwidget/status_surfaces.rs` and `bottom_pane/title_setup.rs`.
[Native hooks](https://developers.openai.com/codex/hooks) have a trust-review
requirement and are not silently installed or trusted by chda.

The direct helper requires shell integration and a CLI supporting these title
items. It applies to new panes; custom functions, aliases, `command codex`,
absolute executable paths and user title overrides can bypass it. It tracks
interactive CLI sessions, not `codex exec`, whose output has no TUI title.
The layout is an upstream interface that must be checked when Codex changes;
regression tests use the recorded 0.160.0 layout and real local shells.
