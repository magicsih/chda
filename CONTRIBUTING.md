# Contributing

## Prerequisites

- Rust 1.98.1 (pinned in `rust-toolchain.toml`; rustup installs it on first build).
- Zig 0.15.2, exactly. `libghostty-vt-sys` clones Ghostty at a pinned commit and runs `zig build`; Ghostty rejects other Zig minor versions. Install with `mise install zig@0.15.2` or `brew install zig@0.15` and put it on `PATH`.
- git on `PATH` (the Ghostty fetch uses it).
- macOS: Xcode with the Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`). Zig 0.15.2 fails to link against the macOS 26.4 and 26.5 SDKs (`undefined symbol: __availability_version_check`, [ziglang/zig#31658](https://codeberg.org/ziglang/zig/issues/31658)); if you hit that, point `DEVELOPER_DIR` at an Xcode 26.3 or older install, or at Xcode 27, which works again.
- Linux: `libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libfontconfig-dev libvulkan1 libasound2-dev` (see `.github/workflows/ci.yml`).
- Windows: Visual Studio Build Tools with the Windows SDK.

The first build fetches the Zed repository (for GPUI) and the Ghostty repository, so expect it to take a while.

## Workflow

- Branch from `main`, open a pull request. Commit messages and PR text in English, in Conventional Commits form (`fix: ...`, `feat(sidebar): ...`); `scripts/check-commits.sh` runs in CI.
- CI must be green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, tests on macOS, release builds on macOS, Linux and Windows.
- Crate dependency direction is enforced by `deny.toml`. See `docs/architecture.md` before adding a dependency between crates.
- Architecture changes go through an ADR under `docs/decisions/`.

## Running

```sh
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
`Release` workflow does the same for `v*` tags using repository secrets and
attaches the zip to a GitHub release, then fills `packaging/homebrew/chda.rb`
and pushes it to `magicsih/homebrew-tap`.

## Releases

Releases are cut by [release-plz](https://release-plz.dev): every push to
`main` updates a pull request titled `chore: release vX.Y.Z` that bumps the
workspace version and writes the `CHANGELOG.md` section from `feat` and `fix`
commits since the last tag. Merging it creates the `vX.Y.Z` tag, and the
`Release` workflow builds, signs, notarizes, publishes and updates the cask.
Both workflows need the `RELEASE_TOKEN` secret, a fine-grained personal
access token with contents and pull requests read/write on this repository
and contents read/write on the tap.
