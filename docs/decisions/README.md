# Architecture decision records

Summaries of decisions that are hard to reverse. Propose a change by adding a
new record rather than editing an accepted one.

| ADR | Decision |
|---|---|
| 0001 | Terminal core is `libghostty-vt` through C FFI, isolated in `chda-term` |
| 0002 | GUI framework is GPUI, pinned to a Zed git commit |
| 0003 | Worktree backend is in-app: gix for reads, `git` subprocess for writes |
| 0004 | macOS first; Linux and Windows must keep building in CI |
| 0005 | Name `chda`, MIT license, personal public repository |
| 0006 | `chda mcp` is a stateless MCP server relaying to the running app over the hook socket |
| 0007 | Proposed: directly typed Codex reports per-pane activity through runtime OSC titles |
| 0008 | Terminal and read-only Git graph tabs have explicit content types; history is queried in the background and rendered incrementally |
| 0009 | Keep live idle panes separate from shells and saved conversations; focus and close by pane ID |
| 0010 | Own windows, recent pane navigation, IPC routing and saved working context at app scope |
| 0011 | Session-preserving Sparkle updates: approved direction, handoff primitives implemented, updater integration pending |
| [0012](0012-window-notification-history.md) | 창별 알림 기록, 읽음 기준과 세션 보존 업데이트에서의 기록 전달 |
| [0013](0013-captured-agent-launches.md) | 정확한 대화에 실행 파일·옵션·권한을 보존하는 관리 실행 |
| [0014](0014-managed-process-restart.md) | 관리 프로세스 종료 정보와 이전 출력 보존, 동일 pane에서의 정확한 재시작 |
