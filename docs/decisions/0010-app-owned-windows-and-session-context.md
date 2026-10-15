# App-owned windows and session context

Status: accepted implementation for #117, #118 and #128.

The environment owns live workspace windows, recent pane navigation and one
IPC receiver. Pane IDs are process-unique. Events with a pane ID reach that
pane's window; requests without a pane use the active window. Event wakes
reach every live window, so closing the original window leaves IPC operational.

Worktree navigation chooses the most recently focused matching pane across
windows. Explicit New Tab still creates another pane. Sidebar selection stays
window-local and uses exact tab identity. Navigation can expand and reveal a
worktree, while background refresh preserves deliberate collapse.

session.json stores a SavedSession envelope containing all windows and the
active-window index. Existing single-window JSON migrates when read. Each
window updates its snapshot; startup restore suppresses partial writes.
Normal quit preserves snapshots; explicit close removes only that window.
Layout, titles, directories, exact supported conversation IDs and last output
timestamps persist. Newly started processes are never marked live because they
were saved. Previous activity stays separate, and shell initialization does
not become new working activity.

## Moving panes and tabs (#248)

Tabs reorder by dragging within the tab bar; the saved order is the restored
order. A running pane moves to another tab, a new tab or another window
without restarting its process. Within a window the pane's terminal view
stays and only the split tree changes. Between windows the source detaches
the session as a session-preserving update does (ADR 0011), and the target
adopts it by its unchanged pane id, keeping scrollback, agent status, child
activity and conversation identity. Because pane ids are unique per process,
Sessions rows, notifications and agent routing follow the pane. A tab left
without panes closes without asking; a window left without tabs closes.

## Closing a tab keeps the branch (2026-10-15)

Closing the shown tab, whether its shell exited, its close button was
clicked or cmd-w was pressed, never moves the window to another branch.
The nearest tab of the same worktree (else directory) is shown, looking
after the closed tab first and then before it. When none is left, no tab is
shown: the sidebar keeps that worktree selected, the tab bar keeps its
repository's tabs, and + at the end of the tab bar or cmd-t opens a new
terminal in that worktree. A window whose last tab closes stays open; cmd-w
on a window without tabs closes it. Moving a window's last pane to another
window still closes the emptied window.

