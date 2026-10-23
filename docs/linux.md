# Linux

Linux packages target x86_64, glibc 2.31 or newer, and a compatible Vulkan graphics driver.
X11 and Wayland use the same executable. The host supplies its graphics loader
and drivers; they are not replaced by the package.

## Download and verify

Download the latest Linux x86_64 AppImage or portable archive and `SHA256SUMS`
from [GitHub Releases](https://github.com/magicsih/chda/releases). `VERSION` below
means the version in the downloaded filename. Verify the files you downloaded:

```sh
sha256sum --check --ignore-missing SHA256SUMS
```

The release also provides the unmodified distribution sources for bundled
shared libraries. The application directory includes their copyright notices.

## AppImage

```sh
chmod +x chda-VERSION-linux-x86_64.AppImage
./chda-VERSION-linux-x86_64.AppImage
```

If the system cannot mount an AppImage through FUSE, use its extract-and-run
mode or the portable archive:

```sh
APPIMAGE_EXTRACT_AND_RUN=1 ./chda-VERSION-linux-x86_64.AppImage
```

## Portable archive

```sh
tar -xzf chda-VERSION-linux-x86_64.tar.gz
./chda-VERSION-linux-x86_64/AppRun
```

Keep the complete extracted directory together. `AppRun` finds its executable
and libraries relative to itself, so it also works through a symlink. It does
not change `LD_LIBRARY_PATH` for your shell or editors. A symlink named `chda`
on your PATH can point at the persistent `AppRun` path for `chda mcp` and CLI
commands. Desktop metadata and an icon are included; desktop integration is
optional and must reference the installed, persistent path.

`git` must be on PATH. AppImage agent helpers also require util-linux `setsid`,
which is supplied by the supported distributions. Install the coding agents you use and sign in through
their own CLIs. A session D-Bus service provides notifications and folder
portals; missing services leave terminal input available and report the failed
operation. Some font characters need a font installed on the host. The
terminal's JetBrains Mono and Nerd Font symbols are bundled.

On X11, an unpatched IBus 1.5.32 XIM bridge can crash after a few keystrokes
and lose input. This also affects other XIM applications. Use your distribution's
IBus update containing [the upstream fix](https://github.com/ibus/ibus/issues/2789),
or use a working input server. Native Wayland input uses a separate protocol;
the verified Fcitx5 session is listed below. The failed Debian 13 IBus 1.5.32-2
session is not counted as an IME pass.

Stock Debian 11 Mesa 20 software rendering failed: neither Vulkan nor EGL exposed
a compatible X11 surface format to the application. glibc 2.31 by itself is not
a graphics compatibility guarantee. This environment is retained as a negative
check and is not counted as a supported startup.

Stock Ubuntu 20.04 Mesa 21.2 software rendering also failed. Its Vulkan device
reported fewer color attachments than the graphics engine requests. A successful
Vulkan sample on that driver does not establish application compatibility.
Both package formats rendered the terminal and accepted input in the Ubuntu
20.04 compatibility-floor development check. That environment retains glibc
2.31 and uses a separate Mesa 25.2.8 Vulkan driver with LLVM 20.1.8. The driver
is not part of either download. Final release-candidate verification remains
listed separately below.

## Shortcuts

| Action | Linux shortcut |
| --- | --- |
| Copy, paste | Ctrl+Shift+C, Ctrl+Shift+V |
| New tab, close pane | Ctrl+Shift+T, Ctrl+Shift+W |
| Split right, split down | Ctrl+Shift+D, Ctrl+Shift+E |
| Commands, add folder | Ctrl+Shift+P, Ctrl+Shift+O |
| Find | Ctrl+Shift+F |
| Change tab within the repository | Ctrl+PageUp, Ctrl+PageDown |
| Select Sessions rows 1 through 10 | Alt+1 through Alt+9, Alt+0 |
| Reset font size | Ctrl+0 |
| Focus a split | Ctrl+Shift+arrow |
| Follow a link or file path | Ctrl+click |

Ctrl+C still interrupts the foreground program. The Commands button exposes
settings, window actions, About and help links without a system menu bar.
Client window controls appear when the compositor does not provide a title bar.

## Updates and saved data

The Linux update notice opens the matching release downloads. Verify the new
package, close chda, then replace the AppImage or extracted directory. Opening
the download page keeps running shells alive; Linux does not use the macOS
session-preserving installer. Keep the old package until its running windows
and child processes have exited.

Configuration remains in `$XDG_CONFIG_HOME/chda/config.toml` (normally
`~/.config/chda/config.toml`); saved sessions and hooks remain under
`$XDG_DATA_HOME/chda` (normally `~/.local/share/chda`). Ghostty config is read
from `$XDG_CONFIG_HOME/ghostty`. Package replacement does not remove these files. AppImage hooks and MCP use
private launchers under `$XDG_DATA_HOME/chda/helpers` that reference the original
image. Keep that image at its installed path while using its saved agent commands.
Each helper uses its own temporary extraction directory so concurrent requests
do not remove the running application's files.

## Verification matrix

Development startup checks on 18 October 2026 verified the rendered shell
prompt, native typed input and rendered command output for both the AppImage
in extract-and-run mode and the portable archive in these x86_64 environments:

| Runtime userland | glibc | Vulkan software driver |
| --- | --- | --- |
| Ubuntu 24.04.5 LTS | 2.39 | Mesa 25.2.8, LLVM 20.1.2 |
| Debian 13 (trixie) | 2.41 | Mesa 25.0.7, LLVM 19.1.7 |
| Fedora 44 | 2.43 | Mesa 26.2.4, LLVM 22.1.8 |
| Arch Linux | 2.44 | Mesa 26.2.5, LLVM 23.1.3 |
| Ubuntu 20.04.6 LTS with a QA driver | 2.31 | Mesa 25.2.8, LLVM 20.1.8 |

All rows use llvmpipe under X11, Xvfb and Openbox. The Ubuntu 20.04 row retains
its libc and adds a separate compatible test-host graphics driver; it does
not establish support for that distribution's stock Mesa driver. No QA driver
is bundled in the downloads.

The full desktop journeys separately exercise Ubuntu 24.04 X11 with IBus and
Debian 13 native Wayland with Mutter client window controls. The additional
labwc and Fcitx5 Hangul session verifies native preedit and commit. These are
isolated software-rendered sessions, not coverage of physical GPUs, every
desktop or every input method. Startup checks do not establish a complete
desktop journey on every distribution or FUSE-mounted AppImage operation.

Final release review uses the exact main commit and existing package digests
for all three independent rounds, without rebuilding between rounds. The
release's `linux-build.json` and `SHA256SUMS` identify its source and downloads;
the [native review workflow](../.github/workflows/linux-native-review.yml)
records the separate execution evidence. ARM64, Alpine/musl, NixOS and Windows
packages are outside the first delivery.
