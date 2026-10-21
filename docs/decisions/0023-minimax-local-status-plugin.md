# 0023: MiniMax Code status through a managed local plugin

Status: accepted within the [approved initial provider scope](../design/2026-10-21-gemini-minimax/approval.md).

MiniMax Code 0.6.12 discovers local plugins in its active data directory. Its
interactive CLI has no per-launch plugin flag, and its hook runner inherits
PATH but discards custom environment variables, including `CHDA_PANE_ID`.

Install a chda-owned `chda-status` package only when starting a managed MiniMax
pane. Preserve user configuration, authentication, plugin enablement and session
databases. Refuse collisions or modified generated files. Keep recoverable
previous contents before atomic file replacement. App startup alone does not
install this package.

Prepend a private chda pane-context directory to the managed process's PATH.
The plugin reads that context, reduces the provider payload to status metadata
and invokes the validated chda helper with the exact pane and data directory.
Without a context it does nothing. Distinct panes carry distinct contexts;
child session IDs cannot replace the root conversation. Keep contexts during
process adoption and remove their owned files when their pane is closed.

This adds a launch preparation step separate from `AgentAdapter::install_hooks`,
whose startup work remains restricted to chda's own data directory. MiniMax
executable lookup uses `mcode`; its provider ID is `minimax`. Existing menus,
status rows, notifications and errors handle it without a new screen. Historical
MiniMax transcripts and exact resume are not exposed.

Reference: [MiniMax hook runner](https://github.com/MiniMax-AI/minimax-code/blob/5821f7c9b0363a9b97a57fff3deaa4cd2da165a4/packages/agent-modules/plugin-hooks/src/runner.ts).
