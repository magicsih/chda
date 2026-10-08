# #163 구현 및 소스 검증

2026-10-09. 승인된 설계 1.0의 PREP-163 범위다. 구조 결정은 [ADR 0018](../../decisions/0018-explicit-worktree-preparation.md), 원본 요구는 [#163](https://github.com/magicsih/chda/issues/163)에 연결한다.

- 저장소별 순서 있는 명령·명시적 상대 파일 경로를 편집한다. 기본값은 비어 있고 저장·import는 실행 승인이 아니다.
- 실제 checkout 후 경로·복사/유지·명령을 확인한다. private 승인 파일은 정확한 저장소 identity와 계획에 묶는다.
- untracked gitignored 일반 파일만 독립 복사한다. symlink, 경로 이탈, tracked/nonignored/missing/special file, 기존 destination의 덮어쓰기를 거부한다.
- 복사 후 configured shell 명령을 순서대로 실행한다. 실패·취소는 뒤 단계와 agent launch를 막고 기존 데이터를 남긴다. Retry는 기존 일반 파일을 유지하며 명령 전체를 다시 실행하는 계획을 확인받는다.
- 완료는 현재 tab/focus를 바꾸지 않는다. Open 또는 명시적 Skip 이후 기존 launch options를 사용한다. 다른 창과 MCP도 미완료 준비를 우회하지 못한다.
- 준비 중인 창을 닫으면 즉시 취소 대상으로 삼는다. 종료할 때는 준비 worker가 소유 프로세스를 정리한 뒤 끝낸다. 준비 작업·편집·보류된 결과는 live update를 지연시킨다.

## 검증 기록

| 검증 | 결과 |
| --- | --- |
| private 승인을 분리한 파일 복사 RED → GREEN | 기존 파일 수정 보존 포함, 통과 |
| 가져온 설정의 확인 전 tab 열림 RED → GREEN | 요구 위반을 재현한 뒤 통과 |
| 창이 닫혀도 view 참조가 남는 worker 취소 RED → GREEN | 요구 위반을 재현한 뒤 통과 |
| core 준비 테스트 | 9개 통과 — 저장소 교체, 설정 변경, 파일 접근, FIFO, 순서/실패/취소 |
| 전체 workspace | 통과. 마지막 창 종료 수정 전 144개 UI 테스트 포함 |
| 마지막 창 종료 수정 후 전체 UI | 146개 통과 — 준비 7개 시나리오 포함 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 통과 |
| `cargo fmt --all --check`, product docs generation/check | 통과 |
| product docs / appcast 테스트 | 6개 / 1개 통과 |
| Pages 1440px / 390px 실제 브라우저 렌더링 | 생성된 준비 설명 가독성 확인, 페이지 가로 넘침 없음 |

실행 로그와 Pages 화면은 별도 로컬 검증 디렉터리에 보존한다. 모든 파일 내용·명령 출력 fixture는 합성 데이터이며 실제 credentials를 사용하지 않았다. GPUI 시나리오는 실제 shell/Git 작업을 구동하지만 픽셀을 그리지 않는 테스트 환경이다.

통합 macOS 개발 runtime의 독립 검수 3회, Slack composer 검수, 빌드 전·공개 전 E2E, signed draft 및 공개 artifact 검증은 아직 수행하지 않았다. 이 소스 검증으로 해당 단계를 통과시키거나 #163을 완료 처리하지 않는다.
