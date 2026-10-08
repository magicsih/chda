# 0014 — 관리 프로세스의 종료와 정확한 대화 재시작

승인 근거: [설계 1.0](../design/2026-10-08-macos-next-release/design.md), #160.

에이전트의 답변 상태와 자식 프로세스의 수명을 분리한다. `ManagedRun`은 Starting, Running, Stopped를 저장한다. chda가 실행한 프로세스만 실제 실행 파일·cwd·옵션·권한을 기록한다. shell에서 타이핑한 명령을 추정하여 shell을 교체하지 않는다. 일반 shell 종료는 기존 pane 닫기를 유지한다.

종료 시 VT 스레드가 화면과 유지 중인 scrollback을 내보낸다. 이전 실행 출력은 data directory의 `agent-output` 아래 0600 파일에 원자적으로 저장하고, 세션 JSON에는 검증 가능한 basename만 보관한다. 읽기 전용 목록은 PTY나 입력 채널을 갖지 않는다. 이전 출력을 보는 동안 키보드 초점은 workspace에 둬 숨겨진 실행 프로세스에 입력하지 않는다. 한 pane에서는 가장 최근 종료 출력 파일을 교체하여 누적하지 않는다.

재시작은 기록된 exact session의 로컬 transcript가 확인된 경우에만 시작한다. 파일명 접미사나 최신 대화로 대체하지 않는다. Claude는 정확한 파일명, Codex는 rollout metadata의 전체 ID, Gemini는 bounded metadata의 sessionId, Copilot은 정확한 session directory를 확인한다. transcript를 확인할 수 없는 provider는 그 제약을 설명한다. 계정 정보는 확인된 launch 시점의 report만 보존하며 인증이나 계정을 바꾸지 않는다.

기록된 대화·디렉터리를 바꾸는 옵션은 exact resume에서 거부한다. cwd·CLI·transcript가 없어도 pane과 이전 출력을 남긴다. 프로세스 준비는 재시작 시 background에서 실행하고 Starting 상태로 중복 요청을 막는다. 닫힌 pane의 완료 작업은 새 pane을 대상으로 하지 않는다. 이전 runtime보다 오래된 hook도 적용하지 않는다.

종료한 pane은 앱 재실행에서도 자동으로 새 대화를 만들지 않는다. live update에서는 종료한 pane의 layout·context·출력 reference를 저장하고 PTY 전달 대상에서 제외한다. 살아 있는 pane은 기존 prepared/commit protocol과 descriptor 검증을 유지한다. Starting 작업은 업데이트 대기 대상으로 포함한다.

pane을 닫으면 앱 전체 세션 manifest를 성공적으로 저장한 뒤 해당 창이 소유했고 어떤 창에서도 참조하지 않는 출력 파일만 정리한다. 다른 파일을 탐색하여 임의로 지우지 않는다. 상속할 terminal 준비가 실패한 GUI는 prepared 승인을 보내지 않아 원래 shell의 복구 경계를 유지한다.

시나리오·단위 검사는 정상/비정상 exit 정보, 동일 pane·cwd·정확한 인수, 중복 클릭, 이전 출력 복사·복원, 누락된 CLI/cwd/transcript, 대화 ID 없음, turn 완료와 shell 생존, stopped handoff를 확인한다. 실제 provider CLI와 업데이트 화면·shell PID 검증은 통합 macOS 검수의 별도 증거다.
