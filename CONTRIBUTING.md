# Contributing

## Prerequisites

- Rust 1.98.1 (pinned in `rust-toolchain.toml`; rustup installs it on first build).
- Zig 0.16.0, exactly, on `PATH`. The terminal engine uses the commits and checked patch recorded in `resources/ghostty/source.json`. Keep your global toolchain if other projects need it; a task-local Zig installation works.
- git on `PATH` (the Ghostty fetch uses it).
- macOS: Xcode with the Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`). CI retains its validated Xcode SDK while the engine moves to Zig 0.16.0.
- Linux: `libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libfontconfig-dev libvulkan1 libasound2-dev` (see `.github/workflows/ci.yml`).
- Windows: Visual Studio Build Tools with the Windows SDK.

The first build fetches the Zed repository (for GPUI) and preparation downloads the pinned Ghostty archive, so expect it to take a while.

## Workflow

- Branch from `main`, open a pull request. Commit messages and PR text in English, in Conventional Commits form (`fix: ...`, `feat(sidebar): ...`); `scripts/check-commits.sh` runs in CI.
- CI must be green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, tests on macOS, release builds on macOS, Linux and Windows.
- Crate dependency direction is enforced by `deny.toml`. See `docs/architecture.md` before adding a dependency between crates.
- Architecture changes go through an ADR under `docs/decisions/`.

## Running

```sh
python3 scripts/prepare-ghostty.py
cargo run -p chda
```

Shortcuts follow Ghostty's macOS defaults: `cmd-t` new tab, `cmd-w` close,
`cmd-1`..`cmd-9` and `cmd-shift-[`/`]` switch tabs, `cmd-d` / `cmd-shift-d`
split right / down, `cmd-alt-arrows` move between splits, `cmd-ctrl-arrows`
resize, `cmd-ctrl-=` equalize, `cmd-shift-enter` zoom a split, `cmd-up` / `cmd-down`
jump between shell prompts, `cmd-b` toggle the sidebar, `cmd-shift-o` add a repository,
`cmd-n` new worktree, `cmd-shift-p` command palette. Double-click a tab to rename it.

## Packaging (macOS)

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/bundle-macos.sh 0.1.0
```

builds `target/bundle/chda.app` as a universal app (Apple silicon and Intel)
and a zip. Without `CHDA_SIGN_IDENTITY` the
app is ad-hoc signed; with it plus `APPLE_ID`, `APPLE_TEAM_ID` and
`APPLE_APP_PASSWORD` it is signed with hardened runtime and notarized. The
`Release` workflow uses the same existing identities for an explicitly
dispatched, evidence-gated build. See [the release procedure](docs/releasing.md).

## Releases

Releases are cut by [release-plz](https://release-plz.dev): every push to
`main` updates a pull request titled `chore: release vX.Y.Z` that bumps the
workspace version and writes the `CHANGELOG.md` section from `feat` and `fix`
commits since the last tag. Merging it creates the `vX.Y.Z` tag, and the
`Release` workflow registers a candidate. Separate `build` and `publish`
dispatches verify native evidence, preserve the signed draft artifact and
publish the same ZIP before updating the cask.
Both workflows need the `RELEASE_TOKEN` secret, a fine-grained personal
access token with contents and pull requests read/write on this repository
and contents read/write on the tap.

## Pinned terminal source

Run `python3 scripts/prepare-ghostty.py` before Cargo. It verifies the archive,
patch and entire source tree, stages preparation atomically, and preserves an
existing source on a mismatch. `--check` verifies without downloading. For an
offline source preparation, pass `--archive` with the same pinned archive.
`.cargo/config.toml` forces `GHOSTTY_SOURCE_DIR` to `.ghostty-source/<source hash>`;
unprepared or changed source fails the build rather than using another engine.
The hash changes with the patch, so Cargo rebuilds the native library instead
of reusing a previous engine. `--test` runs the pinned Debug resize, row-metadata and Kitty boundary regressions.
The source stays outside Cargo's target cache, which may prune non-Cargo
files or restore a partially pruned tree. Git attributes and the apply command
preserve LF bytes even when Git enables Windows autocrlf.
Only Zig's `.zig-cache`, `zig-out` and `zig-pkg` generated directories are excluded
from the source digest. Package dependencies stay pinned by `build.zig.zon`.

The engine patch normalizes a newly active viewport before growth while
retaining genuine history anchors and Debug integrity assertions. Kitty temporary
images are restricted to the process directory after validating the opened file
handle. Remove each corresponding patch when a pinned upstream revision includes the equivalent behavior and passes the
same regressions; do not change optimize mode to avoid a failure.

## Terminal path lookup

OSC 8, URLs and quoted/prose-parenthesized paths retain precedence. Explicit
unquoted paths (`/`, `~/`, `./`, `../`) then use the longest existing candidate
across spaces and filename parentheses on the same logical line. Soft wraps join;
hard newlines, quotes, tabs, `<`, `>` and `|`, and adjacent explicit path starts
separate candidates. New bare-path lookup performs at most 16 filesystem checks.
After exhausting that bound it does not fall back to a cut legacy prefix. Results
are shared for the frame, working directory and segment. Activating a displayed
path checks the same target still exists rather than replacing it.

Bare-path exploration checks the longest candidates first, up to 16 filesystem lookups per request. The bound includes trailing prose on the same logical line; when that would require more candidates, chda leaves the path unlinked instead of guessing a shorter prefix. Existing OSC 8, URL, quoted and prose-parenthesized paths retain precedence. Unsuccessful lookups are not cached; successful lookups are cached only over their resolved cells and revalidated before an action.
