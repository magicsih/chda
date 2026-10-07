# Session-preserving updates

Status: implemented; production signing configuration and administrator-owned
installation acceptance remain release gates. No release is published by this PR.

## Decision

Use the official stable **Sparkle 2.10.0**, pinned by archive SHA-256, for
macOS download, Ed25519 signature verification, system authorization and app
replacement. The daily release check and its existing setting remain separate.
Download and installation require clicking Update. An app-owned controller
shares progress across windows and prevents duplicate attempts. Release notes
and version dismissal remain in a separate menu; Cancel is available before
session preparation. Automatic checks/downloads inside Sparkle are disabled.

The macOS bridge in `chda-ui/platform/update` runs in a signed standalone
helper embedded in the bundle. It hosts Sparkle with a custom user driver,
uses the original app as its host, and disables Sparkle's automatic relaunch.
A private per-attempt directory holds a verified copy of the old signed app,
progress, control markers and the event journal. The helper runs from this
copy, so replacing the original bundle does not remove its executable.

## Ownership and recovery

1. Download and verification happen while all windows remain usable. Native
   authorization cancellation reports an error to the original GUI.
2. Once Sparkle is ready, wait for in-flight repository mutations and their
   UI callbacks, terminal initialization and unfinished sheets or renames.
   Then serialize accepted hook events to the handoff journal, drain the
   preceding event queue, and freeze input, window closing and structural
   changes across the app. The waiting state remains cancellable.
3. Detach every terminal after draining its reader. Capture primary and
   alternate screens, scrollback, VT state, pane IDs, titles, live agent state,
   session start time, window bounds, tab layout and focus. A partial failure
   reconnects all detached panes before unfreezing the original GUI.
4. Start the retained app as `--update-broker`. It verifies the backup's code
   signature, validates the complete window/PTY mapping and acknowledges
   preparation before the GUI commits ownership and exits without destructors.
5. The broker retains every PTY master while Sparkle finishes installation.
   On success it checks the replacement against the old app's Apple code
   requirement and launches it with `--adopt` and inherited descriptors.
6. The successor prepares every window and terminal without consuming output,
   resizing the PTYs or acquiring permission to terminate shells. After all
   first frames exist, the broker commits ownership and the successor starts
   reading. AppKit state restoration is suppressed for this launch because
   the handoff, rather than macOS crash-history restoration, owns the windows.
7. Installation or preparation failure launches the retained signed app with
   the same descriptors and an error banner. Failed successors are killed
   and reaped before recovery, without running shell-owning destructors. If
   recovery itself fails, the broker keeps the descriptors and offers Retry.
   Once a successor has committed, the broker does not rewind consumed output.

The handoff envelope and descriptor list are bounded. Preparation has a
30-second timeout; installation has a five-minute deadline before recovery.
The terminal reader continues draining while a normal shell is terminated,
avoiding a macOS close-time hang when its final output fills the PTY buffer.

Hook clients wait for acknowledgement. During the freeze the IPC server
journals events before acknowledgement; each provisional GUI also journals
accepted events until commit. When no GUI is available, hooks append to a
locked fallback file. The successor replays journaled and fallback events
with the original session start time and pane IDs. Failed provisional GUIs
cannot consume the only durable copy of fallback events.

## Explicit boundary

The current Ghostty formatter does not serialize Kitty image storage. Any
terminal that has used inline graphics rejects preparation before its
resources are relinquished, and the app resumes all panes. This includes
image data no longer visible on screen. The error tells the user to close
that terminal before updating. Text-only shells and full-screen terminal
applications preserve their primary/alternate text and scrollback. Images
are never silently discarded to complete an update.

Linux, Windows, unbundled executables and unsigned development apps do not
install updates. Their existing release notice reports the supported macOS
installation requirement when clicked.

## Release configuration

- Bundle the signed helper and Sparkle framework, including its license, and
  sign nested code before the outer app. Produce the notarized universal ZIP.
- Configure the cataloged production Sparkle key separately, then set the
  release secret `SPARKLE_ED_KEY` and variable `SPARKLE_PUBLIC_KEY`. The first
  is the base64 Ed25519 seed, the second its public key. The release job writes
  the private seed only to a mode-restricted ephemeral file and removes it.
- `scripts/generate-appcast.py` uses the pinned official tools, validates the
  exact release URL/version/archive size and verifies the Ed25519 signature.
  Missing or mismatching signing input fails the release instead of producing
  a placeholder feed. Publish `appcast.xml` alongside the ZIP.
- Embed `https://github.com/magicsih/chda/releases/latest/download/appcast.xml`
  as the feed and the public key as `SUPublicEDKey`.
- The Homebrew cask declares `auto_updates true`. Existing v0.1.19 users need
  one manual upgrade (`brew upgrade --cask --greedy chda`) to reach the first
  updater-enabled release.

No production Sparkle key was found during implementation. Registration,
external secret changes, a public release and release acceptance on an
administrator-owned installation are separate tasks, not implied by a build.

## Verification

Automated Rust scenarios cover the update button/menu, duplicate launch
prevention, waiting for active tasks without freezing the shell, failed
handoff returning to the same live shell, transactional
image rejection, provisional process failure and event-journal/fallback races.
Native tests use isolated Developer ID signed apps and an ephemeral test
Ed25519 key, never the installed user app or production update credentials.
They exercise the official installer, signature/network/cancellation failures,
real broker/successor processes, multiple windows and splits, continuing shell
PIDs, the original active window, installation failure and new-executable
failure with backup recovery. Native narrow-window reconnection progress and
recovered terminal/error displays, plus desktop/narrow Pages, were inspected.
See [testing](../testing.md) for commands and outstanding release checks.

## Sources

- [Sparkle 2.10.0 installation protocol](https://github.com/sparkle-project/Sparkle/blob/2.10.0/Documentation/Installation.md)
- [SPUUpdaterDelegate](https://github.com/sparkle-project/Sparkle/blob/2.10.0/Sparkle/SPUUpdaterDelegate.h)
- [SPUUserDriver](https://github.com/sparkle-project/Sparkle/blob/2.10.0/Sparkle/SPUUserDriver.h)
- [Homebrew self-updating apps](https://docs.brew.sh/FAQ#how-does-brew-upgrade-handle-apps-that-update-themselves)
