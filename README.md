<p align="center">
  <img src="docs/media/hero.png" alt="chda: a terminal with a git worktree sidebar showing agent status, tabs and splits" width="880">
</p>

<h1 align="center">chda</h1>

<p align="center"><b>Checkout · Hack · Deliver · Again.</b><br>
A terminal with a git worktree sidebar and a live status board for Claude Code and Codex.</p>

<p align="center">
  <a href="https://github.com/magicsih/chda/releases/latest"><img src="https://img.shields.io/github/v/release/magicsih/chda?label=release" alt="Latest release"></a>
  <a href="https://github.com/magicsih/chda/actions/workflows/ci.yml"><img src="https://github.com/magicsih/chda/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/macOS-14%2B%20Apple%20silicon-black" alt="macOS 14 or newer, Apple silicon">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
</p>

## Why a worktree terminal

One worktree, one task, one agent. Coding agents such as Claude Code and Codex are
most productive when each runs in its own git worktree, but a normal terminal leaves
you mapping tabs to worktrees in your head. chda puts that map in a sidebar: which
worktree is running which agent, which one is waiting for your answer, which one is
finished and unread. Creating a worktree, launching the agent, merging the branch and
cleaning up all happen from the same panel, without leaving the terminal.

The terminal core is [libghostty-vt](https://github.com/ghostty-org/ghostty), so
escape sequences, Unicode and key encoding behave like Ghostty. The app is Rust on
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).

## Install

```sh
brew tap magicsih/tap
brew trust magicsih/tap
brew install --cask chda
```

