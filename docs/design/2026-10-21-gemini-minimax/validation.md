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

No live authenticated model turn or native desktop interaction was executed.
All provider tests used temporary data or synthetic events. The installed CLI
check did not request a model response or touch the user's MiniMax data.
MiniMax historical conversations, exact resume, restart and usage remain
unsupported. No release, tag, merge or public deployment is part of this work.
Required checks and local review are reported in the PR rather than inferred
from this local record.
