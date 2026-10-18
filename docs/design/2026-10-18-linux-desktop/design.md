# Linux desktop support across distributions

Version: 1.0  
Status: approved; implementation in progress  
Baseline: `8f6f82224ecb7cbb1b2623737d8e17c04335d4c2`  
User direction: prioritize broad distribution support.  
Verification: [test-plan.json](test-plan.json).  
Delivery endpoint: reviewed implementation PRs, verified Linux packages and public release.
Approval: [implementation and publication](approval.md).

## Feasibility and current evidence

The existing Rust, GPUI and PTY architecture can be retained. The pinned GPUI
dependency enables both Wayland and X11. The baseline's
[Linux build passed](https://github.com/magicsih/chda/actions/runs/38929895941/job/117070886370),
but the Linux CI job only builds; it does not run workspace tests or validate a
desktop session. The latest public release, `v1.0.25`, has no Linux artifact.

Current source still has macOS-oriented shortcuts, no Linux foreground-directory
fallback, empty notification and installed-app integrations, and no Linux
packaging. The resource sampler also assumes `/usr/sbin/lsof`. These are scoped
as separate work below. Successful compilation does not establish usable input,
window controls, graphics, shell behavior or installation.

Existing work is tracked in [#14](https://github.com/magicsih/chda/issues/14),
[#185](https://github.com/magicsih/chda/issues/185),
[#186](https://github.com/magicsih/chda/issues/186),
[#187](https://github.com/magicsih/chda/issues/187),
[#188](https://github.com/magicsih/chda/issues/188) and the Linux portions of
[#196](https://github.com/magicsih/chda/issues/196) through
[#199](https://github.com/magicsih/chda/issues/199).
The earlier macOS batch approval explicitly excluded these issues; its direct
main exception does not apply. Use branches and Ready PRs for this work.

## Distribution and architecture contract

The proposed first architecture is x86_64. ARM64 remains a later candidate with
its own artifact and execution matrix; broad distribution support alone does
not establish ARM64 support.

| Item | First delivery |
| --- | --- |
| Distribution families | Ubuntu LTS, Debian, Fedora and Arch |
| Display systems | Wayland and X11 |
| Artifacts | AppImage and a portable archive from the same staged application directory |
| Compatibility target | glibc 2.31 or newer on x86_64, subject to artifact ABI and execution checks |
| Runtime graphics | Vulkan supported by the host; driver support checked separately from packaging |
| Writable state | Existing XDG config and data paths outside the package |

Pin actual distro image versions and digests when implementation starts, and
record them with every execution result. Use a sufficiently old build sysroot
and inspect the finished executable and bundled libraries for required GLIBC
and GLIBCXX symbols. Do not build on `ubuntu-latest` and assume the result runs
on older systems. If the compatibility target cannot be met, record the exact
dependency and obtain a revised support decision before raising the floor.

AppImage is a distribution format, not a compatibility layer. Its own
[packaging guidance](https://docs.appimage.org/reference/best-practices.html)
requires building against an old enough base and testing the intended systems.
The portable archive provides a documented, tested path without a FUSE mount;
the two artifacts share one payload and launcher. Keep host graphics drivers
outside that payload. Alpine's musl environment and NixOS need separate runtime
integration and are outside the first compatibility claim.

## Implementation sequence

1. **Run Linux tests.** Add required shells and tools, Linux Clippy and workspace
   tests to CI. Replace macOS-only test expectations with portable expectations
   where behavior is shared. Keep the macOS tests and Windows build passing.
2. **Make the desktop usable.** Add Linux terminal shortcuts such as
   Ctrl+Shift+C/V/T/W, preserving shell Ctrl+C. Make menu actions reachable and
   use Ctrl+click for paths and links. Verify window move, resize, maximize and
   close on both display systems, including compositors without server-side
   decorations. Retain the existing application layout and macOS behavior.
3. **Complete shell and agent workflows.** Resolve foreground cwd through
   `/proc` when OSC 7 is unavailable. Verify bash, zsh and fish, PATH lookup,
   terminal input, worktrees, pane-specific agent status, MCP and restart with
   isolated data. Preserve existing user configuration and conversations.
4. **Produce runnable packages.** Stage one application directory containing
   the executable, necessary runtime libraries, icon and desktop metadata.
   Produce both artifacts and SHA-256 checksums. Confirm agent hooks and
   `chda mcp` invoke a durable executable after relaunch, including the AppImage
   mount path. Upload CI artifacts for validation; do not publish a release.
5. **Complete desktop integrations.** Implement D-Bus notifications with exact
   pane targeting, installed editor/file-manager discovery, About and font/locale
   handling. Replace the macOS-only resource/port inspection assumptions on
   Linux. Missing session services must not block terminal input. Choose and
   document a Linux update path in an ADR before offering an installation
   action; until then show an accurate manual-download action.

Keep OS-specific implementation in the existing platform boundaries. Add an
ADR for packaging, runtime compatibility and the Linux update path. Remove
replaced platform stubs only after their replacements are covered. Split PRs
along the sequence to keep review and failures attributable.

## Acceptance and validation

Use L3 development-environment regression because this adds an execution
platform and packaging. Run the full applicable workspace suite in confirmed
CI. Drive native Linux application windows under X11 and Wayland in isolated
Linux sessions, capture rendered output, and test actual input and persistence.
GPUI's test platform covers scenarios but does not prove native rendering.

Test the same candidate packages on all four distro families; record startup,
missing-library checks, runtime shell input, a temporary repository/worktree,
agent status, exit and reopen. Exercise desktop controls, Unicode, IME,
clipboard, selection, drag/drop, links, shell interruption and folder dialogs.
Validate notification clicks with a session bus and without a notification
service. Include a compatibility-floor system and a current system.

Software-rendered CI sessions and container tests are recorded as such. They
do not establish physical-GPU or desktop-specific IME coverage. Missing runtime
evidence remains outstanding; it is not a passing result. Existing issue #14's
hardware acceptance and its Windows work are not closed by Linux-only CI.
Newly drawn window controls require three separately planned independent UI
verification rounds under the repository's development policy.

Update the feature catalog, generated README and Pages, installation guide,
roadmap and testing documentation with the implemented and actually verified
support matrix. Desktop and narrow-width review use decoded assets and actual
rendered screens.

## Approval and operational boundary

The user approved this design and public release on 2026-10-18. No Linux implementation, new
test run, VM, container, package installation, CI dispatch or public release was
performed for this design. Current-state evidence consists of source inspection,
existing GitHub job readback, release assets and official upstream documentation.
The local Docker server reports Linux ARM64; this does not validate the proposed
x86_64 target. Use isolated development data during the approved implementation.