Or download `chda-<version>-macos-arm64.zip` from the
[releases page](https://github.com/magicsih/chda/releases). The app is signed and
notarized.

Requirements: macOS 14 or newer, Apple silicon, `git` on `PATH`. `gh` is optional
and adds pull request badges. On a managed Mac where Homebrew cannot write to
`/Applications`, add `--appdir=~/Applications`.

## Sixty-second tour

<p align="center">
  <img src="docs/media/demo.gif" alt="Creating a worktree, running Claude Code, watching its status and merging back" width="880">
</p>

1. Press `cmd-shift-o` (or click "+ repo") and pick a git repository. It appears in
   the sidebar with its worktrees.
2. Click "+" next to the repository, type a branch name (or leave it empty for a
   random one like `brisk-otter`), press Enter. chda creates
   `<repo>.worktrees/<branch>` and opens a tab there. To stack work on another
   branch, right-click its worktree and choose "New worktree from this branch..."
3. Right-click the worktree and choose "Run Claude Code" or "Run Codex". The agent
   starts in that worktree with chda's hooks attached.
4. Watch the dot next to the branch:

   | Dot | Meaning |
   |---|---|
   | grey | idle |
   | blue | working |
   | orange | waiting for your input (you also get a notification) |
   | green | finished; clears when you look at the tab |

5. When the branch is done, right-click it and choose "Merge into main and clean
   up". chda merges, removes the worktree and deletes the branch, and refuses when
   there are uncommitted changes, unpushed commits or an open pull request.

## Features

| Area | What you get |
|---|---|
| Terminal | Ghostty-accurate VT handling, true color, wide glyphs, IME input, mouse selection and reporting, scrollback search (`cmd-f`), cmd-click links and file paths, inline images (Kitty graphics protocol), Mermaid diagrams in agent output rendered offline ("View diagram"), font size shortcuts, prompt jumping (`cmd-up` / `cmd-down`) |
| Tabs and splits | Ghostty's default shortcuts; tabs grouped by repository; rename a tab with a double-click; tabs, splits and directories come back after a restart |
| Sidebar | Repositories, worktrees, dirty / ahead / behind / conflict badges, lines added and removed against the default branch (click for a read-only diff tab), pull request state via `gh`, `glab` or `tea`, open-tab counts, an ACTIVE list sorted by last activity; every badge explains itself on hover |
| Worktrees | Create from a new or existing branch, delete with a safety check, merge-and-clean, bulk cleanup of merged branches, update a branch from its upstream when it is clean and no agent works in it (fast-forward; a diverged branch asks before a rebase or merge and never rewrites pushed commits); a note per branch saying what the task is (right-click "Edit note...", or `chda note <text>` inside the worktree), shown as the row's label and searchable in the palette |
| Agents | Claude Code, Codex, Gemini CLI, GitHub Copilot CLI and OpenCode status from their own hooks on tabs, the ACTIVE list, worktree rows and the Dock badge; notifications that open the agent's pane; jump to the agent that waits (`cmd-shift-a`); session list ordered by the last message, with one-click resume; cmd-click several sessions to resume them side by side in one tab |
| Palette | `cmd-shift-p`: every action, worktree, agent launch and session in one fuzzy list |
| Config | Reads your Ghostty font, colors and padding; chda's own settings in one TOML file; edits apply without a restart |

## How agent status works

chda is the hook. When it launches Claude Code it passes a per-session
`--settings` file that registers `chda hook claude` for session start, prompt
submit, permission requests, stop and session end. Codex gets `-c notify=[...]`
pointing at `chda hook codex`; if you already use a notify program, chda runs it
after its own. Gemini CLI gets `GEMINI_CLI_SYSTEM_DEFAULTS_PATH` pointing at a
copy of your machine's system defaults with chda's hooks added (Gemini CLI
concatenates hooks from all settings layers). GitHub Copilot CLI gets
`--plugin-dir` with a local plugin whose hooks call `chda hook copilot`.
OpenCode gets `OPENCODE_CONFIG_DIR` with a plugin that reports session status
to `chda hook opencode`; if you set `OPENCODE_CONFIG_DIR` yourself, chda leaves
it alone and OpenCode reports nothing. The hook forwards only the session id, working directory, event
kind, a timestamp and the chda pane it runs in (from `CHDA_PANE_ID`, which
every chda shell sets) to the running app over a local socket. Prompt text and
transcripts are never stored. None of the agents' own settings files
(`~/.claude/settings.json`, `~/.codex/config.toml`, `~/.gemini/settings.json`,
`~/.copilot`, `~/.config/opencode`) are modified.

Past sessions are listed under each worktree for Claude Code, Codex, Gemini CLI
and Copilot CLI. OpenCode keeps its sessions in a database chda does not read.

## Configuration

chda reads Ghostty's config files in Ghostty's order:
`~/.config/ghostty/config` and `config.ghostty`, then on macOS the same two in
`~/Library/Application Support/com.mitchellh.ghostty`. It honors `font-family`,
`font-size`, `font-feature`, `theme`, `background`, `foreground`, `palette`,
`cursor-style`, `cursor-style-blink`, `window-padding-x/y`, `shell-integration`
and `scrollback-limit`, so a Ghostty user gets the same look without copying
anything. Like Ghostty, chda ships JetBrains
Mono as the default font and Symbols Nerd Font Mono for prompt and TUI icons.

Programming ligatures (`->`, `!=`, `>=` in JetBrains Mono, Fira Code and
similar fonts) are off by default. Turn them on with `font-feature = calt`;
`font-feature` takes Ghostty's syntax (`-liga`, `ss01`, `cv01=2`, repeated or
comma-separated). The character under the cursor is always drawn on its own.

chda's own settings live in `~/.config/chda/config.toml`:

```toml
repos = ["/path/to/repo"]
worktree-path-template = "{repo_parent}/{repo_name}.worktrees/{branch}"
default-action = "terminal"   # terminal | claude | codex | gemini | copilot | opencode
tab-title = "branch"          # branch | path
agents = ["claude", "codex"]  # offered in menus; add "gemini", "copilot", "opencode"
sidebar-width = 280
sidebar-visible = true
notifications = true
restore-session = true        # reopen the last tabs, splits and directories
restore-agents = true         # ...and the agent conversations those panes had open
editor = "zed {file}:{line}:{column}"  # opens cmd-clicked paths; unset: default app
theme = "Catppuccin Mocha"    # replaces the Ghostty config's theme; unset: follow it
pull = "ff-only"              # ff-only | rebase | merge: "Update branch" on a diverged branch

[repo-hosts]                  # pull request host when the remote does not say
"/path/to/repo" = "github.example.com"
```

"Select theme..." in the command palette lists the 650+ themes chda bundles
(the iTerm2-Color-Schemes set Ghostty ships) plus any in
`~/.config/ghostty/themes`. The arrow keys preview each one; `enter` saves it as
`theme` in `config.toml`, `esc` puts the previous one back.

