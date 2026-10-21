# Validation

Date: 2026-10-21. Approved scope: [approval.md](approval.md).
Implementation: `eee1edced4655412b25a2c812d96389806b6fc58` and the follow-up
`64fbe007c8665eb1f91f1166897f7bca376ee6a8`.

## Executed locally

| Check | Result |
| --- | --- |
| Requirement test before implementation | MiniMax provider registration failed on the baseline, then passed with the adapter |
| Workspace regression | `cargo test --workspace --no-fail-fast` passed, including 243 UI crate tests; used the verified pinned Ghostty source, Zig 0.16.0 and a per-process open-file limit of 8192 |
| Follow-up MiniMax tests | Five adapter tests passed, covering native payloads, startup isolation, executable lookup, exact pane contexts, permission failures, collisions, modified files, previous-byte recovery and configuration preservation |
| Actual hook process | Binary integration test passed with the generated Node helper, a restricted native-shaped environment and the real `chda hook minimax` process; separate panes, child startup exclusion, input waiting, continued work, completion and metadata-only fallback logs verified |
| Installed CLI | Explicitly selected mcode 0.6.12 discovered the generated local package in a fresh temporary HOME and data directory; disabling it and repeating installation preserved `enabled: false` |
| Formatting and lints | `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` passed |
| Documentation | Product documentation tests and `scripts/sync-product-docs.py --check` passed |
| Pages layout | Inspected screenshots at 1280px and 390px; no horizontal overflow or clipped provider text |
| Native entrypoint paths | A provider directory containing quotes and `$(...)` reproduced path evaluation with the original command; reading PLUGIN_ROOT inside Node fixes it, and the binary integration now runs the declared native shell entrypoint |

The workspace regression covers the implementation commit. The follow-up's
additional root-session guard and permission test were checked by the targeted
MiniMax tests and actual hook-process integration. Existing UI behavior was
unchanged by that follow-up. Cross-platform CI must check the final PR head.

Pages evidence: [desktop](review-pages-desktop.png),
[narrow](review-pages-narrow.png). These show the local documentation page,
not a native terminal window.

An initial workspace run exhausted macOS's default 256 open-file limit; the
complete run passed with 8192. One bridge run lost an event with the original
three-second child deadline. The child deadline is now eight seconds within
the native hook's ten-second limit, and the hook integration passed again.

## Boundaries

No authenticated Gemini/MiniMax conversation through chda or native desktop
interaction was executed.
All provider tests used temporary data or synthetic events. The installed CLI
check did not request a model response or touch the user's MiniMax data.
MiniMax historical conversations, exact resume, restart and usage remain
unsupported. No release, tag, merge or public deployment is part of this work.
The local MiniMax advisory review and its one retry both reached the configured
40-step execution limit at `93018bc`; no review result was obtained. The later
native-entrypoint correction was self-reviewed and verified by its regression
test. Required checks are reported in the PR rather than inferred from this
local record.

## Fresh-launch follow-up

A new app process can reuse a numeric pane ID whose MiniMax context survived
the previous process. Its old root-session binding prevented the new
`SessionStart` and subsequent status events from being reported. Fresh managed
launch preparation now removes only that pane's previous `state.json` before
publishing its context. Process adoption skips preparation and retains the
running session's binding.

The actual hook-process integration was extended to prepare the same pane
again and report a new root's startup and prompt. It failed before the fix
(zero events instead of two) and passed afterward. The five MiniMax adapter
tests, workspace clippy and formatting also passed.

After the user requested another advisory review, the 100-step run at
`dcd4c35` also ended at its execution limit after approximately nine minutes.
Its log showed blocked attachment reads and continued source exploration.
A corrective retry supplies the code and surrounding implementation directly
as text and prohibits further tool exploration, with the same 100-step and
20-minute limits. Its outcome and the current candidate's CI are recorded in
the PR. Review output remains outside the repository and is not posted to the PR.
