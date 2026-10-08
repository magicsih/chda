# 0013 — 대화별 실행 옵션과 권한

승인 근거: [macOS 다음 릴리스 설계 1.0](../design/2026-10-08-macos-next-release/design.md), #159.

Claude Code와 Codex의 관리 실행은 실행 창에서 권한을 선택한다. 기본값은 CLI 설정 상속이며, 실효 권한은 확인하지 못했음을 표시한다. Bypass는 명시적으로 선택해야 하며 Codex에서는 approvals와 sandbox를 모두 끈다. preset은 수정하지 않고 충돌 옵션이 있으면 실행을 중단한다. hook와 status 설정은 권한 옵션과 별도로 결합한다.

`AgentLaunchContext`는 실제 실행 파일, cwd, 원본 preset 옵션, 선택한 권한과 provider의 정확한 대화 ID를 보관한다. `agent-launches.json`은 provider와 대화 ID를 함께 키로 사용한다. 대화 ID가 아직 없거나 사용자가 shell에서 직접 실행한 명령은 신뢰할 수 있는 기록으로 저장하지 않는다. 명령행이나 대화 본문을 진단 로그로 복사하지 않는다.

재개 시 기록이 있으면 새 preset이나 PATH로 대체하지 않는다. 세션 복원과 세션 보존 업데이트에도 같은 context를 전달한다. 손상된 기록은 오류로 표시하며 빈 기록으로 덮어쓰지 않는다. 앱 소유 기록 파일은 같은 디렉터리의 독점 생성 임시 파일에 쓰고 원자적으로 교체한다. Unix에서 파일 권한은 0600이다.

전역 bypass나 preset 자동 변경은 대화 간 권한을 섞거나 사용자가 선택한 제한을 삭제하므로 사용하지 않는다. UI 시나리오는 기본값, 명시적 선택, preset 충돌, 변경된 preset 이후의 재개·복원을 검증한다. 실제 CLI 권한 화면과 재개는 통합 macOS 검수에서 별도로 확인한다.
