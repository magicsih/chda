> Historical record: the session-service releases were withdrawn on 2026-10-24. Current macOS distribution is v1.3.0. See [the rollback record](https://github.com/magicsih/chda/blob/main/docs/design/2026-10-24-session-service-rollback/design.md).

# Dedicated session ownership

Status: accepted. Service ownership, explicit recovery and transactional update
adoption are implemented; release readiness depends on the recorded L3, native
review and exact signed-artifact gates.

## Decision

On macOS, move PTY and virtual-terminal ownership into a per-user, per-profile
`chda-session` process, independent of GPUI. Reuse `chda-term` for parsing, input,
images and retained scrollback. Keep Linux and Windows on the existing backend.
The new crate is platform-neutral; local socket, peer credentials and process
detachment stay in `chda-pty`, following the existing platform boundary.

Use a versioned, bounded local protocol with distinct session, execution and
server identities. Detachment cannot terminate a child. Reconnection returns a
current frame; old effects and uncertain input are not replayed. The service
owns hook processing and read-only coordination when there is no GUI.

Use private atomic text checkpoints and layout metadata for server restart
recovery. Restart never executes a saved command. Exact provider resume must be
explicit and validated using the existing adapter and captured launch context.
Disk failures are visible but do not terminate live processes.

## Rationale and alternatives

Keeping ownership in GPUI cannot survive force quit. PTY descriptor handoff is
appropriate for the first transition from an older app but requires a live
predecessor and cannot recover after a crash. A dedicated service preserves live
images and input modes without serializing Ghostty's private image store. Text
checkpoints cover process/server loss separately from live attachment.

The existing `restore-session` setting remains. No remote service, launch agent,
login item or automatic post-reboot command execution is introduced.

## Delivery and evidence

The [approved design](../design/2026-10-22-session-service/design.md) records
ownership, recovery UI, update compatibility and L3 acceptance. The first update
uses the existing broker preparation/commit boundary and preserves its inline
image restriction. The service receives the commit channel directly so that
GUI loss cannot interrupt ownership after the predecessor commits. Subsequent
GUI updates reconnect without transferring PTYs. Binary lifetime locks and a
profile startup lock prevent cleanup or idle replacement racing a live server. Normal release checks and immutable
signed-artifact verification remain mandatory for the next public release.
