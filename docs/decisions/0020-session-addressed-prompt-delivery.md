# ADR 0020: Session-addressed prompt delivery

Status: proposed; awaiting the user's decision. It would amend the Delivery
section of [ADR 0017](0017-private-diff-review.md). Research:
[#252](https://github.com/magicsih/chda/issues/252).

## Context

ADR 0017 keeps direct delivery of review notes unavailable because no
session-addressed input path that preserves a TUI's unfinished input had been
proven. On 2026-10-14 throwaway prototypes drove real Claude Code 2.1.299 and
Codex 0.162.1 sessions in private terminals.

- **Claude Code channels** (research preview): a stdio MCP server that declares
  `claude/channel` delivers to exactly the session that started it, also after
  `--resume`. It never touched typed but unsent input or an open approval
  dialog. An idle session starts a new turn; a busy one runs the event after
  the turn, or merges it at the next tool-call boundary; an open approval keeps
  it queued. The terminal shows `← <server>: …` and the transcript records a
  user message with `origin.kind = channel`. Claude sends no acknowledgement,
  but the `UserPromptSubmit` hook fires with the delivered text, and its
  `prompt_id` tells a new turn from a merge. Requirements: a legacy MCP
  handshake (servers that negotiate revision 2026-07-28 are silently not
  registered, so `chda mcp` cannot be reused), and
  `--dangerously-load-development-channels`, which shows a warning at every
  launch until a chda channel plugin is allowlisted.
- **Codex `thread/queue/add`** (experimental app-server method, also behind
  `codex queue`): stores the message durably and starts it as a new turn only
  when the thread is idle; it never merges into an active turn. It reaches
  TUIs attached to the shared daemon and embedded ones (chda's `-c` launches,
  ADR 0007), which poll the shared state about every ten seconds. The resulting
  turn carries the sender's `clientUserMessageId`, so delivery can be
  confirmed; `thread/queue/delete` cancels.
- Rejected after observation: Codex `turn/start` and `turn/steer` merge into a
  busy turn and lost the text when the user denied a pending approval, and a
  client that subscribes with `thread/resume` receives the thread's approval
  requests. Bracketed paste plus Enter submitted the user's draft together
  with the message in both TUIs and would confirm an open approval dialog.

## Decision (proposed)

Direct delivery becomes available per provider, only through these two native
paths and only when a capability check passes; otherwise the existing Copy and
focus flow stays.

- Claude Code: an opt-in `chda channel` server with a legacy-only handshake,
  added to the Claude sessions chda launches. It never declares the permission
  relay capability and relays only messages the app sends over the private
  hook socket for its own session and pane, each with `sender` and
  `delivery_id` metadata. States: sent; delivered as a new turn or merged once
  the matching `UserPromptSubmit` hook arrives; unknown after a timeout.
- Codex: `thread/queue/add` on the exact thread ID through the existing proxy
  transport, only while the target pane's Codex process is alive. States:
  queued (cancellable), started when a turn carries the message's client ID,
  completed or interrupted from that turn, unknown otherwise. The UI says the
  message runs after the current turn.
- chda never uses `turn/start`, `turn/steer`, `thread/resume` or
  `thread/inject_items` for delivery, never answers provider requests, and never
  types into a terminal.
- Every delivery comes from an explicit user action or an explicit `chda mcp`
  call, records sender, target, provider identity and outcome in the
  notification history, and claims only states the provider confirmed. An
  agent as sender needs the user's permission per target.

## Consequences and risks

- Delivered text runs with the target session's permissions; under bypass or
  `never` approval policies it can run tools without asking. chda needs its
  own sender policy because both providers trust any process of the same OS
  user.
- Claude channels are a research preview: flag names, the protocol revision and
  organization settings (`channelsEnabled`) can change, and channels do not
  work with `-p` or the Agent SDK.
- Codex `thread/queue/*` is experimental and may change; a queued message can
  run after the user's next turn when they interrupt, and in the daemon even
  when no terminal shows it.
- Feature work (Diff review delivery for #162, a Sessions "Send message"
  action, an agent-to-agent `chda mcp` tool building on #249) follows only
  after this ADR is accepted.
