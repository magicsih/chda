# Architecture

chda is a Cargo workspace of eight crates. Arrows only point downward; the
direction is enforced by `deny.toml` (`cargo deny check bans`).

| Crate | Role | May depend on |
|---|---|---|
| `chda` (bin) | Entry point. GUI, and CLI subcommands such as `chda hook` and `chda note` | `chda-ui`, `chda-core`, `chda-agents`, `chda-config` |
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
holds `cmd` or right-clicks. Paths can be quoted or backslash-escaped to
contain spaces, and `file://` URLs count as paths. Since `ls` prints bare
names, any word is a path candidate: the UI resolves candidates against the
pane's directory (OSC 7) and keeps the first that exists, so the underline
(solid for files, dotted for folders) only appears on real files and
folders. Right-click (cmd-shift-click while an application reads the mouse)
opens a menu to open, reveal in the file manager, open a tab or `cd` there
(only while the cursor sits on a shell prompt) and copy the path.

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
`sequenceDiagram`, ...) and end at the blank line after their body, as
agents print them after rendering Markdown. A blank line followed by a line
indented deeper than the header, or inside an open `subgraph`, `loop` or
similar block, stays in the diagram. Frames carry each block's rows and
source; the pane shows a "View diagram" chip on its first row, only for
blocks whose end is in view, and cmd-click on the block or
the palette's "View last diagram" (which searches the whole scrollback) open
it. The viewer is an HTML page written to the data directory next to a
bundled `mermaid.min.js` and opened in the default browser, so diagrams never
leave the machine.

