# Security

## Reporting

Report vulnerabilities privately through
[GitHub security advisories](https://github.com/magicsih/chda/security/advisories/new).
Please do not open a public issue for them. You will get a reply within a week.

## What chda touches

- **Agent hooks.** chda registers itself as a hook command for Claude Code
  (per launch, through `--settings`) and Codex (per launch, through
  `-c notify`). The hook receives the agent's event payload and forwards only
  the session id, working directory, event kind and a timestamp to the running
  app over a Unix socket in `~/Library/Application Support/chda` (mode 0600).
  Prompt text and transcripts are never stored.
- **Session lists.** chda reads `~/.claude/projects` and `~/.codex/sessions`
  to list past sessions. It keeps the first prompt (120 characters) in memory
  for display and writes nothing back.
- **Git.** Reads use gix in-process. Writes (worktree add/remove, merge,
  branch delete) run the `git` binary on `PATH`. Pull request badges run `gh`
  on `PATH`; chda does not read its token.
- **Network.** chda itself makes no network requests. `git` and `gh` do, with
  their own credentials.
- **Shell integration.** chda points `ZDOTDIR` at its own zsh scripts, which
  restore your `ZDOTDIR` and source your `.zshenv`. The scripts live in
  `~/Library/Application Support/chda/shell-integration` and are embedded in
  the binary.

## Supported versions

Only the latest release receives fixes.
