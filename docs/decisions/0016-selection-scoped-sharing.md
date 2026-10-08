# Selection-scoped sharing previews

Status: accepted. Approved design: [macOS next release, section 2.5](../design/2026-10-08-macos-next-release/design.md). Issue: #158.

## Decision

A terminal selection context menu offers ordinary Copy and Copy for sharing. Sharing requests the same VT selection through a tagged reply without changing the clipboard. The selected pane, tab and exact provider conversation remain fixed while source lookup runs off the UI thread. Menu invocation, async completion and final copy each revalidate that scope. A stale reply cannot reactivate a pane or copy another conversation.

Local transcript lookup is bounded by time, visited entries, file size and answer count. Only assistant text from a confirmed exact conversation participates. Incomplete lookup cannot establish a unique match. A unique literal partial match retains only the selected substring. Claude Code 2.1.294 and Codex 0.161.0 gutters are removable only when the entire selected answer equals the entire original assistant block after removing its confirmed display prefix. Pipes, quotes, Markdown and indentation inside the answer remain untouched. Other layouts and ambiguous matches retain the exact VT selection for manual editing; there is no generic Markdown or border removal.

The preview identifies the captured conversation, shows source-match state and offers the original terminal selection alongside an editable plain-text field. Enter copies, Shift-Enter inserts a newline, Escape cancels and returns focus. Source matching never logs or persists answer bodies. No message is sent to Slack or to an agent.

## Evidence

Installed Claude Code 2.1.294 and Codex 0.161.0 rendered a synthetic resumed conversation in an isolated configuration and a real 100-column PTY without model requests. Codex’s `AgentMarkdownCell::render_lines` at tag `rust-v0.161.0` uses a first-line bullet and two-column continuations. Claude’s captured first-line marker is `⏺ ` with two-column continuations. The formatter accepts only these confirmed versions plus a full original-text match; rich-rendered changes that cannot match are left for review.

Tests first failed when the sharing action and gutter matching were absent. GPUI scenarios use a real shell and VT selection and cover clipboard preservation, edited Unicode/multiline text, cancellation, menu and final-copy scope changes. These scenarios draw no pixels and do not replace the required native macOS and Slack-composer verification.

Captured selections, source text, CLI versions and ANSI capture hashes are recorded in [sharing-cli-renderings.json](../../crates/chda-agents/tests/fixtures/sharing-cli-renderings.json). Captures replay through the actual VT selection formatter. Only synthetic assistant text is retained; onboarding, configuration and temporary paths are excluded. Claude transforms an ordinary Markdown quote into a display border, while Codex transforms list bullets; these fixtures prove such changed text remains editable without automatic replacement.
