# Window notification history

Status: implemented; native visual review remains a release gate.

## Decision

Replace the transient sidebar message with a window-owned queue in
`chda-core::notifications`. Operational events append at their occurrence;
an ongoing branch update retains one stable entry as its progress changes.
Retried provider events with the same provider/session/pane/time/state key
do not create another unread arrival. No terminal content is collected.

The title-bar bell precedes the installed-app picker and stays available
without that picker or the sidebar. Entries show severity, an absolute date
and UTC time, message and known source context. Time uses the already locked
`time` crate's [Unix timestamp conversion](https://docs.rs/time/0.3.55/time/struct.OffsetDateTime.html#method.from_unix_timestamp_nanos).
The popup scrolls within the window. Closing restores a weak reference to
the previous input focus, falling back to the current terminal if it vanished.
Selecting a source event navigates only to its still-open, exact pane ID.

## Lifetime and reads

Opening snapshots an arrival watermark. New arrivals remain unread even
while the popup stays open; selecting a row reads that arrival. Operation
updates retain their row ID but advance the arrival sequence. Independent
events with equal wording remain separate. Timestamp ties use the arrival
sequence for deterministic ordering. History retains 500 entries and reports
the number dropped. Provider-event keys share that same bounded lifetime.

Cold session saves omit notification data, so normal restart clears history.
The live update envelope carries queues in the same order as its window
snapshots, including arrival sequence and read state. Adoption consumes one
queue for each restored window. Older handoffs default to empty queues.
The existing PTY preparation and commit protocol is unchanged.

OS notifications, Dock attention, quota controls and resource status remain
independent. Configuration polling records changed errors once and recovery
as a separate event; it does not clear earlier history.

## Verification

The consecutive-error scenario failed against the previous message renderer
because the title-bar queue did not exist. Queue tests cover read transitions,
equal-time order, operation updates, retry deduplication, capacity and handoff.
GPUI scenarios cover focus, exact navigation, per-window lifetime, hidden
controls and narrow bounds. These tests do not draw native pixels; the approved
release review separately verifies the popup in the macOS development runtime.
