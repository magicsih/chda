# Agent instructions

Guidance for any AI coding agent (Claude Code, Codex, OpenCode, Copilot, ...) working in this repository.

## Project

chda is a terminal emulator with a git worktree side panel and a live status board for LLM coding agents. Rust application, terminal core via `libghostty-vt`, UI on GPUI. macOS first; Linux and Windows must keep building in CI.

Architecture decisions are summarized in `docs/` once code lands. Do not re-litigate them in code; propose changes as an ADR under `docs/decisions/`.

## Crate layout and dependency rules

Workspace crates: `chda` (binary), `chda-ui`, `chda-core`, `chda-term`, `chda-pty`, `chda-git`, `chda-agents`, `chda-config`.

- Only `chda-ui` may depend on GPUI.
- Only `chda-term` may depend on `libghostty-vt`. It exposes plain Rust types (`CellGrid`, events); no FFI types leak out.
- `chda-core` is the domain hub and knows nothing about GPUI, libghostty, or gix.
- Platform-specific code (`#[cfg(target_os = ...)]`) lives only in `chda-pty`, `chda-ui/platform`, and `chda-agents/ipc`.

## Conventions

- Rust stable, `cargo fmt`, `cargo clippy -D warnings`, tests pass on macOS; build passes on Linux and Windows.
- Branch + pull request for every change. Commit messages and PR text in English.
- Commit subjects and PR titles follow Conventional Commits: `<type>(<scope>)?: <description>`, types `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert`; `!` before the colon marks a breaking change. CI rejects anything else. release-plz turns `feat` and `fix` commits into the changelog and the next version, so write the description as a user-facing sentence: `fix(sidebar): detect squash-merged branches`.
- Do not edit `CHANGELOG.md` or the workspace version by hand; the release pull request does that.
- Do not add AI attribution, signatures, or marketing lines to code, commits, or PRs.
- No secrets, tokens, or user-specific paths in the repository.
- Remove dead code and compatibility shims when you replace them.
- A bug fix comes with a test that fails without the fix; user-facing flows get a scenario test. See `docs/testing.md`.

## Local overrides

Agents may keep private notes in `CLAUDE.local.md`, `AGENTS.local.md`, or `.agents/local/`. These are gitignored.
