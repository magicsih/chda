# ADR 0021: Linux distribution and update channel

Status: accepted within the approved Linux desktop design, 2026-10-18.

## Decision

Build the first Linux x86_64 executable on pinned Ubuntu 20.04, with glibc 2.31.
Audit imported GLIBC, GLIBCXX and CXXABI symbols in the finished executable and
every bundled library. Preserve the host's glibc, C++ runtime and graphics
loader/drivers and their Wayland/XCB client libraries. A host Mesa EGL driver
can require newer client symbols than the compatibility builder provides;
loading an older bundled client first breaks that driver. Bundle the remaining
linked libraries with relative RUNPATHs.
Do not export `LD_LIBRARY_PATH` into terminal shells or launched editors.
Include dependency copyright files and publish the unmodified distribution
source archives beside the binaries.

Stage one AppDir with a small `AppRun` launcher. Produce the AppImage and portable
archive from that same directory, and compare their complete payload manifests.
Pin both appimagetool and its runtime by version and SHA-256. The portable archive
is the installation path when a FUSE mount is unavailable; neither package needs
root access. Host Vulkan support is a runtime requirement, checked in native
sessions separately from the ABI audit.

AppImage agent commands use a private launcher in the user's chda data directory,
referencing the original image rather than its temporary mount. Every helper
uses extract-and-run in a separate temporary directory, including outside the
GUI environment and without FUSE, so overlapping hooks, MCP and the
running GUI cannot delete each other's runtime. The launcher requires util-linux
`setsid`, preserves protocol stdin, forwards direct termination to its child
process group and removes its own temporary directory on normal exit or trapped
signals. Portable binaries keep their installed executable path.

Linux updates are manual downloads from GitHub Releases. Linux release discovery
selects the newest stable release that contains a Linux x86_64 package, rather
than offering a macOS-only release. The title bar opens its release page and
does not offer the macOS session-preserving installation flow. Downloaded files
are verified against the published SHA-256 manifest before replacement.

Publish a Linux-only stable release with GitHub's `make_latest=false`, preserving
the existing macOS release, Sparkle appcast and Homebrew installation channel.
The shared workspace version and `v<version>` tag still belong to release-plz.
A later macOS release may skip a Linux-only version; its existing signed native
release gates continue to apply. Public installation links identify the current
published release for the chosen operating system.

## Verification and limits

Run the same candidate packages in Ubuntu LTS, Debian, Fedora and Arch sessions.
Use X11 and Wayland native windows with software Vulkan, and retain artifact
digests, display backend, distro digest, rendering device and screenshots.
Perform three independent UI review rounds for the client decorations. Workspace
tests use GPUI's test platform and are separate from native rendering evidence.
These runs do not establish physical-GPU or desktop-specific IME coverage.
The support matrix must describe the actually executed environments.

The first artifact is x86_64. ARM64, Alpine/musl, NixOS and Windows packaging
require their own decisions and execution evidence. This does not close the
broader hardware and Windows acceptance in issue #14.
