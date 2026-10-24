> Historical record: the session-service releases were withdrawn on 2026-10-24. Current macOS distribution is v1.3.0. See [the rollback record](https://github.com/magicsih/chda/blob/main/docs/design/2026-10-24-session-service-rollback/design.md).

# Dedicated sessions and restart recovery

The owner's implementation request approves this design and its L3 test scope.
The implementation is delivered in three ordered changes: service foundations,
GUI attachment and recovery, then update adoption and release packaging. The
next macOS public release is the delivery target, subject to normal release
checks; no earlier native-validation exception applies.

## User journeys

| Action | Result |
| --- | --- |
| Quit, crash or close a window | Disconnect views; keep the server's processes and images |
| Close a tab or terminal | Existing confirmation, then terminate only those processes |
| Reopen chda | Attach to the same running session, run and PID |
| Restart after the server or computer stopped | Show saved text, timestamp and an explicit Start shell or Resume conversation action |
| Exact provider resume unavailable | Explain why; offer a separate new conversation action |

Restored text receives no input. Startup, migration and attachment never resume a
stopped process. No remote transport, login item or post-reboot command execution
is included. Linux and Windows keep their existing local session backend.

## Ownership and protocol

`chda-session` owns terminals through the existing `chda-term` and `chda-pty`
layers. Its executable has no GPUI dependency. The UI owns only attachments.
Dropping an attachment disconnects. Only Terminate ends a process. A profile lock
serializes competing startups before a stale socket can be removed. Private Unix
sockets authenticate the peer UID and bound request and response sizes. Version
mismatch leaves the old server and its processes alive.

Session, run and server identities are separate. Requests name all three;
hooks inherit the run identity. An old run cannot terminate, resize or update a
new one. Attach returns current state and a complete frame before subsequent
updates. Frames coalesce in the terminal frame slot; a slow consumer cannot stop
PTY output parsing. Effect sequences are bounded and attachment skips old
clipboard and notification effects. Uncertain input is never replayed.

The server receives hooks and handles read-only MCP coordination without a GUI.
GUI-only requests fail when there is no attached GUI. Repository boundaries and
unfinished-input protection remain part of the read-only contract.

## Storage and failure handling

The server is the sole writer of its layout and checkpoints. Layout and
execution changes are saved immediately. Dirty text is saved at most five seconds
apart, with extra checkpoints at disconnect and exit. Checkpoints retain text,
exact conversation metadata, captured launch choices and layout references;
extra process environment and input events are never persisted. Output retention
follows scrollback-limit. Each session keeps its current checkpoint and one
validated previous checkpoint, using private temporary files and atomic rename.
An interrupted write cannot replace the previous good checkpoint with corrupt
bytes. Legacy manifests are backed up before migration, without starting any
process. Malformed entries are isolated and recovered separately.

Save failures keep processes alive. The UI exposes the last successful save and
failure state; the last unsaved output may be lost after abrupt power loss.
Server restart simulation is not a physical power-loss test.

## Recovery screen

The existing process-pane layout is reused: a short status with save time,
read-only Previous output and one primary Resume conversation action when exact
resume is verified. Start shell is separate. An unavailable provider, missing cwd,
missing transcript or unverified conversation ID has a reason and cannot fall
back to another conversation. Detached windows are reachable from Sessions.
Small-window, keyboard, empty, error and save-failure states need runtime review.

## Updates

The existing prepared/commit PTY transfer remains the first-upgrade boundary.
Preparation must not read, resize or kill the previous owner's terminal. Failed
preparation returns to the previous app. Existing inline-image rejection remains
for that first transfer. Subsequent GUI updates reconnect to the service, retaining
its executable while live sessions exist. Incompatible updates are blocked before
installation. Retained server and hook executables live outside the replaced app;
they are verified before use and are cleaned only after their server is gone.

## Acceptance and release

See [test-plan.json](test-plan.json). All recorded results distinguish local
logic, process integration, GPUI scenarios, rendered native review and signed
artifact checks. The existing macOS release validator, signing, notarization,
Linux tests and Windows build stay required. Version and changelog generation
remain owned by release-plz. The user profile, installed application, live
sessions, provider accounts and credentials must remain untouched.
