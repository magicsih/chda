# Session-preserving updates

Status: approved direction; implementation incomplete. This change supplies
terminal handoff primitives, not a working in-app updater.

## Decision

For distributed macOS apps, use Sparkle 2.10.0 for download, signature
verification, system authorization and app replacement. Keep existing daily
update-check preferences. Download and installation begin only after the
user clicks Update; release notes and version dismissal remain separate.
One app-owned controller must publish progress to every window and reject
duplicate update attempts.

Use a separate process to keep sessions alive while the original GUI exits.
Sparkle cannot finish replacement until the target app terminates. Its
updater can outlive that target, and its delegate can disable automatic
relaunch. The broker, rather than Sparkle, must launch the successor with
`--adopt` after receiving installation completion.

Before quitting, retain a verified copy of the current signed app and hand
the broker all PTY masters, terminal snapshots, window bounds, tab layouts,
focus, pane IDs and agent state. Freeze input and structural changes only
once installation is ready. Preserve agent events throughout the handoff;
reopening the ordinary restore file is not live-session recovery.

The successor first prepares every window and terminal without reading
PTY output, resizing the PTYs or owning shell termination. Only after all
preparation succeeds may the broker grant ownership. Failed preparation
must release the successor's resources without killing the shell. The
broker keeps recovery descriptors until this decision.

Download failure or cancelled authorization leaves the old GUI running.
After the GUI exits, installation or successor preparation failure must
reopen the retained signed app with the same sessions and an error message.
Do not roll back to an obsolete snapshot after a successor has committed
and started consuming output.

## Implemented primitives

- PTY detach/adopt with an interruptible reader and recovery descriptors.
- Terminal snapshots of primary/alternate text screens, scrollback and VT
  state. Kitty graphics are **not** serialized by the existing snapshot
  implementation; complete graphical terminal preservation remains open.
- A bounded version 2 handoff envelope and explicit `prepared` / `commit`
  exchange. Abandoned or rejected successors are killed and reaped before
  the predecessor reuses its PTYs.
- Provisional terminal adoption: construction publishes the restored frame;
  output reading and shell termination ownership begin only on commit.
- Window/pane metadata serialization retaining pane IDs for hook routing.

The command-line entry point does not yet dispatch `--adopt`; the production
GUI does not yet use these primitives. A successful primitive test therefore
does not prove a session-preserving app update.

## Remaining integration and release gates

- Sparkle framework packaging and a macOS bridge under `chda-ui/platform`.
- Shared update UI, process supervision, all-window preparation, partial
  detach recovery, and input/window mutation guards.
- Continuous hook/agent event routing while the GUI is absent; switching
  listeners must not discard events accepted by the preceding listener.
- Verified old-app retention, native installation completion monitoring and
  successor/old-app launch with bounded recovery handling.
- Signed ZIP appcast generation and upload as `appcast.xml`; feed URL:
  `https://github.com/magicsih/chda/releases/latest/download/appcast.xml`.
- Provision the Sparkle signing key separately and embed its public key.
  Do not ship a placeholder key or publish an unsigned update feed.
- Add Homebrew `auto_updates true` when the updater actually works.
- Update the product catalog and generated README/Pages when the user flow
  is implemented, without describing this foundation as a shipped feature.

Release acceptance requires signed user-owned and administrator-owned app
installations, real distinct GUI processes with multiple windows/splits,
active jobs, text/scrollback, focus and agent events. Exercise network and
signature failures, authorization cancellation, partial handoff, installation
failure and successor preparation failure. Verify narrow-window progress
and errors and macOS tests plus Linux/Windows builds.

Users on v0.1.19 need one manual update to the first updater-enabled version.
This work does not authorize a release or external credential changes.

## Sources

- [Sparkle 2.10.0 installation protocol](https://github.com/sparkle-project/Sparkle/blob/2.10.0/Documentation/Installation.md)
- [SPUUpdaterDelegate](https://github.com/sparkle-project/Sparkle/blob/2.10.0/Sparkle/SPUUpdaterDelegate.h)
- [SPUUserDriver](https://github.com/sparkle-project/Sparkle/blob/2.10.0/Sparkle/SPUUserDriver.h)
- [Homebrew self-updating apps](https://docs.brew.sh/FAQ#how-does-brew-upgrade-handle-apps-that-update-themselves)
