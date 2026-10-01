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

- Branch from `main`, open a pull request. Commit messages and PR text in English.
- CI must be green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, tests on macOS, release builds on macOS, Linux and Windows.
- Crate dependency direction is enforced by `deny.toml`. See `docs/architecture.md` before adding a dependency between crates.
- Architecture changes go through an ADR under `docs/decisions/`.

## Running

```sh
cargo run -p chda
```
