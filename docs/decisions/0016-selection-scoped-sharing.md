# Selection-scoped sharing previews

Status: accepted. Approved design: [macOS next release, section 2.5](../design/2026-10-08-macos-next-release/design.md). Issues: #158, #229. Additional approved design: [0.1.23, section 2.4](../design/2026-10-12-v0.1.23/design.md#24-setup경로공유-복사).

## Decision

A terminal selection context menu offers ordinary Copy and Copy for sharing. Sharing requests the same VT selection through a tagged reply without changing the clipboard. The selected pane, tab and exact provider conversation remain fixed while source lookup runs off the UI thread. Menu invocation, async completion and final copy each revalidate that scope. A stale reply cannot reactivate a pane or copy another conversation.

Local transcript lookup and matching share a two-second deadline and bounds on visited entries, file size and answer count. Only assistant text from the exact provider conversation participates. Malformed or incomplete lookup cannot establish a unique match. Literal and confirmed display matches count all occurrences, including overlaps; matches from different answers or source ranges retain the original selection.

A version-specific display projection keeps original source offsets while omitting only confirmed display gutters and hidden top-level fence delimiter lines. Claude Code 2.1.296 and Codex 0.162.1 actual PTY/VT captures confirm two-column continuation gutters and omitted fenced-code delimiters. The same capture confirms Claude's two-column nesting levels for hyphen lists: selected display indentation maps back to its original whitespace, including 2/4/8-space source indentation. Each displayed indentation cell maps to its corresponding source whitespace group. Partial first and last lines remain partial; the formatter concatenates only mapped selected ranges, never hidden fences or the remaining answer. Old Claude 2.1.294 and Codex 0.161.0 fixtures retain their confirmed plain-gutter behavior.

This is exact source correspondence, not generic Markdown cleanup. Actual pipes, quotes, table cells, bullets and code indentation remain source content. Claude's quote/table borders, Codex's changed list bullets/table layout, unknown versions and unconfirmed layouts remain the original selection whenever they cannot match uniquely. Ordinary Copy is unchanged.

The preview identifies the captured conversation, shows source-match state and offers the original terminal selection alongside an editable plain-text field. Enter copies, Shift-Enter inserts a newline, Escape cancels and returns focus. Source matching never logs or persists answer bodies. No message is sent to Slack or to an agent.

## Evidence

Installed Claude Code 2.1.294 and Codex 0.161.0 rendered a synthetic resumed conversation in an isolated configuration and a real 100-column PTY without model requests. Codex’s `AgentMarkdownCell::render_lines` at tag `rust-v0.161.0` uses a first-line bullet and two-column continuations. Claude’s captured first-line marker is `⏺ ` with two-column continuations. The formatter accepts only these confirmed versions plus a full original-text match; rich-rendered changes that cannot match are left for review.

Tests first failed when the sharing action and gutter matching were absent. GPUI scenarios use a real shell and VT selection and cover clipboard preservation, edited Unicode/multiline text, cancellation, menu and final-copy scope changes. These scenarios draw no pixels and do not replace the required native macOS and Slack-composer verification.

Captured selections, source text, CLI versions and ANSI capture hashes are recorded in [sharing-cli-renderings.json](../../crates/chda-agents/tests/fixtures/sharing-cli-renderings.json). Captures replay through the actual VT selection formatter. Only synthetic assistant text is retained; onboarding, configuration and temporary paths are excluded. Claude transforms an ordinary Markdown quote into a display border, while Codex transforms list bullets; these fixtures prove such changed text remains editable without automatic replacement.


## 0.1.23 implementation evidence

Claude Code 2.1.296 and Codex 0.162.1 resumed synthetic local conversations in an isolated 100×50 PTY with closed loopback model endpoints. ANSI output was replayed through the pinned Ghostty terminal, and mouse selection extracted a code/list subsection for Claude and a code subsection for Codex. Both fixtures failed against the old full-answer implementation. The source projection restores only their selected indentation and omits both hidden delimiters. An additional Claude capture confirms nested hyphen-list indentation at 2/4/8 source spaces and shows why changed ordered-list labels require fallback. Only synthetic source/selection text, versions and ANSI hashes are committed; onboarding and temporary paths stay private.

These captures establish formatter inputs. They do not constitute the final independent native review rounds or release E2E.
