# 상세 설계 1.3 문서 검증

2026-10-09, 기준 소스 `8ea18dd10e505bf78c00d98294e02eb4ca940243`.
검증 대상은 설계 본문과 합성 화면 시안이다. 제품 구현, 실제 앱 검수,
사람의 설계 승인, 빌드·배포 전 E2E는 아직 수행하지 않았다.

- GitHub의 열린 이슈 18개를 다시 조회했다. 이 설계는 #181, #182와
  v0.1.22 검수 계약을 다룬다. #173과 #14는 각자의 기록에서 처리한다.
- 사용자가 #182의 네 가지 결정(에이전트 pane마다 한 행, PROJECT 강조는
  마지막 직접 선택 유지, Sessions 최대 40%, 둘 다 접혀 있었을 때만 접힘
  이전)을 골랐고, 본문과 시안에 반영했다.
- 근거로 인용한 현재 코드를 읽어 확인했다: 사이드바의 단일 `ScrollHandle`,
  `focus_active` → `sync_sidebar_selection(true)` → `select_context`의
  PROJECT 펼침·reveal 예약, Add repo의 `cx.prompt_for_paths` 직접 호출,
  고정된 GPUI의 `NSOpenPanel::openPanel` 호출과 objc2 0.6.4의 NULL panic.
- 공식 문서 링크 3개(objc2 `msg_send!`, Apple `NSOpenPanel`,
  `beginWithCompletionHandler:`)가 HTTP 200으로 열리는 것을 확인했다.
- 공용 Playwright(Chromium 153.0.8010.12)로 `review.html`을 1440px,
  640px, 390px에서 렌더링했다. 세 폭 모두 문서 전체의 가로 넘침이 없다.
  좁은 폭에서는 앱 시안만 자체 가로 스크롤을 쓴다. 390px에서 처음 발견한
  grid 최소 폭 넘침은 열 너비를 `minmax(0,1fr)`로 고쳐 다시 확인했다.
- 렌더링을 직접 읽어 Sessions·PROJECT 두 영역, 상태 아이콘과 문구,
  터미널 행 닫기, 하위 에이전트 행, 높이 규칙 네 가지, 탐색 규칙 표,
  light/dark 좁은 창, Add repo 실패 알림, 설정 이전 표를 확인했다.
  시안 확인은 실제 GPUI 앱 검수를 대신하지 않는다.
- `python3 scripts/sync-product-docs.py --check`와 `git diff --check`:
  통과. Rust와 제품 설정은 바꾸지 않아 Cargo 검증은 수행하지 않았다.

화면 증거는 저장소 밖의 세션별 디렉터리에 보존한다. 다음 hash는 합성
설계 시안의 이미지이며 제품 실행 증거가 아니다.

| 증거 | SHA-256 |
| --- | --- |
| `review-1440.png` | `0f2cae99ab7f84c29a017f99f2084264078e43fb9a639dd5ef6a7a06fdf72be6` |
| `review-640.png` | `9c5e1454f1adc30d958fc166cf645f11a466c87ad7e97fbd0e3c0cfc2e6ebc2f` |
| `review-390.png` | `7ae4d81671a85cdbf1463e909cbe1959e7f29515f3a26af8f7480f0e985de265` |

승인 상태는 **사람 검토 대기**다.
