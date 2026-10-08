# ADR 0018: Explicit preparation of new worktrees

Status: accepted within macOS release design 1.0. Issue: [#163](https://github.com/magicsih/chda/issues/163).

## Decision

The repository menu's Worktree preparation editor stores an ordered list of exact shell commands and explicitly named relative files. Both lists default to empty. Fields preserve spaces, Unicode and intentional newlines. Saving settings executes nothing. Configuration imports cannot supply consent.

Checkout completes before preparation. The first or changed plan requires a preview of the actual working directory, source and destination paths, file actions and ordered commands. Consent lives separately in private atomic app data and matches the canonical primary checkout, shared Git directory, its creation timestamp and the exact plan. Replacing a repository or changing a command/file list requires new confirmation. Filesystems without a usable directory creation timestamp report an identity error instead of inferring consent. Corrupt, unsupported or oversized approval data is kept and reported.

## File access

Sources must be untracked and gitignored. Git's NUL-delimited check-ignore protocol preserves paths containing spaces and newlines and excludes force-added tracked files. Only regular files are supported. All directory components are opened without following symlinks through cap-std/cap-fs-ext directory capabilities. Absolute paths and traversal are rejected; final source opens also prohibit symlinks and use nonblocking mode so special files cannot wait for a writer. Source metadata and destination state are inspected before confirmation and rechecked before execution.

Copies create independent files exclusively, protect new files before writing any bytes and preserve source permissions after completion. Existing files are never overwritten. A failed or cancelled copy keeps the worktree and any files already created, including a partially copied file. Explicit Retry names existing regular files as Keep existing and reruns commands from the beginning. Users can inspect or remove a partial file in the retained worktree before retrying. Directories and symlinks are rejected even on retry. A changed destination action requires a new review.

## Execution and navigation

A worker owns the preparation operation and its cancellation flag. It copies selected files before running commands serially in the configured shell at the new worktree, with the captured pane environment. Commands have no terminal input and stdout/stderr are discarded. Progress contains only the stage, command number and exit result. File content, command output and environment values never enter chda notifications or diagnostic logs.

Cancellation terminates and reaps only the owned helper process group and prevents later steps. Failure retains the worktree and blocks launch. Explicit Skip is the only unfinished-preparation action that proceeds to the planned terminal or agent. Successful background completion preserves the current tab and focus; Open is an explicit action. Claude/Codex still use the existing per-launch permission confirmation. Setup in the title bar reopens hidden runs.

App-owned preparation ownership is shared across windows. Ordinary tab/agent launches and MCP OpenTab cannot bypass an outstanding run; its row remains busy for repository mutation checks. MCP CreateWorktree also uses this preparation gate, including when it does not request a tab. Closing the owning window cancels its worker. In-flight workers, their completion callbacks and retained runs/editors defer session-preserving updates; the PTY prepared/commit contract is unchanged. Closing a retained run keeps its filesystem data. Cold restart never automatically reruns setup commands.

## Verification

Temporary-repository tests cover exact private consent, changed plans, bounded/corrupt approval data, tracked and nonignored sources, missing files, directories, symlinks, traversal, FIFO rejection, independent Unicode-path copies, existing edits, ordered commands, exit failures and cancellation. Real-shell GPUI scenarios cover imported plans, confirmation, failure/edit/retry, cancellation/explicit skip with native launch options, MCP launch gating and another pane's unfinished input during completion. These are headless scenarios; pixel/UX review in the macOS development app and release E2E remain separate gates.

The file-access APIs follow the [cap-fs-ext directory documentation](https://docs.rs/cap-fs-ext/4.0.3/cap_fs_ext/trait.DirExt.html) and [cap-std capability model](https://github.com/bytecodealliance/cap-std/blob/main/README.md).
