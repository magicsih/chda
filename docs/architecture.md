# Architecture

chda is a Cargo workspace of eight crates. Arrows only point downward; the
direction is enforced by `deny.toml` (`cargo deny check bans`).

| Crate | Role | May depend on |
|---|---|---|
| `chda` (bin) | Entry point. GUI, and CLI subcommands such as `chda hook` | `chda-ui`, `chda-core`, `chda-agents`, `chda-config` |
| `chda-ui` | GPUI views, elements, theme. The only crate that knows GPUI | `chda-core`, `chda-term`, `chda-config`, `gpui` |
| `chda-core` | Domain hub: workspace, worktrees, agents, event bus | `chda-term`, `chda-git`, `chda-agents`, `chda-config` |
| `chda-term` | `libghostty-vt` wrapper. PTY bytes in, plain-struct frames out | `chda-pty`, `libghostty-vt` |
| `chda-pty` | PTY create/read/write/resize over `portable-pty` | `portable-pty` |
| `chda-git` | `GitBackend` trait; gix for reads, `git` subprocess for writes | `gix` |
| `chda-agents` | `AgentAdapter` trait, hook receiver, session indexer | — |
| `chda-config` | Ghostty config subset and chda's own TOML config | — |

Rules:

- Only `chda-ui` depends on GPUI. Only `chda-term` depends on `libghostty-vt`;
  no FFI type leaves that crate. `chda-core` knows nothing about GPUI,
  libghostty or gix.
- `#[cfg(target_os = ...)]` is allowed only in `chda-pty`, `chda-ui/platform`
  and `chda-agents/ipc`.
- GPUI is a git dependency pinned to a Zed commit in the workspace
  `Cargo.toml`. It is bumped once per milestone in its own pull request.

## Terminal data flow

`libghostty-vt` handles are `!Send`, so each session runs two threads:

- a reader thread that blocks on the PTY and forwards byte chunks;
- a terminal thread that owns the VT state, parses output, encodes input
  (keys, paste, focus, resize, scroll) and writes to the PTY.

The terminal thread publishes `Arc<Frame>` snapshots (plain structs: cells,
rows, cursor, colors, scrollbar, title, pwd) at most every 8 ms while output
streams, and sends `Event`s (frame, title, pwd, bell, clipboard, exit) over a
channel plus a wake callback. The UI never touches the VT state; it reads the
latest frame when it paints.

Scrollback search runs on the terminal thread: the whole screen is formatted
as plain text, one line per row, and matched with the `regex` crate; frames
carry the highlights for the visible rows. A tracked grid reference keeps the
current match in place while new output re-runs the search (at most four
times a second). Frames also carry OSC 8 hyperlinks; plain URLs and file
paths are found in the visible rows, joined across soft wraps, when the user
holds `cmd`.

## Workspace model

`chda-core` holds the window model: tabs, each with a binary split tree of
pane ids, the focused pane, and per-pane info (title, cwd, bell). It does
geometry (layout in the unit square, directional focus, resize, zoom) but
knows nothing about views. `chda-ui`'s `WorkspaceView` maps pane ids to
`TerminalView` entities, renders the tab bar and split tree, and turns
terminal events (exit, title, cwd, bell, focus) into model updates.

## Shell integration

`chda-term` ships its own scripts for zsh, bash and fish
(`shell-integration/`) that emit OSC 133 prompt marks, OSC 7
working-directory reports and an OSC 2 title. They are written to the
per-user data directory on first use, and `launch_for` returns the arguments
and environment that make the shell load them without touching the user's
dotfiles:

- zsh: `ZDOTDIR` points at chda's `.zshenv`, which restores the user's
  `ZDOTDIR` and sources their files.
- bash: `bash --rcfile <script>`. bash ignores `--rcfile` in login shells,
  and the macOS bash 3.2 does not honor `ENV` with `--posix`, so the script
  reads the files a login bash would (`/etc/profile`, then the first of
  `~/.bash_profile`, `~/.bash_login`, `~/.profile`) before installing its
  `PROMPT_COMMAND` and `DEBUG` trap hooks, keeping the user's own.
- fish: chda's data directory goes first on `XDG_DATA_DIRS`, so fish sources
  `fish/vendor_conf.d/chda-integration.fish`, which restores the variable.
  fish 4 writes OSC 133 and OSC 7 itself, so the script only adds them for
  fish 3 (or with the `mark-prompt` feature off); the title stays with
  `fish_title`.

Because the bash launch changes the command line, it applies only when chda
starts the login shell itself, not to agent commands. Shells without
integration fall back to asking the OS for the foreground process's
directory (`chda-pty`, macOS only so far).

## Sidebar and agents

`chda-core::Sidebar` holds registered repositories, their worktrees with git
badges, per-agent status (idle, working, waiting for input, review) and the
sessions found in agent transcripts. `chda-git` reads with gix and writes
with the `git` binary. `chda-agents` knows each agent: how to launch it with
hooks (Claude Code gets a per-launch `--settings` file; Codex gets a
`-c notify=[...]` override that chains the user's own notify program), where
its transcripts live and how to parse them. Agents report through
`chda hook <agent>`, which forwards the event over a Unix socket in the data
directory, or appends to `events.jsonl` when no app is running. The UI
refreshes badges every 5 s while the sidebar is visible, one refresh per
repository at a time, and re-indexes sessions every 30 s with an mtime/size
cache. The UI reaches git and agents only through `chda-core`.

Every pane's shell gets `CHDA_PANE_ID`. Agents inherit it and `chda hook`
sends it back, so status is tracked per pane as well as per worktree: tabs
and the ACTIVE list show the most urgent status of their panes, the Dock badge
counts waiting agents the user has not looked at, and a notification click
(through `UNUserNotificationCenter` in the app bundle) focuses that pane.

Finishing a worktree (`chda-core::cleanup`) merges its branch into the
default branch inside the main worktree, removes the worktree and deletes
the branch; blockers (uncommitted changes, unpushed commits, an open pull
request, the base branch not being checked out) stop it unless the user
forces. Pull request badges come from `gh pr list` per branch, fetched at
most once a minute per repository.

## Config reload and session restore

`chda-core::FileWatcher` watches the directories of the Ghostty config files
(with includes and the theme) and `config.toml`. A change reloads both; a
Ghostty value Ghostty itself would reject, or a `config.toml` that does not
parse, keeps the previous values and shows a message.

`chda-core::SavedWindow` is the window's tabs, split tree, ratios, focus,
zoom, tab names and pane directories, written to `session.json` in the data
directory whenever they change and read on the next launch.

## Environment

`WorkspaceView` gets an `Environment`: the config and data directories,
Ghostty paths, the shell, extra pane environment, the agent adapters and a
`System` for side effects outside the window (notifications, Dock badge,
beep, window restore, the shells' locale). `run()` builds it from the user's
real locations; the scenario tests build it over a temporary home with a
recording `System` (see `docs/testing.md`).

Decisions that are hard to reverse are recorded under `docs/decisions/`.
