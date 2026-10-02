# Changelog

All notable changes to chda. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); entries are
generated from conventional commit messages by release-plz.

## [0.1.6](https://github.com/magicsih/chda/compare/v0.1.5...v0.1.6) - 2026-10-02

### Added

- *(ui)* 메뉴 막대, 테마 선택기, 내장 글꼴 ([#45](https://github.com/magicsih/chda/pull/45))
- *(sidebar)* show worktrees whose folder is missing and prune them

### Fixed

- *(config)* read Ghostty's config.ghostty files, not only the legacy config ([#48](https://github.com/magicsih/chda/pull/48))
- *(ui)* start panes in the configured shell, and allow GPUI's test deps

### Changed

- *(agents)* show agent session lists right after launch

## [0.1.5](https://github.com/magicsih/chda/compare/v0.1.4...v0.1.5) - 2026-10-02

### Fixed

- *(term)* show IME composition in apps that hide the cursor
- *(platform)* start shells in a UTF-8 locale when launched from the Dock

### Changed

- *(term)* show typed characters without waiting for the frame throttle

## [0.1.4](https://github.com/magicsih/chda/compare/v0.1.3...v0.1.4) - 2026-10-01

### Added

- *(config)* apply Ghostty and config.toml changes without a restart
- *(ui)* restore windows, tabs, splits and directories on launch
- *(agents)* show agent status on tabs, in the activity list and on the Dock
- *(term)* add shell integration for bash and fish
- *(term)* open links with cmd-click
- *(term)* search the scrollback with cmd-f
- *(ui)* change the font size at runtime with cmd-=, cmd-- and cmd-0

## [0.1.3](https://github.com/magicsih/chda/compare/v0.1.2...v0.1.3) - 2026-10-01

### Other

- Detect squash and rebase merges and compare against origin for the merged marker
- Add issue and PR templates, security policy, changelog, Pages workflow and onboarding copy

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

[0.1.2]: https://github.com/magicsih/chda/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/magicsih/chda/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/magicsih/chda/releases/tag/v0.1.0
