# Verification record

Runtime implementation: `829bfa04d7d68d96df6f9f8537951c1cc5927779`.
Baseline: `8512a802c5c3eaffba1c2c1179c8aa58646ec3a3`.
Final native round outcomes, CI and merge status are recorded in
[PR #304](https://github.com/magicsih/chda/pull/304).

An independent macOS run using Rust 1.98.1 and pinned Zig 0.16.0 passed
`cargo test --workspace -- --test-threads=4`: 538 passed, 0 failed, 5 ignored.
The shell's file-descriptor limit was raised to 4096 for the multi-window
scenarios. `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings` passed. Product docs
have seven checks, and Linux rendered-readiness helper tests have three.

Native reviews use disposable macOS apps with private HOME, XDG and provider
directories and synthetic provider metadata. They do not modify the installed
user app or make paid provider requests. The staged binary SHA-256 is
`59dfd1464be2e37d9633bfa76ba4b2a9d5d64574046b19c9daba4088dd2fad23`.
When the UI tool stopped finding that private bundle's window, a second bundle
identity was tested with an ad-hoc signature. Its binary SHA-256 is
`8b90adb1430fde9e9ca06896c8d773e37fdf7a3143e87bd672120333a8fb085c`;
the original and second bundle's Mach-O `__TEXT,__text` listings have identical
SHA-256 `88e93293363d51484620ae9bf2cfbc95c475dc1d2677027b0ae30b65a16838eb`.
The runtime source is unchanged by the following documentation and test-helper
changes. Fixture paths were canonicalized to match native shell directories.

The original Linux startup check missed a clearly rendered shell prompt:
Tesseract's sparse full-frame pass returned no words. The correction reads an
unscaled crop of the current terminal background and still requires the exact
normalized marker. Original PNGs, raw OCR, crop bounds and source hashes remain
available. An independent replay of the failing capture confirmed both the
empty original result and successful crop OCR; this replay is a helper check,
not a new native product run. CI executes the actual packages again.

The feature catalog generates README and Pages. The native section crop is
kept at its intrinsic width and was independently rendered at 320, 768 and
1280px without horizontal overflow. Source and export provenance is in
`docs/media/README.md`.

Retained artifact-store key:
`chda/2026-10-25/829bfa04d7d68d96df6f9f8537951c1cc5927779/sidebar-sections-304`.
The final manifest records source and destination sizes and SHA-256 values.
It includes independent test logs, three native review records, original
screenshots, fixture provenance and the initial CI failure evidence. A partial
or unavailable UI check is kept as such, never promoted to a pass. No release,
tag, hand-edited version or installed-app replacement is part of this PR.
