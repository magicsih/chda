# 추가 설계 1.1 승인 기록

- 승인 일자: 2026-10-09
- 승인자: 사용자
- 승인 원문: “승인한다..”
- 승인 대상: 추가 설계 1.1, 커밋 `693da23543eb0fecba548e06a925c867f5a1f42c`
- 정본: [설계 본문](design.md), [시각 검토](review.html)
- 실행 티켓: [#173](https://github.com/magicsih/chda/issues/173). #161의 exact parent/child 계약은 유지한다.
- 범위: pane별 private relay와 provider proxy를 이용한 일반 Codex 공유 서버 연결, exact thread/turn 상태, shell/menu/resume 연결 및 소유 자원 정리.
- 채택 조건: 기존 설정·승인·sandbox·managed authentication·user notify의 의미를 실제 실행으로 대조하고 보존할 때만 일반 실행에 활성화한다.
- 실패 조건: 보존에 실패하면 제품 연결을 활성화하지 않고 확인된 차이와 대안을 #173에 기록한다. 다른 연결 구조나 보존 계약 변경은 재검토한다.

기존 설계 1.0의 14개 이슈와 다음 macOS 공개 릴리스 승인은 유지한다.
이 승인은 실제 macOS 검수 3회, 빌드 전 및 공개 전 E2E, 서명된 산출물 검증을
통과했다는 뜻이 아니다. 아직 실행하지 않은 검증은 통과로 기록하지 않는다.