Markdown files are previewed the same way (`chda-ui::markdown_preview`): the
file is rendered with pulldown-cmark (GitHub tables, task lists,
strikethrough, footnotes, alerts) into a page next to the same Mermaid
script, with `<base>` pointing at the file's folder so relative images load.
Fenced `mermaid` blocks become diagrams. Raw HTML is kept, and a Content
Security Policy keeps the page offline and limits scripts to chda's own,
which carry a per-page nonce: nothing remote loads, and scripts or event
handlers in the document do not run. The right-click menu of a Markdown path
and the palette (the Markdown files in the focused pane's folder) open it.

Files dropped from other applications go through
`chda-ui::external_drop`. GPUI only hands a drop to the element under the
mouse, and nothing counts as under the mouse while the last input was a key,
which a drag from Finder does not change. So targets (panes and the sidebar)
remember the dragged paths from `on_drag_move`, and an invisible layer takes
the drop on mouse up inside their bounds. A pane pastes the paths quoted for
the shell (`chda_term::shell_quote`); the sidebar adds dropped folders as
repositories.

## Workspace model

`chda-core` holds the window model: tabs, each with a binary split tree of
pane ids, the focused pane, and per-pane info (title, cwd, bell). It does
geometry (layout in the unit square, directional focus, resize, zoom) but
knows nothing about views. `chda-ui`'s `WorkspaceView` maps pane ids to
`TerminalView` entities, renders the tab bar and split tree, and turns
terminal events (exit, title, cwd, bell, focus) into model updates.

Redraws stay local. Terminal panes and the sidebar are cached views: a
window redraw (a status bar tick) reuses their last frame unless they
notified. While an agent works, the status spinner redraws the tab bar and
the visible sidebar eight times a second; panes stay cached and the timer
stops when no agent works or the system asks to reduce motion. A pane's output notifies only
its own view. Its title (agents such as Codex animate it while working) and
its once-a-second activity time do not redraw the window unless a tab label
or agent status changes; Sessions ages pick up the time on their own
one-second tick, and the session file saves it at most once every five
seconds (and on quit). The OS window title is set only when its text changes.

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

## Title bar

On macOS the system title bar is transparent and chda draws its own row
under the window buttons: the active tab's title, and an "open in" control.
Pressing on the row and moving drags the window; a double-click does what
the system setting for title bars says (zoom or minimize). The control opens
the root of the worktree the focused pane is in (else the pane's directory)
in a GUI app. `chda-ui/platform` looks up a fixed list of apps (Finder,
editors, IDEs, git clients, terminals) by bundle identifier through
`NSWorkspace`, draws each installed one's icon into a 64-pixel PNG once
after the first frame, and opens folders with `open -b <bundle id>`. The
pick is saved as `open-in` in `config.toml`. On Linux and Windows the system
title bar stays and no apps are listed yet.

## Sidebar and agents

The sidebar has two peer sections with their own scroll handles
([ADR 0019](decisions/0019-unified-sessions.md)). Sessions lists
`chda-core::SessionRow`s built by `session_rows`: one per agent pane
(`agent_live` or a chda launch) and one per other tab, terminals last, in tab
and pane order; a status change never changes a row's key or place, and
terminal output only updates activity times in place. Sessions takes its
content height up to 40% of the sidebar and is scrolled only by the user.
PROJECT takes the rest; its highlight follows the focused pane's worktree.
User focus changes (`focus_active`) reveal that worktree inside PROJECT,
moving only a clipped row to the nearest edge below the sticky repository
name; restoring focus after closing or dismissing something, window
activation, startup restore and agents' `chda mcp` requests
(`restore_focus`) only update the highlight. Only explicit PROJECT
navigation expands a collapsed PROJECT section. `sessions-collapsed` and
`project-collapsed` are saved in `config.toml`; a missing
`sessions-collapsed` is derived from the earlier `active-collapsed` and
`idle-agents-collapsed` keys without rewriting the file.

`chda-core::Sidebar` holds registered repositories, their worktrees with git
badges and branch notes (git's own `branch.<name>.description`, so other git
tools see them and they are never pushed), per-agent status (idle, working, waiting for input, review) and the
sessions found in agent transcripts. `chda-git` reads with gix and writes
with the `git` binary. `chda-agents` knows each agent: how to launch it with
hooks, where its transcripts live and how to parse them. Every agent gets its
hooks per launch, never in the user's own config:

| Agent | Hooks | Transcripts |
|---|---|---|
| Claude Code | `--settings` file | `~/.claude/projects/<dir>/*.jsonl` |
| Codex | Runtime OSC titles and per-launch `-c notify=[...]` for directly typed and menu launches, preserving session IDs for restore and chaining the user's notify program | `~/.codex/sessions/**/*.jsonl` |
| Gemini CLI | `GEMINI_CLI_SYSTEM_DEFAULTS_PATH`: the machine's system defaults plus chda's hooks | `~/.gemini/tmp/<project>/chats/*.jsonl` |
| Copilot CLI | `--plugin-dir` with a plugin whose hooks pass the event name as an argument | `~/.copilot/session-state/<id>/events.jsonl` |
| OpenCode | `OPENCODE_CONFIG_DIR` with a JavaScript plugin that maps bus events to hook kinds | none (SQLite) |

Interactive zsh, bash and fish shells receive `CHDA_CODEX_TITLE_CONFIG`.
Unless the user already defines a `codex` alias/function, their integration
script adds a shell function that runs the real CLI with that one `-c`
override and forwards all arguments and the exit status. Menu launches use
the same title layout. `chda-agents::parse_codex_title` recognizes only that
layout; `WorkspaceView` converts the OSC title of the originating pane into
the existing agent event path. Animated title updates do not repeat events.
`Ready` at startup is idle; after a turn it means completion. Codex's `Waiting`
run state means background work, while its `Action Required` title means user
input. A cleared title, shell prompt or pane exit ends tracking. Title events
have no session ID and never replace the conversation recorded by a notify
hook. See ADR 0007 for the upstream format and scope.

A pane runs a program and its arguments only, so environment variables an
adapter sets go in front through `env`. Bounded subprocess probes live in
`chda-agents::ipc`; the UI reaches them through core and never depends on
`chda-pty`. Claude status-line and Codex account quota sources and resource
sampling limits are documented in [status-bar.md](status-bar.md).

`chda mcp` (ADR 0006) is a stdio MCP server for agents. It keeps no state:
`create_worktree`, `open_tab` and `list_worktrees` become requests on the
hook socket (`{"request": ...}`, one per connection, answered with one reply
line) that the window carries out with its own config; `report_status` sends
an ordinary hook event. It serves revision 2026-07-28 and the
`initialize`-based revisions before it. Agents report through
`chda hook <agent>`, which forwards the event over a Unix socket in the data
directory, or appends to `events.jsonl` when no app is running. The UI
refreshes badges every 5 s while the sidebar is visible, one refresh per
repository at a time, and re-indexes sessions every 30 s with an mtime/size
cache. Sessions are ordered by their transcript's last write (the last
message). Resuming several at once opens one tab and gives each next session
the largest pane, split along its longer side (`Workspace::split_largest`). The UI reaches git and agents only through `chda-core`.

Every pane's shell gets `CHDA_PANE_ID`. Agents inherit it and `chda hook`
sends it back, so status is tracked per pane as well as per worktree: tabs
show the most urgent status of their panes and Sessions each agent pane's own, the Dock badge
counts waiting agents the user has not looked at, and a notification click
(through `UNUserNotificationCenter` in the app bundle) focuses that pane.

Each non-main worktree also gets its change size against the default branch
(`origin/<default>` when it exists): `git diff --numstat` from its merge base,
so commits and uncommitted edits to tracked files count and later commits on
the default branch do not. Clicking it opens a tab that runs `git diff --stat
--patch <merge base>` through `less`; quitting the pager closes the tab.

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
`pull` key changes that default. Pull request badges come from the forge CLI of each repository's host
(`chda-core::forge`), fetched at most once a minute per repository: `gh pr
list --repo <host>/<owner>/<name>` per branch, `glab mr list` and `glab mr
view` (for the head pipeline) with `GITLAB_HOST`, or one `tea pulls list`
with the `tea` login whose URL is on that host. The host and path come from
the remote `gh` would pick (`gh repo set-default`, then `upstream`,
`github`, `origin`), with SSH aliases resolved through `ssh -G` and
`repo-hosts` in `config.toml` as an override. github.com, gitlab.com,
gitea.com and codeberg.org map to their forge; other hosts go to the first
CLI logged in to them. Logins are checked per host and cached for five
minutes; a host no CLI is logged in to gets a hint on its repository row.

## Terminal text contrast

A running TUI can cache its prompt background at startup while using the
terminal's default foreground. Switching from a light theme to a dark one
then combines a retained light background with the new light text.
The renderer checks each visible text cell against its actual background,
including application-supplied RGB colors and resolved inverse colors.
Colors meeting the minimum ratio are preserved; otherwise the foreground
becomes whichever of black or white has greater WCAG contrast. Application
backgrounds, input, processes and terminal bytes remain intact. Selection
and search highlights keep their own color pairs, and SGR invisible text
remains hidden. Color emoji and images are not recolored.

chda defaults to a ratio of 3. Ghostty's `minimum-contrast` accepts 1 through
21 and reloads live; 1 disables the adjustment. This maintains readability
without restarting an agent. It does not replace a CLI's cached palette or
claim that the CLI has switched its own theme.

## Config reload and session restore

`chda-core::FileWatcher` watches the directories of the Ghostty config files
(with includes and the theme) and `config.toml`. A change reloads both; a
Ghostty value Ghostty itself would reject, or a `config.toml` that does not
parse, keeps the previous values and shows a message.

Themes resolve like Ghostty's: the theme directories first, then the
iTerm2-Color-Schemes set `chda-config` embeds (`assets/themes.txt`, packed by
`scripts/update-themes.sh`). `theme` in `config.toml` replaces the Ghostty
config's, so picking a theme in chda never edits Ghostty's files.

`chda-core::SavedSession` contains every window's tabs, split tree, ratios,
focus, zoom, tab names, pane directories and previous working activity,
plus the active window. An app-owned registry writes all windows together
in `session.json`; it also reads the earlier single-window format. Closing a
window updates only that entry. Hook events from before this app launch cannot
restore runtime agent liveness. See ADR 0010.

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
