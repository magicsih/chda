# Changelog

All notable changes to chda. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); entries are
generated from conventional commit messages by release-plz.

## [0.1.12](https://github.com/magicsih/chda/compare/v0.1.11...v0.1.12) - 2026-10-04

### Added

- *(sidebar)* keep active tabs stable with activity ages and branch labels ([#101](https://github.com/magicsih/chda/pull/101))

### Fixed

- *(agents)* find installed agents using the login shell PATH ([#103](https://github.com/magicsih/chda/pull/103))
- *(agents)* track Codex started from a terminal prompt ([#99](https://github.com/magicsih/chda/pull/99))

## [0.1.11](https://github.com/magicsih/chda/compare/v0.1.10...v0.1.11) - 2026-10-03

### Added

- *(ui)* resize the sidebar by dragging its border and collapse it from the title bar ([#98](https://github.com/magicsih/chda/pull/98))
- *(sidebar)* add folders that are not git repositories, with an offer to run git init ([#96](https://github.com/magicsih/chda/pull/96))

## [0.1.10](https://github.com/magicsih/chda/compare/v0.1.9...v0.1.10) - 2026-10-03

### Added

- tell when a newer chda release is out ([#91](https://github.com/magicsih/chda/pull/91))
- run on Intel Macs with a universal macOS app ([#90](https://github.com/magicsih/chda/pull/90))
- *(sidebar)* show each agent session's token usage and its API-equivalent cost ([#88](https://github.com/magicsih/chda/pull/88))

## [0.1.9](https://github.com/magicsih/chda/compare/v0.1.8...v0.1.9) - 2026-10-03

### Added

- *(ui)* open the focused worktree in Finder, VS Code or another app from the title bar ([#82](https://github.com/magicsih/chda/pull/82))

### Fixed

- *(term)* show GitHub alert titles and tidy footnotes in Markdown previews ([#78](https://github.com/magicsih/chda/pull/78))

## [0.1.8](https://github.com/magicsih/chda/compare/v0.1.7...v0.1.8) - 2026-10-03

### Added

- *(term)* preview Markdown files in the browser from a right-clicked path or the palette ([#77](https://github.com/magicsih/chda/pull/77))

## [0.1.7](https://github.com/magicsih/chda/compare/v0.1.6...v0.1.7) - 2026-10-03

### Added

- *(agents)* let agents create worktrees, open tabs and report status through `chda mcp` ([#68](https://github.com/magicsih/chda/pull/68))
- *(sidebar)* add notes to worktree branches and show them in the sidebar ([#62](https://github.com/magicsih/chda/pull/62))
- *(sidebar)* show merged, missing folder, open tabs and deleting as icons ([#72](https://github.com/magicsih/chda/pull/72))
- *(agents)* add named agent launch presets ([#66](https://github.com/magicsih/chda/pull/66))
- *(ui)* drop files on a pane to paste their paths, and folders on the sidebar to add repositories ([#67](https://github.com/magicsih/chda/pull/67))
- *(agents)* track Gemini CLI, GitHub Copilot CLI and OpenCode like Claude Code ([#60](https://github.com/magicsih/chda/pull/60))
- *(term)* right-click file and folder paths to open, reveal, cd or copy them ([#56](https://github.com/magicsih/chda/pull/56))
- *(sidebar)* show each worktree's diff size and open its diff in a tab ([#65](https://github.com/magicsih/chda/pull/65))
- *(sidebar)* show merge request and pull request badges for GitLab and Gitea ([#64](https://github.com/magicsih/chda/pull/64))
- *(sidebar)* create a worktree from any branch, with a random name when left empty ([#55](https://github.com/magicsih/chda/pull/55))
- *(sidebar)* update a worktree's branch from its upstream when it is safe ([#63](https://github.com/magicsih/chda/pull/63))
- *(sidebar)* show pull request badges for every GitHub host a repository uses ([#54](https://github.com/magicsih/chda/pull/54))
- *(sidebar)* order sessions by last message and resume several side by side ([#57](https://github.com/magicsih/chda/pull/57))
- *(sidebar)* explain every sidebar badge in a tooltip ([#50](https://github.com/magicsih/chda/pull/50))
- *(term)* open Mermaid diagrams from the output in an offline viewer ([#61](https://github.com/magicsih/chda/pull/61))
- *(term)* show images sent with the Kitty graphics protocol ([#59](https://github.com/magicsih/chda/pull/59))
- *(term)* turn on font ligatures with Ghostty's font-feature setting ([#52](https://github.com/magicsih/chda/pull/52))
- *(agents)* reopen agent conversations when restoring the last session ([#53](https://github.com/magicsih/chda/pull/53))

### Fixed

- *(sidebar)* stop the window crashing when the mouse moves over a past session ([#73](https://github.com/magicsih/chda/pull/73))
- *(sidebar)* delete a worktree with one confirmation, even when it is locked or large ([#51](https://github.com/magicsih/chda/pull/51))
- *(term)* keep unfenced Mermaid diagrams whole across blank lines ([#71](https://github.com/magicsih/chda/pull/71))

## [0.1.6](https://github.com/magicsih/chda/compare/v0.1.5...v0.1.6) - 2026-10-02

### Added

- *(ui)* add a macOS menu bar with File, Edit, View, Agents, Window and Help ([#45](https://github.com/magicsih/chda/pull/45))
- *(palette)* pick a theme with a live preview from 650+ bundled themes ([#45](https://github.com/magicsih/chda/pull/45))
- *(term)* bundle JetBrains Mono and Nerd Font icons, and size cells like Ghostty ([#45](https://github.com/magicsih/chda/pull/45))
- *(sidebar)* show worktrees whose folder is missing and prune them

### Fixed

- *(config)* read Ghostty's config.ghostty files, not only the legacy config ([#48](https://github.com/magicsih/chda/pull/48))
- *(platform)* give the macOS app icon a transparent background ([#49](https://github.com/magicsih/chda/pull/49))
- *(palette)* move the selection with the arrow keys ([#45](https://github.com/magicsih/chda/pull/45))
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