Both files are watched: saving one updates open windows within a second. A
value Ghostty would reject keeps the previous settings and shows a message.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `cmd-t` / `cmd-w` | new tab / close pane |
| `cmd-1`…`cmd-9`, `cmd-shift-[` `]` | switch tabs within the repository |
| `cmd-d` / `cmd-shift-d` | split right / down |
| `cmd-alt-arrows`, `cmd-[` `]` | move between splits |
| `cmd-ctrl-arrows`, `cmd-ctrl-=` | resize / equalize splits |
| `cmd-shift-enter` | zoom a split |
| `cmd-up` / `cmd-down` | previous / next shell prompt |
| `cmd-f` | search the scrollback; `enter` / `shift-enter` older / newer, `alt-c` case, `alt-r` regex, `esc` close |
| `cmd-click` | open a link or file path (hold `cmd` to see it underlined: solid for files, dotted for folders) |
| drop files on a pane / a folder on the sidebar | paste the quoted paths / add the folder as a repository |
| right-click a path | open, reveal in Finder, open a tab or `cd` there, copy the absolute or relative path (`cmd-shift-click` in apps that read the mouse) |
| `cmd-=` / `cmd--` / `cmd-0` | bigger / smaller / configured font size |
| `cmd-shift-a` | go to the agent that waits for input, then to finished turns |
| `cmd-b` | toggle the sidebar |
| `cmd-shift-o` | add a repository |
| `cmd-n` | new worktree |
| `cmd-shift-p` | command palette |
| `cmd-c` / `cmd-v` | copy selection / paste |
| `cmd-,` / `cmd-shift-,` | open `config.toml` / reload both configs |
| `cmd-m`, `cmd-h`, `cmd-alt-h` | minimize, hide chda, hide others |

Every action is also in the menu bar, with agent launches and session resume
under Agents.

## Status and roadmap

v0.1.x runs on macOS with Apple silicon. Linux and Windows compile in CI and are
the next milestone. See [docs/roadmap.md](docs/roadmap.md) and
[CHANGELOG.md](CHANGELOG.md).

## FAQ

**How is this different from Ghostty?** Ghostty is the terminal; chda borrows its
core and adds the worktree sidebar and agent status board. If you do not run
several worktrees at once, Ghostty is the better terminal.

**Intel Mac?** Not yet. The release is arm64 only; an x86_64 build is a matter of
adding it to the release workflow once someone can test it.

**macOS says the app is damaged or from an unidentified developer.** Releases are
signed and notarized, so this should not happen. If it does, you probably have a
build from somewhere else; download the zip from the releases page.

**Do I need `gh`?** No. Without it you lose only the pull request badges.
Each repository asks the host of its remote: `gh` for GitHub and GitHub
Enterprise Server, `glab` for GitLab, `tea` for Gitea and Forgejo (Codeberg).
github.com, gitlab.com, gitea.com and codeberg.org are recognized by name; any
other host goes to whichever of the three is logged in to it. SSH aliases from
`~/.ssh/config` are resolved with `ssh -G`. A repository whose host is not
logged in shows the command to run instead of badges.

**Other shells?** Status, directory tracking and prompt jumping come from shell
integration for zsh, bash and fish. Other shells fall back to polling the
shell's working directory; prompt marks are missing there.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the build setup (Rust, Zig 0.15.2,
Xcode) and [docs/architecture.md](docs/architecture.md) for the crate layout.
Bugs and ideas go to [issues](https://github.com/magicsih/chda/issues).

## Acknowledgements

[Ghostty](https://ghostty.org) for libghostty-vt, [Zed](https://zed.dev) for GPUI,
and [Ghostree](https://github.com/sidequery/ghostree) for showing that a worktree
sidebar belongs inside the terminal.

## License

MIT. The bundled fonts keep their own licenses:
[JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) under the SIL Open
Font License 1.1 and [Symbols Nerd Font](https://github.com/ryanoasis/nerd-fonts)
under MIT; both texts are in `crates/chda-ui/assets/fonts`. The bundled themes
come from [iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes)
under MIT (`crates/chda-config/assets/themes-LICENSE.txt`).
The diagram viewer bundles [Mermaid](https://github.com/mermaid-js/mermaid)
12.1.0 under MIT (`crates/chda-ui/assets/mermaid/LICENSE`).
