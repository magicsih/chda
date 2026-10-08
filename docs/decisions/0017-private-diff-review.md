# ADR 0017: Private notes on a native worktree diff

Status: accepted within macOS release design 1.0. Issue: [#162](https://github.com/magicsih/chda/issues/162).

## Decision

Keep the existing diff pager and add one native Diff review tab per worktree. It has a collapsible file list, virtualized unified rows, old/new line selection and Shift-click ranges within one hunk. Binary and metadata-only changes have no text anchors. Refresh uses a resolved merge base and two matching bounded Git captures; it disables external diff and text conversion commands. Raw NUL-delimited paths provide rename identities without interpreting quoted patch headers.

The core stores a note's worktree, original diff revision, merge base, old/new paths, side, line range and exact nearby row content. Refresh attaches only a unique unchanged context in the same base. Missing or ambiguous context stays Stale until the user explicitly selects new lines. Editing, resolving and copying are separate operations. Copied notes remain open; manual pasting cannot be acknowledged by chda.

Notes live in private atomic `review-notes.json`, shared by all windows through one background repository. Failed writes preserve the editor text. Optimistic comparison rejects edits to a note changed by another window. Unsupported or corrupt storage produces an error rather than replacing it with an empty notebook. Note bodies never become notifications or diagnostic logs.

## Delivery

Send review notes prepares a fresh diff, selected notes and exact live target choices. The target must have a provider/session identity and a cwd whose actual Git worktree root matches the review; directory similarity does not identify a conversation. Nested repositories are different worktrees. Copy rechecks the diff revision, note generation and target identity, then writes plain text and focuses that pane. It never pastes, clears input, submits to a shell or posts a GitHub review.

Direct delivery remains unavailable: a session-addressed native input interface that preserves a TUI's unfinished input has not been proven. The approved manual fallback is explicit, including for working and approval-waiting agents. A target or review-tab change blocks copying and keeps the notes. Stale locations in the preview refer to the original revision, never to an inferred current line.

## Lifecycle and verification

Review tabs restore as virtual tabs without spawning PTYs. Notes persist across cold restart; open editors and batch previews defer session-preserving updates. Background queries and mutations are counted as update operations. Cross-window notification is deferred to avoid borrowing a view during its event callback.

Core/Git tests cover paths containing newlines and Unicode, renames, binary changes, external-helper exclusion, context movement/ambiguity, private persistence, corrupt versions, optimistic edits and restore/handoff. Real-shell GPUI scenarios cover menu discovery, range entry, save/refresh/edit/restore, failed writes, changed diff/session rejection and preservation of unsent terminal input. Headless GPUI scenarios do not draw pixels; the separate native macOS review and release E2E gates remain required.

Git behavior follows the [official diff documentation](https://git-scm.com/docs/git-diff), including raw NUL-delimited paths and disabling external helpers.
