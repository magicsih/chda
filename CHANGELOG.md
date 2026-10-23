# Changelog

All notable changes to chda. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); entries are
generated from conventional commit messages by release-plz.

## [1.5.1](https://github.com/magicsih/chda/compare/v1.5.0...v1.5.1) - 2026-10-23

### Fixed

- *(sessions)* recover connections without blocking terminals ([#295](https://github.com/magicsih/chda/pull/295))

## [1.5.0](https://github.com/magicsih/chda/compare/v1.4.0...v1.5.0) - 2026-10-23

### Added

- *(sessions)* select Sessions rows with number shortcuts ([#293](https://github.com/magicsih/chda/pull/293))

### Fixed

- *(window)* reopen closed windows from the Dock ([#291](https://github.com/magicsih/chda/pull/291))

## [1.4.0](https://github.com/magicsih/chda/compare/v1.3.0...v1.4.0) - 2026-10-22

### Added

- *(session)* preserve terminals through macOS app updates ([#288](https://github.com/magicsih/chda/pull/288))
- *(session)* keep macOS terminals alive across GUI restarts ([#287](https://github.com/magicsih/chda/pull/287))
- *(session)* keep terminal processes in an independent service ([#286](https://github.com/magicsih/chda/pull/286))

## [1.3.0](https://github.com/magicsih/chda/compare/v1.2.0...v1.3.0) - 2026-10-21

### Added

- *(agents)* add MiniMax Code and expose Gemini by default

## [1.2.0](https://github.com/magicsih/chda/compare/v1.1.0...v1.2.0) - 2026-10-20

### Added

- *(sidebar)* show a pixel identicon before each Claude Code and Codex session ([#280](https://github.com/magicsih/chda/pull/280))
- *(sidebar)* select and reveal a folder as soon as it is added ([#279](https://github.com/magicsih/chda/pull/279))

### Fixed

- *(terminal)* stop mouse reports typing into the prompt after a program exits ([#278](https://github.com/magicsih/chda/pull/278))

## [1.1.0](https://github.com/magicsih/chda/compare/v1.0.25...v1.1.0) - 2026-10-18

### Added

- *(linux)* offer platform-specific manual update downloads
- *(linux)* add desktop shortcuts and client window controls
- *(linux)* track directories and integrate desktop services

### Fixed

- *(linux)* keep AppImage agent helpers usable after restart
- *(linux)* preserve the visible window size after restoring
- *(palette)* keep command input after window reactivation
- *(palette)* keep commands and keyboard selection inside small windows
- *(linux)* show the full About description in narrow dialogs
- *(linux)* isolate AppImage helper input and process lifetimes
- *(linux)* open About after window updates and restore terminal focus
- *(linux)* match graphics logging targets by string value
- *(linux)* report graphics adapter initialization failures
- *(linux)* leave unsupported hide shortcuts available to terminals
- *(linux)* compile notification routing and preserve Bash directory titles
- *(linux)* keep file reads from repeating configuration and sidebar refreshes
- *(fonts)* render bundled icons on Linux without system fonts
- *(palette)* preserve command order while adding the Linux menu
- *(linux)* identify application windows to the desktop
- *(linux)* respect compositor window control capabilities
- *(linux)* specify window control label types
- *(linux)* keep helper runtime directories private and usable
- *(linux)* run saved AppImage helpers without the GUI environment

### Changed

- *(linux)* satisfy platform Clippy checks

### Other

- Merge branch 'feat/linux-packaging' into test/linux-native-validation
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging
- Merge branch 'feat/linux-platform' into feat/linux-packaging

## [1.0.25](https://github.com/magicsih/chda/compare/v0.1.24...v1.0.25) - 2026-10-16

### Fixed

- *(agents)* keep usage updates and coordination working across windows
- *(agents)* start Codex in chda without the shared background server warning ([#257](https://github.com/magicsih/chda/pull/257))

## [0.1.24](https://github.com/magicsih/chda/compare/v0.1.23...v0.1.24) - 2026-10-15

### Added

- *(theme)* follow the macOS light or dark appearance with a theme pair
- *(terminal)* run user-defined commands for a worktree in a tab, split or floating terminal
- *(terminal)* reorder tabs and move running panes between tabs and windows
- *(mcp)* let agents list sessions, read output and wait for other agents
- *(sidebar)* show child agents as one compact line of icons and counts
- *(terminal)* show OSC 9 and OSC 777 desktop notifications from programs
- *(sidebar)* expand or collapse every repository in PROJECT with one button

### Fixed

- *(terminal)* close a tab when its shell exits and stay on the branch
- *(sidebar)* open a branch with a single click again
- *(sidebar)* let hints come back after a menu and read the bulk hint at build time
- *(sidebar)* keep fold hints, agent badges and child counts readable
- *(session)* reopen a full-screen window at its last windowed size
- *(status-bar)* show Claude usage from any session of the same sign-in
- *(agents)* find agent CLIs when the login shell takes more than two seconds

## [0.1.23](https://github.com/magicsih/chda/compare/v0.1.22...v0.1.23) - 2026-10-13

### Fixed

- *(agents)* Codex 기본 대화를 재개 목록에 표시 ([#245](https://github.com/magicsih/chda/pull/245))
- *(sharing)* 유일한 부분 선택의 들여쓰기와 0.1.23 인수조건을 검증 ([#240](https://github.com/magicsih/chda/pull/240))
- *(paths)* 공백·괄호를 포함한 전체 경로와 UTF-8 diff를 표시 ([#238](https://github.com/magicsih/chda/pull/238))
- *(ui)* 메뉴와 실행 시트의 입력·포커스·Setup 동작을 수정 ([#237](https://github.com/magicsih/chda/pull/237))
- *(sidebar)* 브랜치를 선택한 뒤 더블클릭으로 열고 행과 분할 번호를 유지 ([#236](https://github.com/magicsih/chda/pull/236))
- *(session)* 실제 입출력과 대화 디렉터리를 추적하고 종료 출력을 보존
- *(term)* 스크롤백 크기 변경과 엔진 소스 무결성을 수정

## [0.1.22](https://github.com/magicsih/chda/compare/v0.1.21...v0.1.22) - 2026-10-11

### Added

- *(sidebar)* combine agents and terminals into one Sessions list with independent scrolling ([#203](https://github.com/magicsih/chda/pull/203))

### Fixed

- *(agents)* keep a new Codex conversation working through its title helper ([#223](https://github.com/magicsih/chda/pull/223))
- *(agents)* keep a finished child's state when later polls know less ([#221](https://github.com/magicsih/chda/pull/221))
- *(sidebar)* make the sticky repository header act on its repository and clear closed agents' working status ([#222](https://github.com/magicsih/chda/pull/222))
- *(sidebar)* keep status icons visible in light themes and give + repo its own tooltip ([#215](https://github.com/magicsih/chda/pull/215))
- *(terminal)* wrap long lines in the Copy for sharing preview ([#214](https://github.com/magicsih/chda/pull/214))
- *(terminal)* open paths that follow prose inside parentheses ([#213](https://github.com/magicsih/chda/pull/213))
- *(agents)* keep a live agent's Sessions row when another process reports for its worktree ([#206](https://github.com/magicsih/chda/pull/206))
- *(sidebar)* keep chda running when macOS cannot open the folder picker ([#201](https://github.com/magicsih/chda/pull/201))

## [0.1.21](https://github.com/magicsih/chda/compare/v0.1.20...v0.1.21) - 2026-10-09

### Added

- *(worktrees)* prepare local files and setup before opening new worktrees ([#176](https://github.com/magicsih/chda/pull/176))
- *(review)* keep private notes on worktree diff lines ([#175](https://github.com/magicsih/chda/pull/175))
- *(terminal)* preview selected assistant text before sharing ([#174](https://github.com/magicsih/chda/pull/174))
- *(agents)* show confirmed child activity under each conversation ([#172](https://github.com/magicsih/chda/pull/172))
- *(agents)* restart the exact conversation after a managed process exits ([#171](https://github.com/magicsih/chda/pull/171))
- *(agents)* choose and retain permissions for each conversation ([#170](https://github.com/magicsih/chda/pull/170))
- *(notifications)* keep operational messages in a window history ([#169](https://github.com/magicsih/chda/pull/169))

### Fixed

- *(notifications)* render the title-bar bell with a font fallback ([#179](https://github.com/magicsih/chda/pull/179))
- *(sidebar)* keep repository context while navigating worktrees ([#168](https://github.com/magicsih/chda/pull/168))
- *(terminal)* open space-containing parenthesized paths ([#166](https://github.com/magicsih/chda/pull/166))

## [0.1.20](https://github.com/magicsih/chda/compare/v0.1.19...v0.1.20) - 2026-10-07

### Added

- *(update)* install signed macOS updates while preserving live sessions ([#156](https://github.com/magicsih/chda/pull/156))

## [0.1.19](https://github.com/magicsih/chda/compare/v0.1.18...v0.1.19) - 2026-10-07

### Fixed

- *(terminal)* keep prompt text readable after theme changes ([#147](https://github.com/magicsih/chda/pull/147))

## [0.1.18](https://github.com/magicsih/chda/compare/v0.1.17...v0.1.18) - 2026-10-07

### Fixed

- *(sidebar)* reveal selected branches and collapse agent lists ([#146](https://github.com/magicsih/chda/pull/146))
- *(status-bar)* keep quota reports stable during refreshes ([#144](https://github.com/magicsih/chda/pull/144))

## [0.1.17](https://github.com/magicsih/chda/compare/v0.1.16...v0.1.17) - 2026-10-06

### Fixed

- *(sidebar)* show a spinner for working agents and a badge for waiting ones ([#142](https://github.com/magicsih/chda/pull/142))

## [0.1.16](https://github.com/magicsih/chda/compare/v0.1.15...v0.1.16) - 2026-10-05

### Added

- *(sidebar)* reorder repositories by dragging their headers ([#139](https://github.com/magicsih/chda/pull/139))
- *(sidebar)* star branches above ACTIVE for quick access ([#136](https://github.com/magicsih/chda/pull/136))

### Fixed

- *(sidebar)* keep idle rows, menus and new panes clean ([#138](https://github.com/magicsih/chda/pull/138))

### Changed

- *(workspace)* keep busy background panes from redrawing the window ([#137](https://github.com/magicsih/chda/pull/137))

## [0.1.15](https://github.com/magicsih/chda/compare/v0.1.14...v0.1.15) - 2026-10-05

### Added

- *(workspace)* preserve working context and show agent quota ([#132](https://github.com/magicsih/chda/pull/132))

## [0.1.14](https://github.com/magicsih/chda/compare/v0.1.13...v0.1.14) - 2026-10-05

### Added

- *(sidebar)* show live idle agent sessions separately ([#115](https://github.com/magicsih/chda/pull/115))
- *(tabs)* confirm closing active agent sessions ([#114](https://github.com/magicsih/chda/pull/114))
- *(git)* view commit ancestry in a read-only tab ([#113](https://github.com/magicsih/chda/pull/113))
- *(titlebar)* show the focused branch note ([#110](https://github.com/magicsih/chda/pull/110))

## [0.1.13](https://github.com/magicsih/chda/compare/v0.1.12...v0.1.13) - 2026-10-04

### Fixed

- *(terminal)* forward Escape to running agents ([#105](https://github.com/magicsih/chda/pull/105))

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
