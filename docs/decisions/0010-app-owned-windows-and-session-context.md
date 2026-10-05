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
