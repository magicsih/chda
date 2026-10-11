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

## Amendment (2026-10-11): first-turn helper notify

Codex 0.162 generates a new conversation's title through an ephemeral thread
([`tui/src/app/thread_title.rs`](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/app/thread_title.rs)
starts it with `ephemeral: true` in
[`tui/src/temporary_structured_request.rs`](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/temporary_structured_request.rs))
that still runs the legacy `notify` mid-turn. Treated as the
conversation's completion, it showed a working pane as idle with a close button
that skipped the confirmation, announced an unfinished turn and recorded an
unresumable conversation ID.

- `chda hook codex` reports a notify only for a thread with a saved transcript
  under `$CODEX_HOME/sessions`: a provider-confirmed child under its parent,
  otherwise the conversation itself. Codex saves a conversation before its
  first turn completes, so a thread without a transcript is an ephemeral
  helper and reports nothing. Neither does a sender whose transcripts cannot
  be searched to the end.
- While a pane's title reports Codex's run state, turns come from the title
  alone; a notify only names the conversation, whichever path arrives first.
- A repeated title state restores the pane's status when it fell behind, and
  closing a pane asks while its Codex title reports work or a wait for input.

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
