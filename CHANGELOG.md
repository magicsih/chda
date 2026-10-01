# Changelog

All notable changes to chda. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.1.2] - 2026-10-01

### Added
- Tabs are grouped by repository: the tab bar shows the current repository's
  tabs, and the sidebar's ACTIVE list shows every tab by last activity.
- Automatic tab titles use the worktree branch (`tab-title = "branch"`) or
  the directory name (`"path"`).
- "New worktree from branch..." on a repository picks an existing local branch.
- "Delete worktree..." explains whether deletion is safe (merged, clean) or
  must be forced, and removes the branch with the worktree.
- A green `merged` marker on worktrees that are safe to delete.

### Changed
- The pane-count badge reads "n tabs".

## [0.1.1] - 2026-10-01

### Added
- Rename a tab by double-clicking it (or "Rename tab" in the palette).
- The sidebar status message has a close button.

### Fixed
- Status dots now sit on the branch name's baseline.

## [0.1.0] - 2026-10-01

First release. macOS, Apple silicon.

- Terminal on libghostty-vt and GPUI: colors, wide glyphs, keyboard and IME
  input, mouse selection and reporting, scrollback, Ghostty config for font,
  colors and padding.
- Tabs and splits with Ghostty's default shortcuts; zsh shell integration.
- Worktree sidebar with git badges, create/merge/delete, agent status from
  Claude Code and Codex hooks, session resume, pull request badges via `gh`,
  command palette.
- Signed and notarized app bundle, Homebrew cask.

[Unreleased]: https://github.com/magicsih/chda/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/magicsih/chda/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/magicsih/chda/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/magicsih/chda/releases/tag/v0.1.0
