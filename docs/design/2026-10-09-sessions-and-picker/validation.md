# 상세 설계 1.3 문서 검증

2026-10-09, 기준 소스 `8ea18dd10e505bf78c00d98294e02eb4ca940243`.
검증 대상은 설계 본문과 합성 화면 시안이다. 제품 구현, 실제 앱 검수,
사람의 설계 승인, 빌드·배포 전 E2E는 아직 수행하지 않았다.

- GitHub의 열린 이슈 18개를 다시 조회했다. 이 설계는 #181, #182와
  v0.1.22 검수 계약을 다룬다. #173과 #14는 각자의 기록에서 처리한다.
- 사용자가 #182의 네 가지 결정(에이전트 pane마다 한 행, PROJECT 강조,
  Sessions 최대 40%, 둘 다 접혀 있었을 때만 접힘 이전)을 골랐고, 본문과
  시안에 반영했다.
- 첫 검토(커밋 `53601de`)에서 사용자가 결정 2를 바꿨다: “엑티브 세션에서
  누르면 프로젝트 폴더의 특정 브랜치로 포커스 되는 것도 유지해야한다.
  프로젝트의 네비게이터의 포커스가 바뀌는건 괜찮다. 엑티브세션들 사이에서
  네비게이팅 하는것이 힘들어서 추가한 스펙이다.” 이에 따라 PROJECT 강조는
  포커스한 세션의 브랜치를 따라가고 PROJECT 영역 안에서만 최소 이동하며,
  Sessions 영역은 어떤 경우에도 스스로 스크롤하지 않도록 본문·시안·조건을
  고쳤다.
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
| `review-1440.png` | `e169f7d7cffd306aae2180e4db7c17990ad03b95f57c24a51ade3dee9cfee7b2` |
| `review-640.png` | `e225754e5a0636e7355ec9dba80482e780c61fcf0e97330ddb6535bd5729a69b` |
| `review-390.png` | `a04f3240c68689db1288e2b2009dc2786ccc8d16175e6f043dad46dd3c87108a` |

승인 상태는 **사람 검토 대기**다.
