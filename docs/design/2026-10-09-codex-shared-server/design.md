# Codex 공유 서버 연결 — 추가 설계 1.1

- 상태: 검토 대기. 구현 승인이나 실행 검증 완료가 아니다.
- 연결: [#173](https://github.com/magicsih/chda/issues/173), [기존 릴리스 설계 1.0](../2026-10-08-macos-next-release/design.md), [시각 검토](review.html).
- 영향: Codex launch/status adapter, zsh/bash/fish wrapper, exact resume, child activity. 다른 기능의 승인 1.0은 유지한다.

## 목적과 확인된 원인

chda는 일반 Codex 실행에도 terminal title과 notify 설정을 `-c`로 추가한다. 설치된 Codex 0.161.0의 공식 `config_exclusion`은 이 두 설정을 공유 서버 사용 가능 목록에 포함하지 않는다. 따라서 일반 실행이 embedded 방식으로 바뀌고 경고가 나온다. 하나만 제거해도 남은 설정이 같은 조건에 걸린다.

근거: [고정된 0.161.0 공유 서버 선택 코드](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/daemon_startup.rs#L59-L92), [연결 선택과 remote 모드 코드](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/lib.rs). 설치된 CLI의 `--help`도 `--remote unix://PATH`를 제공한다. 기존 daemon의 metadata-only 조회는 실행으로 확인했지만, 아래 pane 연결 중계는 아직 실행으로 확인하지 않았다.

## 제안과 사용자 흐름

일반적인 지원 Codex TUI 실행은 provider의 공유 서버에 연결하되, pane별 private local endpoint를 경유한다. chda가 title/notify `-c`를 추가하는 대신, 이 연결에서 provider가 확인한 thread/turn 상태만 읽어 exact pane에 연결한다. CLI의 화면, 입력, 대화, 모델 실행은 Codex가 담당한다. 사용자는 기존대로 `codex`, 메뉴 시작, exact resume를 사용한다.

CLI → private pane relay → `codex app-server proxy` → 기존 provider daemon의 구조다. relay는 원래 통신을 전달하면서 확인된 ID·상태만 UI로 보고한다. 모델 요청을 새로 생성하거나 대화를 cwd로 추정하지 않는다. 직접 review-note 전달은 이번 변경에 포함하지 않는다.

**조건부 채택:** `--remote`는 Codex에서 별도의 client 설정 경로를 사용한다. 공유 서버에 연결됐다는 사실만으로 기존 권한·인증 설정이 보존된다고 판단하지 않는다. 기본 실행과 중계 실행의 configuration/approval/sandbox/managed authentication/notify 동작을 대조해 모두 같은 계약을 만족할 때만 일반 실행에 활성화한다. 만족하지 않으면 #173을 차단 상태로 유지하고 근거와 대안을 다시 검토한다.

## 책임과 데이터

| 위치 | 책임 | 보존하지 않는 데이터 |
| --- | --- | --- |
| `chda-agents` | 지원 capability 확인, 사용자 실행 옵션 보존, provider 상태 해석 | prompt, answer, tool body |
| `chda-agents/ipc` | private endpoint, raw transport, owned proxy lifecycle | raw frame logs, auth values |
| `chda-term` | shell wrapper와 원래 CLI 실행·exit status | 별도 채팅 UI |
| `chda-core` | exact provider/thread/pane 상태, 기존 resume·child 규약 | cwd 기반 대화 추정 |
| `chda-ui` | 기존 상태 표시, 명확한 연결 오류 | 자동 모델 요청·입력 덮어쓰기 |

허용 상태는 provider가 확인한 thread ID와 start/working/approval-waiting/turn-complete/ended다. Unknown은 Unknown으로 남긴다. 본문은 메모리 내 통신 전달에만 사용하며 로그·디스크·복구 자료에 저장하지 않는다. provider 응답과 request ID를 대조해 다른 pane으로 연결하지 않는다. 완료 이벤트는 동일 turn 기준으로 중복 처리하지 않는다.

## 실행·실패·복구

- endpoint는 private directory와 현재 사용자 접근 권한으로 만든다. 주소는 이 실행의 부속 정보이며 다음 실행의 영구 launch options에 넣지 않는다.
- relay/proxy는 해당 PTY 실행에 속한다. chda의 GUI 교체 때 원래 CLI·shell과 함께 유지하고 `prepared/commit` 경계를 바꾸지 않는다. 종료 시 해당 실행의 자원만 정리한다. 사용자 daemon과 다른 Codex 실행은 종료하지 않는다.
- 연결 실패는 설명 가능한 실행 오류다. 경고 숨기기, 자동 `--no-daemon`, 새 embedded 대화 시작, hook-trust 우회로 성공처럼 처리하지 않는다.
- 사용자가 직접 선택한 `--remote`, `--no-daemon`, profile, custom configuration, aliases/functions는 원래 의미를 유지한다. 해당 경로를 임의로 일반 중계 경로로 바꾸지 않는다. 구형 CLI의 기존 지원 범위는 version/capability 근거로 구분한다.
- 전역 Codex config/auth 파일을 수정하거나 복제하지 않는다. user notify를 제거하지 않는다. 대체된 일반 실행용 title/notify 주입은 함께 제거한다.

## 대안과 결정

| 방법 | 판단 |
| --- | --- |
| 경고 필터 또는 자동 embedded 선택 | 실행 방식 문제가 그대로여서 제외 |
| title/notify 둘 다 제거하고 cwd로 상태 추정 | exact 대화·동시 pane 상태를 잃어 제외 |
| 전역 설정에 chda 옵션 저장 | 사용자 설정과 다른 CLI에 영향을 주므로 제외 |
| pane relay와 provider 원본 상태 | 제안. 보존 조건을 통과해야 활성화 |
| upstream의 local client 상태 API 추가를 기다림 | 중계가 보존 조건을 만족하지 못할 때 재검토할 대안 |

## 인수조건과 검증

| 단계 | 검증 | 통과 근거 |
| --- | --- | --- |
| 구조 | fake raw transport에서 ID/turn/pane, partial frames, errors, cancellation, owned process cleanup RED→GREEN | bounded transport/status tests |
| native capability | 모델 요청 없이 실제 CLI가 기존 daemon에 연결; startup/config/permission/notify 비교 | 실행 모드와 provider 응답, 차이 기록 |
| native lifecycle | zsh/bash/fish, menu, exact resume, concurrent panes, waiting/completed, user options/notify | 원래 대화 ID, 상태, 종료 동작 |
| update | relay·CLI·shell PID, typed input, children 유지 | 기존 native update E2E에 추가 |
| 공개 문서 | catalog·ADR·README/Pages와 실제 지원 범위 일치 | source와 native 증거 분리 |

native lifecycle은 모델 사용이 필요한 실제 turn과 모델 호출 없는 capability probe를 구분한다. 기존 3회 macOS 기능·UX 검수와 build 전/publish 전 전체 E2E에 포함한다. 중계 검증을 이유로 기존 검수 횟수와 릴리스 gate를 줄이지 않는다.

## 승인과 추적

이 추가 설계는 기존 Codex adapter의 연결 구조를 바꾸므로 정확한 문서 commit과 1.1 범위의 승인을 받은 뒤 제품 구현에 적용한다. 문서 병합은 구현 승인이 아니다. 승인 전에는 공식 자료 확인과 비파괴 capability 조사, 기존 1.0 범위 작업을 계속한다. 결과가 보존 조건에 어긋나면 확인된 사실을 #173에 기록하고 변경된 대안을 검토한다.
