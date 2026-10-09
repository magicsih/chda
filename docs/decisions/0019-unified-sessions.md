# ADR 0019: One Sessions list beside a separately scrolling PROJECT

Status: accepted within design 1.3 (2026-10-09). Issue: [#182](https://github.com/magicsih/chda/issues/182).
Replaces the ACTIVE/IDLE hierarchy of #151 and the shared-scroll reveal of
#149; amends the sidebar presentation of [0009](0009-live-idle-agents.md) and
[0015](0015-provider-confirmed-child-activity.md).

## Context

ACTIVE, IDLE and PROJECT shared one scroll handle. A session click went
through `focus_active`, which expanded PROJECT and revealed the worktree in
that shared viewport, pushing the session rows out of view. A live idle agent
appeared twice, as its tab in ACTIVE and its pane in IDLE, and moved between
sections when its status changed.

## Decision

- `chda-core` builds `SessionRow`s: one per pane that is `agent_live` or was
  launched by chda (`SessionKey::Pane`), one per other tab
  (`SessionKey::Tab`: Git tree and Diff review views, then terminals last).
  Rows follow tab order, then pane order. Status never changes a key or a
  place, and each status also has a word for tooltips.
- Sessions and PROJECT are sibling scroll containers with headers outside
  them. Sessions takes its content height up to 40% of the sidebar, or the
  remaining height while PROJECT is collapsed. Reveal, the sticky repository
  name and drag auto-scroll use PROJECT's handle only; nothing but the user
  scrolls Sessions.
- PROJECT highlights the focused pane's worktree. A focus change the user
  makes reveals it inside PROJECT with #149's minimal movement. Restoring
  focus after closing or dismissing something, window activation, startup
  restore and agents' `chda mcp` requests only update the highlight. Only
  explicit PROJECT navigation expands a collapsed PROJECT section. A
  notification's worktree whose pane is gone keeps the highlight until the
  focus moves.
- `sessions-collapsed` replaces `active-collapsed` and
  `idle-agents-collapsed`. When it is missing it is true only if both earlier
  keys were true. Loading never rewrites the file; the next save drops the
  earlier keys. Going back to v0.1.21 ignores it and shows both sections
  expanded, without data loss.

## Alternatives

A tab row with pane sub-rows nests three levels once child agents expand.
Leaving PROJECT untouched on session clicks hides the session's branch,
which the design review rejected. A draggable divider between the regions
is a larger change, to propose separately if needed.

## Verification

`scenarios::sessions` covers independent wheel scrolling and the 40% cap,
session clicks including split-pane agents, status changes, output and
child updates, PROJECT navigation, closing rows and the collapse migration;
`reorder::dragging_near_project_edges_scrolls_only_project`,
`chda-core` `sessions::tests` and `chda-config`
`legacy_session_collapse_keys_migrate` cover the rest. Pixels, small windows,
large text and both themes are checked in the native release rounds.
