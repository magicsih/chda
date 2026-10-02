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

Images from the Kitty graphics protocol are stored and placed by
libghostty-vt; chda decodes PNG payloads for it (`png` crate) and allows the
direct, file, temporary file and shared memory media, with Ghostty's 320 MB
storage limit. Frames carry each visible placement with its viewport cell,
pixel size, source rectangle and layer (below backgrounds, below text, above
text), and share the decoded RGBA pixels by image version. The UI turns each
image version into one texture, draws the layers around backgrounds and text,
and frees textures that left the screen. Cell sizes reach the terminal in
device pixels, as in Ghostty, so images and pixel mouse reports match the
display. Unicode placeholder (virtual) placements are not drawn yet.

Mermaid diagrams are found in the visible rows on the terminal thread
(`chda-term::mermaid`): fenced ```` ```mermaid ```` blocks, and unfenced
blocks that start with a diagram keyword on its own line (`flowchart LR`,
`sequenceDiagram`, ...) and end at a blank line, as agents print them after
rendering Markdown. Frames carry each block's rows and source; the pane
shows a "View diagram" chip on its first row, and cmd-click on the block or
the palette's "View last diagram" (which searches the whole scrollback) open
it. The viewer is an HTML page written to the data directory next to a
bundled `mermaid.min.js` and opened in the default browser, so diagrams never
leave the machine.

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
cache. Sessions are ordered by their transcript's last write (the last
message). Resuming several at once opens one tab and gives each next session
the largest pane, split along its longer side (`Workspace::split_largest`). The UI reaches git and agents only through `chda-core`.

Every pane's shell gets `CHDA_PANE_ID`. Agents inherit it and `chda hook`
sends it back, so status is tracked per pane as well as per worktree: tabs
and the ACTIVE list show the most urgent status of their panes, the Dock badge
counts waiting agents the user has not looked at, and a notification click
(through `UNUserNotificationCenter` in the app bundle) focuses that pane.

Finishing a worktree (`chda-core::cleanup`) merges its branch into the
default branch inside the main worktree, removes the worktree and deletes
the branch; blockers (uncommitted changes, unpushed commits, an open pull
request, the base branch not being checked out) stop it unless the user
forces. Deleting a worktree moves its folder into the repository's
`.git/chda-trash/` (a rename, instant even for a large `node_modules`),
unlocks it and drops git's record of only that worktree, deletes the branch,
then empties the trash in the background; the row shows "deleting" and
refuses a second request meanwhile, terminals open in it are closed, and a
failure is shown under the row. "Update branch" (`chda-core::update`) is offered when a worktree is
clean, has no merge or rebase stopped half way, has an upstream and no agent
works in it; it fetches the upstream, fast-forwards, and on divergence
stops and offers a rebase only when none of the local commits is on any
remote (`git rev-list @{u}..HEAD --not --remotes`), a merge otherwise; the
`pull` key changes that default. Pull request badges come from `gh pr list --repo <host>/<owner>/<name>`
per branch, fetched at most once a minute per repository. The host and path
come from the remote `gh` would pick (`gh repo set-default`, then
`upstream`, `github`, `origin`), with SSH aliases resolved through
`ssh -G` and `repo-hosts` in `config.toml` as an override. `gh auth status
--hostname` is checked per host and cached for five minutes; a host that is
not logged in gets a hint on its repository rows only.

## Config reload and session restore

`chda-core::FileWatcher` watches the directories of the Ghostty config files
(with includes and the theme) and `config.toml`. A change reloads both; a
Ghostty value Ghostty itself would reject, or a `config.toml` that does not
parse, keeps the previous values and shows a message.

Themes resolve like Ghostty's: the theme directories first, then the
iTerm2-Color-Schemes set `chda-config` embeds (`assets/themes.txt`, packed by
`scripts/update-themes.sh`). `theme` in `config.toml` replaces the Ghostty
config's, so picking a theme in chda never edits Ghostty's files.

`chda-core::SavedWindow` is the window's tabs, split tree, ratios, focus,
zoom, tab names and pane directories, written to `session.json` in the data
directory whenever they change and read on the next launch.

A pane also keeps the agent and session id its hooks last reported (through
`CHDA_PANE_ID`); a `SessionEnd` or a new shell prompt (the agent exited)
forgets it. With `restore-agents` on, such a pane comes back running the
adapter's resume command (`claude --resume <id>`, `codex resume <id>`) with
chda's hooks, in the saved directory. When the agent is not on `PATH` or
`AgentAdapter::has_session` finds no transcript, the pane gets a shell and the
restore note says why.

## Environment

`WorkspaceView` gets an `Environment`: the config and data directories,
Ghostty paths, the shell, extra pane environment, the agent adapters and a
`System` for side effects outside the window (notifications, Dock badge,
beep, window restore, the shells' locale). `run()` builds it from the user's
real locations; the scenario tests build it over a temporary home with a
recording `System` (see `docs/testing.md`).

Decisions that are hard to reverse are recorded under `docs/decisions/`.
