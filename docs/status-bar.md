# Quota and selected-pane resources

Quota is separate from conversation tokens, context size and estimated cost.
The compact bar chooses the highest reported quota window, with its name.
Details keep every window and scope; reports are never added together.

Claude Code supplies five-hour, seven-day and gateway spend-limit windows
through a per-launch status-line command. Reset timestamps come from the
provider. The payload has no verified account identity, so reports stay
separate by session. Reports arrive on CLI status-line events and become stale
after five minutes. Before the first usage report, the bar says **Waiting for
usage**, including after a restart. Claude supplies subscription windows only
after its first API response. An event without quota does not erase the same
session's last valid reading or change its observation time. Another session
does not inherit those values. Existing user/project status-line commands still receive
the original payload and produce their usual output. User settings are not
overwritten. A custom status line can change Claude footer hints; unsupported
versions or project overrides can make quota unavailable. See the
[official reference](https://code.claude.com/docs/en/statusline).

All windows share provider reports and one Codex query loop. Closing the first
window does not stop refreshes in the remaining windows. Codex is queried once
per minute after successful reads through its local app-server protocol:
initialize, initialized, account/read with refreshToken=false, then
account/rateLimits/read. Probes have a fifteen-second deadline and their process
groups are reaped. Only supported ChatGPT account snapshots produce quota.
Account identity and named buckets stay separate. Startup says **Loading**.
Transient failures say **Retrying**, with retries after 2, 5, 10 and then at most
30 seconds between attempts. When a probe verifies the account and then loses
its quota connection, the same account's last reading remains visible with a
retrying label and its original observation time. If account verification
fails or the server rejects a request, history remains only in details until
verification succeeds again. Signed-out, missing-CLI and unsupported modes say
**Sign in**, **Not installed** and **No quota**, respectively; these are checked
again once per minute. No login, conversation,
model request or credential-file reader is introduced. See the
[official reference](https://developers.openai.com/codex/app-server/).

Countdowns use actual reset timestamps and update locally. After a reset,
chda waits for a new report rather than inventing a zero. Loading, missing,
authentication, retry and stale states are explicit. Reports stay in
memory with at most 128 retained scopes.

On macOS, a worker samples the selected pane's PTY process and descendants
every two seconds. CPU uses one-core percentages, so parallel work can exceed
100%; the initial sample is unknown. Summed resident set size (RSS) can count
shared pages repeatedly and is not exact physical memory. PID and start time
identify the root; exited or reused PIDs are not zero usage. A reused PID
stays rejected on subsequent ticks and after switching away and back. Each
visited pane keeps its own sampler until that pane closes; only the selected
pane is sampled, and a new pane starts with an unknown CPU sample.

lsof has a one-second deadline and a 1 MiB output bound. Listeners refresh
approximately every six seconds (every third sample). Details list TCP LISTEN
and bound UDP endpoints, protocol, address and owning PID, excluding outbound
connections. Process count and root uptime stay in details. Inspection errors
remain visible and no elevated access is requested. Linux and Windows keep
building with explicit unsupported resource inspection.

Disk throughput and network traffic were considered and omitted: attribution
and sampling cost are less reliable than the PTY subtree. The compact bar keeps
quota, CPU, RSS and listener counts to leave room for terminal work.
