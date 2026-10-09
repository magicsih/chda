# 0009: Keep live idle agents distinct from saved conversations

Status: accepted (2026-10-05). Sidebar presentation amended by
[0019](0019-unified-sessions.md): live idle agents are rows of the Sessions
list instead of a separate IDLE section.

## Context

Clearing a pane's agent when its finished output was reviewed made a live
interactive session indistinguishable from a plain shell. A saved conversation
ID and terminal-output age establish neither process life nor idle duration.

## Decision

- Keep `PaneAgent` after review, with `Idle` status and the original completion
  timestamp. Track whether a runtime signal explicitly named the pane in
  `PaneInfo::agent_live`. Worktree-only fallback reports continue to drive
  attention but cannot create live idle rows. Restored conversation metadata
  does not restore runtime status.
- Codex's existing requested OSC Ready title establishes an idle live pane at
  startup. Claude and other hook adapters establish life with pane-bound
  lifecycle events. Session start shows idle with unknown duration; completion
  gives a known start. Repeated SessionStart for the same live conversation
  preserves its state during compaction/resume. Output inactivity never changes
  an agent's status.
- Claude's `idle_prompt` [notification](https://code.claude.com/docs/en/hooks#notification)
  is distinct from permission requests. It establishes idle if no known
  completion exists, but never clears unread `Review` or resets known timing.
- Session end, a shell prompt, and pane closure end runtime tracking. Late
  events naming a closed pane never fall back to another pane. An end event
  for a different known conversation cannot end the replacement conversation.
- Derive sidebar idle rows from live panes in tab/pane order. Show agent,
  repository/worktree, tab and pane, and a tooltip with the directory. A dash
  means the idle start is unavailable; terminal-output age remains in ACTIVE.
- Idle-row focus targets the exact pane. Its close button uses #108's pane
  close path, including process shutdown and revalidation. Closing a whole tab
  containing working or waiting agents still asks for confirmation. Idle is
  never a reason to terminate automatically.

## Boundaries

This consumes existing hooks and terminal titles rather than scanning process
names or transcript timestamps. Unsupported agents or launches bypassing the
configured lifecycle integration cannot establish a live idle entry. Process
exit without a SessionEnd hook is detected when the shell prompt returns;
custom shells without prompt integration cannot provide that exit signal.
No live flag or idle timestamp is persisted across application restarts.
