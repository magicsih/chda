# Sessions that survive the window

On macOS, closing a window, quitting chda or losing the GUI leaves its terminal
processes in a separate local service. Open chda again to attach to the same
processes, screen, scrollback and inline images. A window closed while another
window remains can be reopened from the detached rows in **Sessions**.

Closing a terminal with cmd-w or closing a tab is an explicit stop. The existing
confirmation for active agents still applies. Closing an empty window has no
terminal to stop. Linux and Windows keep their existing process ownership.

## After the service or computer stops

With `restore-session = true`, the saved windows, tabs, splits, directories and
names return. Their previous text is read only. No saved command runs at startup.

- **Start shell** starts a shell in the saved directory.
- **Resume conversation** starts the exact supported conversation with its
  captured executable, options and permission choice. Missing executables,
  directories, conversation IDs or transcripts produce an explanation.
- **New conversation** is a separate choice for a captured agent. MiniMax does
  not expose verifiable exact resume; use this choice or Start shell.

Previous output remains separate from a new run. Repeated clicks during startup
start one successor. Typing in previous output never sends input to a process.
If a connection fails, chda reports it and does not replay uncertain input;
reopen to reconnect to a surviving server.

## Saving and privacy

The service is the only writer of the new layout and session checkpoints. It
saves layout and execution metadata as they change, and dirty text at most every
five seconds, plus disconnect and exit. Output follows `scrollback-limit`.
Only the current checkpoint and one validated previous copy are retained per
session. Files and sockets are private to the current user and data profile.
No full process environment or input event log is added to saved metadata.
Terminal output can contain sensitive text just as ordinary scrollback can.

The recovery status shows the last text save age. Abrupt power loss can lose
output since that save. Save failures keep live processes running; an error
shows the last successful save. Free the profile's storage or correct its
permissions before relying on a new checkpoint. Existing session.json is backed
up before migration. Damaged entries are isolated; valid entries and previous
checkpoints remain available. Migration itself starts no processes.

The service starts when chda needs it. It exits after no running session and no
GUI connection remains. There is no remote endpoint, login item or automatic
command execution after a reboot. Live processes cannot survive the operating
system shutting down; saved text and explicit conversation resume are the
recovery path.

## Updating while terminals are running

The first update from an app that owns its terminals uses the existing prepared
and commit handshake. The predecessor keeps its PTY descriptors until the new
service has prepared every terminal and receives the broker's commit directly.
A failed preparation leaves ownership with the predecessor. Inline images still
prevent this first transfer, as before; finish those terminals or remove their
images before updating.

Later GUI updates attach to the running service without moving PTYs or images.
The service and hook executable live in a verified, content-addressed private
profile directory outside the replaced app bundle. A running service holds a
lifetime lock; its executable is never overwritten or cleaned up. Once every
run and old terminal attachment ends, chda can retire the idle service and
start the bundled compatible replacement. It also checks for this idle
transition before a new window starts its first shell. Other clients prevent
retirement, and a failed or mismatching protocol probe leaves the current
server intact. Saved runs remain stopped.

The signed update feed declares its session protocol. The updater rejects a
missing or incompatible protocol before downloading/installing a replacement.
A direct installation with a mismatching protocol reports the connection error
and leaves the existing server and its processes alive. Protocol compatibility
does not authorize automatically restarting a conversation.
