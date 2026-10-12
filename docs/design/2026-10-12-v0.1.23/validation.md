# 설계 1.0 검증 기록

## 현재 상태

- 설계 원문·시각 시안·인수조건 작성: 완료.
- 정확한 설계 commit 승인: `f58906aa9b158176a42b3b455187f745de34dba5` 사용자 승인 확인. [승인 기록](approval.md).
- 엔진 구현·RED/GREEN·최종 원격 CI: 통과. 교차 코드 리뷰: 공급자 사용량 제한의 해제 시각 대기. 세션 수명 구현: 아래 기록. native 검수·서명 artifact·공개: 미실행.
- 조사 근거: main 및 원격 SHA f09aa62873a4cab100ae8fd3dbba272674a9550e 일치; 최신 public release v0.1.22; 208–231 범위의 open issue 16개; 기존 installed Claude 2.1.296/Codex 0.162.1/Zig 0.15.2.
- 기존 `app/chda/sparkle/update-signing` catalog preflight: active, catalog errors 0. 이는 Developer ID·공증·실제 서명·릴리스 artifact 검증이 아니다.

## 설계 자료 검증

- 로컬 Chromium에서 설계 페이지 상호작용 **16개 검사 통과**. [검사 목록](review-checks.json).
- 단일 클릭 무실행, 더블클릭 1회 열기, 우클릭 무실행, 이전 메뉴 대상 재사용 방지, 배경 보고의 순서·선택 보존을 확인했다.
- 기본 Cancel, Enter 취소, Tab → Stop, Space 확정, Escape 취소·포커스 반환을 확인했다.
- 1280×900 dark / 280px, 640×900 light / 180px 렌더를 생성하고 직접 열어 확인했다. 390×844에서도 문서 자체 가로 overflow가 없음을 확인했다. 최소 640px 앱 시안은 더 좁은 문서 안에서 가로 스크롤한다.
- 시안의 180px sidebar에서 #1/#2가 축소되지 않고 sidebar bounds 안에 있음을 확인했다.
- [Desktop 렌더](review-desktop.png) · [좁은 화면 렌더](review-narrow.png) · [기본 취소 확인창](review-confirmation.png).
- JavaScript error 없음. 이 결과는 **합성 설계 자료**에만 해당하며 실제 chda native·18pt·640×600·제품 대비율 검수는 미실행이다.

## 문서·작업 보존 확인

- `python3 scripts/sync-product-docs.py --check`: 통과.
- `python3 scripts/test-product-docs.py`: 6 tests 통과.
- `git diff --check`: 통과.
- 원래 checkout은 `.serena/` untracked 상태를 그대로 유지; 제품 코드·version·CHANGELOG 변경 없음.

## 구현 시작 전 기준 검사

- 승인 자료 commit에서 fmt 통과, appcast 1 test 통과, 릴리스 정책 38 tests 통과.
- 릴리스 정책의 첫 Swift 실행은 sandbox에서 실패했다. 정상 macOS 도구 접근으로 전체 재실행 후 통과했으며 제품 실패로 분류하지 않는다.
- 이 기준 검사 시점에는 제품 코드를 변경하지 않았다. 이후 결과는 아래에 별도로 기록한다.

## 영역 1 · 터미널 엔진 (#224)

- libghostty-rs `d2036baeb605d31d09a9e2ba6d0c923e04c34d54`, Ghostty `3425025e585a3403340d4dc6d65132f42e05e605`, Zig `0.16.0`을 고정했다. 전역 Zig나 원래 checkout을 바꾸지 않았다.
- chda의 최소 PageList 패치는 rows 증가·reflow에서 새 active 영역에 들어온 viewport를 첫 `grow()` 전에 정상화한다. 진짜 history pin과 Debug 무결성 검사를 보존한다.
- 구현 세부: 준비 소스를 source hash별 경로에 두어 패치 변경 시 upstream native 빌드의 환경 지문도 바뀐다. archive·patch·전체 source digest를 검증하고, 수정된 기존 소스를 덮어쓰지 않는다.
- RED: 기존 0.2.2에서 scrollback → row growth/reflow subprocess가 Debug 무결성 abort. 고정 upstream에 새 직접 테스트만 적용해도 row growth/reflow 사례가 `ViewportPinInsufficientRows`로 abort했다.
- RED: 기존 0.2.2 Ghostty에 upstream recycled-row 회귀를 이식한 격리 검사에서 `!rac.row.wrap`가 실패했다. 이전 API의 `init` 인자 모양만 맞췄다. Rust hard-line 시나리오는 기존 엔진에서도 통과하므로 RED 근거로 세지 않는다.
- GREEN: 최종 pinned source의 직접 resize·reflow·history pin·tracking allocation 실패 및 upstream recycled-row·trimmed-row·viewport-pin 회귀가 Debug에서 통과했다. 광범위 `PageList` 전체 검사는 대용량 max-bytes stress 실행 중 중단했으며 통과로 세지 않는다. CI의 명시적 회귀 목록은 `prepare-ghostty.py --test`에 있다.
- Rust terminal: **64 passed, 2 ignored**. Kitty temporary-file 경계·선택/줄바꿈·이미지·scrollback·snapshot 회귀 포함. ignored 두 건은 기존 성능 측정이다.
- 최종 source 경로의 workspace clippy, fmt, cargo deny 통과. deny의 기존 unused-wrapper 안내와 외부 `block 0.1.6` future incompatibility 안내는 보존한다.
- 전체 workspace 검사: UI **172 passed, 1 failed**. 폭 조절 시나리오의 마지막 오른쪽 drag에서 180px/기대 600px 불일치. 제품 코드를 바꾸지 않은 단독 재실행은 통과했다. 원인은 확정하지 않았으며 원격 전체 CI로 재확인한다. 다른 crate의 검사와 workspace doc tests는 통과했다.
- 실제 준비 소스의 파일을 변경했을 때 Cargo가 source integrity mismatch로 거부했고, 정확한 원문 복원 뒤 Cargo와 source 검증이 통과했다. 미준비 소스도 자동 fallback 없이 빌드를 거부했다.
- 준비 절차 9 tests, 제품 문서 6 tests, 생성 문서 일치, appcast 1 test, 릴리스 정책 38 tests, diff 형식 통과.
- 최종 native font/window resize·복사·실제 이미지와 Linux/Windows release 빌드는 아직 통과 근거가 없다. 0.1.23의 최종 후보 검수와 원격 CI에서 확인한다.

### 첫 원격 CI의 빌드 준비 수정

- Windows: patch integrity mismatch. Git `core.autocrlf=true` 격리 checkout에서 실제 패치 바이트가 CRLF로 바뀌어 실패하는 RED를 확인했다. 해당 패치를 LF로 고정하는 Git attribute 추가 뒤 같은 검사가 통과했다.
- macOS/Linux: 준비 성공 뒤 cache 처리 후 `GHOSTTY_SOURCE_DIR`의 `build.zig`가 없어 빌드 거부. 준비 단계를 Rust cache 뒤로 옮겼다. 릴리스 build 단계도 같은 순서를 적용했다.
- 이 CI 실패는 통과로 세지 않는다. 수정된 최종 HEAD의 전체 CI는 재실행해 확인한다.

### 리뷰 수용 · source 변경 감지와 승인 상태

- Cargo는 지정 directory 아래를 재귀 감시한다. 실제 nested 수정은 기존 guard도 거부했다. 다만 새 root 파일을 추가하면 Cargo가 이전 결과를 재사용하는 별도 RED를 재현했다.
- source root를 감시하도록 바꾸어 root 추가와 nested 수정을 모두 거부한다. 생성 cache 변경은 재검증을 촉발할 수 있지만 source digest에는 포함하지 않는다. 실제 Cargo 회귀 두 건을 CI에 추가했다.
- 원문 commit의 승인 기록을 보존하면서 design·acceptance·시안의 현재 상태 문구를 승인 완료로 맞췄다. 승인된 동작·저장·인수조건 범위는 바꾸지 않았다.

### 두 번째 CI의 준비·검사 수정

- Linux release build 통과. Windows는 Git apply가 전역 autocrlf에 따라 추출한 소스의 줄바꿈을 바꾸어 digest가 달랐다. 전역 `core.autocrlf=true`를 적용한 준비 회귀에서 같은 오류를 재현했고, apply 호출에 `core.autocrlf=false`를 지정해 원래 LF 바이트를 보존한다.
- macOS 전체 UI는 172/173 통과했으며 PROJECT reveal offset 검사에서 -25px/기대 -82px로 실패했다. 폭 조절 사례는 이 실행에서 통과했다. 화면 frame loop가 없는 테스트 플랫폼에서 기존에도 이동 후에만 frame을 전달했으므로, 좌표와 예상 delta를 읽기 전에도 실제 layout frame을 전달하도록 검사 구동을 맞췄다. 기존의 offset·bounds·width assertion은 유지한다.
- 이 실행 역시 전체 통과로 기록하지 않는다. 새 최종 HEAD의 CI와 단독·관련 시나리오 결과를 확인한다.

- 수정 후 로컬 준비 9 tests·실제 Cargo guard 2 tests·관련 sidebar 마우스/스크롤 시나리오 10 tests 통과. 명시적 width·offset·bounds 검사는 유지했다. 전체 최종 CI 결과는 별도로 확인한다.

### source cache 분리와 Kitty 경계 완성

- 세 번째 CI: Windows의 source 준비는 통과했지만 Zig 의존성 다운로드가 HTTP 504로 실패했다. Linux는 Rust cache가 복원한 불완전한 source를 무결성 검사에서 거부했다. 이 결과를 빌드 통과로 세지 않는다.
- 준비 소스를 Cargo target 밖의 `.ghostty-source/<source hash>`로 분리한다. 소스 변조 거부와 기존 source 보존 규칙은 유지하며, cache 안의 파일을 지워 문제를 숨기지 않는다.
- upstream은 명시적 임시 디렉터리 외에도 `/tmp`·`/dev/shm`를 허용한다. 제공된 process dir만 허용해야 하는 직접 Debug 회귀가 수정 전 실패했고, 두 fallback을 제거한 최소 패치 뒤 통과했다. 열린 file handle의 실제 경로 검증·경계·Windows canonical 경로 처리는 유지한다.
- 새 `terminal-integrity.patch`에는 PageList와 이 Kitty 경계 수정만 포함한다. 고정 archive를 깨끗하게 추출해 전체 source hash와 파일별 audit hash를 다시 계산했다. 새 source/hash의 최종 검사를 수행한다.

- 세 번째 HEAD `364afb7`의 macOS CI는 workspace 전체 검사·clippy·Debug 회귀·Cargo guard·updater 컴파일·release build까지 통과했다. Linux/Windows 실패가 있어 PR 전체 통과는 아니다.
- 새 source `9d05e6c187faa7d2`에서 명시적 Debug 회귀·Rust terminal 64 passed/2 ignored·실제 Cargo guard 2 tests·준비 9 tests·제품 문서 6 tests·fmt·생성 문서 일치를 다시 확인했다. 최종 CI 및 교차 코드 리뷰를 확인한 뒤 통합한다.


### 엔진 최종 CI와 교차 리뷰 상태

- PR #234 HEAD `4159cdfe730fbbbfaff17285269cedfe9feaf95b`의 Actions run `38248539602`에서 7개 job 모두 통과했다. macOS workspace tests·clippy·Debug 무결성·실제 Cargo guard·updater 컴파일·release build, Linux/Windows release build, fmt·commit·dependency·문서 검사를 포함한다.
- 읽기 전용 로컬 Claude 교차 리뷰 첫 요청은 weekly limit으로 실행되지 않았다. 공급자 응답의 초기화 시각은 2026-10-12 23:00 Asia/Seoul이다. 시각 이전에 재요청하지 않으며 리뷰 통과로 기록하지 않는다. CI 통과는 최종 native 인수조건이나 공개 릴리스 증거가 아니다.

## 영역 2 · 활동과 세션 수명 (#208, #226, #227, #231)

- RED: 수정 전 실제 PTY를 사용하는 화면 없는 UI 시나리오 네 건이 요구사항으로 실패했다. 테마 변경이 활동 시각을 갱신하고, 복원은 확인된 대화 cwd 대신 오래된 pane cwd를 사용하며, Idle parent의 Working child를 확인하지 않고, 일반 process 종료가 tab을 제거했다.
- 실제 PTY output frame과 성공적으로 전달한 사용자 입력만 활동으로 계산한다. focus 보고·빈 입력·색·크기·스크롤 frame은 제외한다. echo 없는 입력과 화면 이벤트를 terminal/PTY 회귀로 확인했다.
- exact provider+conversation의 cwd를 저장하고 같은 대화의 cwd 없는 보고는 기존 값을 보존한다. 다른 대화는 상속하지 않는다. 보고→캡처 launch→transcript→기존 pane 우선순위를 사용하며 확인된 디렉터리 소실 시 stopped/error로 남긴다. 과거 JSON의 추가 필드 없는 복원도 확인했다.
- `ManagedRun`을 승인된 공통 `PaneRun`으로 일반화했다. 기존 저장 `run`과 state 값은 유지한다. 모든 최상위 process의 종료 상태·private output을 보존하며 cold restore나 입력이 shell을 자동 실행하지 않는다. Start shell과 지원되는 exact Restart만 명시적 실행이다.
- parent Idle에서도 Working/Waiting/Unknown child가 있으면 pane shortcut·Sessions ×·tab × 모두 최신 상태로 확인한다. Cancel 기본, Tab/Shift-Tab 선택, Enter/Space/Escape 소비를 확인했다.
- 최종 변경의 선택 검사: 새 UI lifecycle 9 tests, 기존 managed restart 3 tests, sidebar width/bounds 마우스 1 test 통과. core 69 tests, terminal 65 passed/2 ignored 및 workspace doc tests 통과. 마지막 코드 변경 뒤 workspace clippy·fmt·생성 문서 일치·제품 문서 6 tests·source 준비 무결성·diff 형식을 확인했다.
- 전체 workspace의 두 번째 실행은 UI 180/181 통과했고 마지막 sidebar drag가 실패했다. 검사 입력이 1200px 창 내부를 건너뛰어 1385px부터 시작하는 경로를 확인했다. MouseDown 뒤 frame을 전달하고 100px 이하 간격으로 실제 창 경계를 지나도록 구동을 보완했다. width·offset·bounds assertion은 유지하며 단독 검사는 통과했다. 이 전체 실패를 통과로 표시하지 않으며 최종 HEAD의 전체 CI에서 재확인한다.
- managed cwd 우선순위 변경 후 기존 missing-folder fixture가 낮은 우선순위의 launch cwd만 변경해 기대 오류가 발생하지 않았다. 실제 authoritative report cwd를 사라진 디렉터리로 설정하도록 fixture를 맞췄고 원래 오류·복원 assertion은 유지했다.
- 생성 Pages를 Chromium의 desktop 1280px와 좁은 390px에서 렌더해 직접 확인했다. 13개 feature card, 이미지 로딩·가로 overflow·page error 검사가 통과했다. 이는 Pages의 실행 근거이며 chda native UI 검수 회차에 포함하지 않는다.
- 최종 원격 CI·교차 코드 리뷰·macOS native 검수·GUI update handoff·공개는 아직 통과 근거가 없다. native 검수는 승인된 최종 후보의 3회 계획으로 진행한다.

### 리뷰 수용 · 어댑터별 정확한 transcript 경로

- Copilot은 `<id>/events.jsonl`, Gemini는 `.json`/`.jsonl` 안의 전체 `sessionId`로 대화를 식별한다. 공통 filename 조회는 transcript가 있어도 cwd를 찾지 못했다. 실제 저장 형식을 격리한 두 child-process 회귀가 수정 전 모두 실패했다.
- `session_file_for_id`를 어댑터별로 제공하고 재개 가능 여부와 cwd 조회가 같은 exact 파일을 사용하도록 했다. 기존 Copilot path traversal 거부와 Gemini 전체 ID 비교를 유지한다. 사용자 환경 변수를 변경하지 않고 테스트 프로세스에만 fixture 환경을 전달한다.
- 수정 후 agents 61 passed/기존 수동 검사 2 ignored, doc tests 통과. 새 exact cwd 두 회귀 및 잘못된/부분 ID 거부를 포함한다. 원격 CI와 최종 교차 리뷰는 새 HEAD에서 확인한다.


## 영역 3 · 사이드바 (#209, #216, #228, #230)

- RED: 기존 정렬에서 background hook이 `/a` 위로 `/z`를 이동시키는 core 검사, 실제 마우스의 PROJECT/STARRED 단일 클릭 실행·agent launch sheet·포인터 아래 행 이동·독립 분할 번호 부재 다섯 시나리오와 밝은 테마의 기존 star 대비 실패를 확인했다.
- main worktree 우선, branch 이름순, 동일 이름 path순, branch 없는 항목은 path순으로 뒤에 둔다. 사용되지 않는 활동 정렬 선택을 제거하며 transcript 목록의 최근 활동 정렬과 기존 repository 순서는 유지한다.
- PROJECT/STARRED는 한 번 클릭해 선택만 하고 두 번째 클릭에서 기존 open을 한 번 실행한다. 우클릭은 캡처한 path의 기존 메뉴만 연다. 누락 STARRED의 선택도 표시하며 생성하지 않는다. Sessions는 단일 클릭 탐색을 유지한다.
- 단일 선택 뒤 실제 typing focus를 잃어 `cd`가 전달되지 않는 추가 RED를 확인해 비이동 클릭 뒤 원래 pane의 입력 focus를 돌려준다. 같은 pane의 cwd 보고가 바뀌어도 다른 session focus 전까지 선택을 유지한다.
- 분할 번호는 별도의 nonshrinking #1/#2로 표시한다. 단일 agent 행은 번호를 추가하지 않고 기존 label minimum을 유지한다. mark 색은 기존 status 색 계산을 공유해 hue를 유지하며 normal/selected/waiting/hover surface에서 3:1, diff text는 4.5:1을 요구한다. PR badge palette는 바꾸지 않았다.
- 단계별 GREEN: 새 실제 마우스 시나리오 5개, sidebar 관련 29개(기존 PR palette·status 대비 포함)가 통과했다. 18pt terminal font·640×600·280/180px sidebar의 실제 PTY 기반 bounds 검사에서 분할 번호를 확인했다. synthetic agent가 shell prompt redraw로 종료되는 fixture를 실행 중 process로 맞췄으며 native 검수로 세지 않는다.
- 마지막 source의 전체 workspace·clippy·fmt·문서 검사를 수행한다. 최종 native 화면·변경된 Sessions screenshot·Pages 배치·교차 리뷰와 공개 인수조건은 아직 미실행이다.

- 최종 전체 workspace 실행: core 71, agents 61/수동 검사 2 ignored, terminal 65/기존 성능 측정 2 ignored 및 다른 crate 검사 통과. UI 187/188 통과, shared-row label이 180px에서 0px로 축소되는 기존 readability assertion 실패를 확인했다. 모든 행에 기존 48px label minimum을 유지한 최소 수정 뒤 해당 idle 검사 1개와 새 sidebar 시나리오 5개(번호와 닫기 bounds 포함)가 통과했다. 실패한 전체 실행을 통과로 세지 않으며 최종 HEAD CI에서 전체를 확인한다.
- workspace clippy·fmt·제품 문서 6 tests·생성 문서 일치·diff 형식 통과. CI와 코드 리뷰, native 세 회차는 별도 확인한다.
- 수정 후 agents 61 passed/기존 측정 2 ignored, doc tests 통과. 새 exact cwd 두 회귀 및 잘못된/부분 ID 거부를 포함한다. 원격 CI와 최종 교차 리뷰는 새 HEAD에서 확인한다.

### macOS CI 수용 · 실제 입력과 조용한 프로세스

- lifecycle HEAD `01a8dfa`의 macOS CI는 UI 179/182 통과, sidebar HEAD `79f38d5`는 UI 187/188 통과였다. quiet resize 검사는 실제 zsh의 WINCH 출력 때문에 활동 시간이 달라졌다. lifecycle의 no-echo 입력 검사와 background redraw 검사도 실패했다. Linux·Windows release 빌드와 다른 CI 검사는 두 HEAD 모두 통과했다. macOS 전체 성공으로 세지 않는다.
- 조용한 pane과 redraw 시나리오를 준비 완료 표식을 출력한 뒤 echo 없는 `cat`으로 전환한다. 테마·resize에는 실제 출력이 없고, 원래 timestamp·redraw assertion은 유지한다.
- 준비 출력 직후의 성공한 입력을 검사한 단독 RED에서 활동 시간 대기가 실패했다. 출력 갱신의 기존 1초 제한이 사용자 입력에도 적용되는 실제 누락이었다. 입력은 즉시 기록하고 프로그램 출력만 기존 제한을 유지한다.
- 공유 임시 Cargo 산출물의 다른 worktree 형식 재사용으로 생긴 컴파일 실패는 RED로 세지 않는다. 해당 작업 소유 cache의 core/UI 산출물을 다시 컴파일한 뒤 위 RED를 확인했다.
- 수정 후 lifecycle 9 tests, background redraw 1 test, UI all-targets clippy 통과. 최종 HEAD의 전체 원격 CI와 native 인수 검수는 별도로 확인한다.

## 영역 4 · 메뉴·tooltip·포커스·Setup (#211, #217, #219, #220)

- RED: 활성 PROJECT tooltip을 표시한 채 같은 메뉴 이벤트를 열면 tooltip이 남았다. 메뉴가 열린 동안 기존·새 hint를 숨기고 종료 후 hover를 복구하는 공통 창별 수명 관리로 수정했다. GPUI source·fork는 변경하지 않았다.
- RED: 포인터가 머문 상태에서 section toggle 이벤트를 적용하면 이전 Collapse 문구가 남았다. 토글은 기존 hint를 무효화하며 다음 hover가 현재 Expand/Collapse 상태를 읽는다. 실제 click-only 기준 검사는 수정 전에도 통과했으므로 RED 근거로 세지 않는다.
- RED: Usage popup의 Escape가 닫히지 않았고, launch sheet의 Down → Tab → Enter가 Cancel 대신 시작했다. Usage의 Escape·×는 원래 pane 입력을 복원한다. launch group·Cancel·Start의 Tab/Shift-Tab 이동, 방향키 policy 선택, focused button의 Enter/Space 동작을 구현했다.
- 실제 Star/Refresh/Copy link/Open link/Reveal click 뒤 원래 pane에 고유한 shell 출력이 전달되고 Open terminal은 목적지에 입력되는 시나리오를 추가했다. 입력 복귀 구현을 제거한 격리 mutation에서 Star 이후 입력 대기가 실패했고 정확한 구현 복원 후 통과했다.
- RED: Setup의 tracked source 오류가 상대 entry 없이 `tool exited unsuccessfully`만 표시했다. 항목별 missing/tracked/not ignored/directory/nonregular/symlink/traversal 이유를 표시한다. 열린 parent metadata와 각 component의 no-follow open, regular file handle 검사, 전부 검증한 뒤 복사하는 순서를 보존한다.
- Git index/ignore 판정을 기존 Git CLI와 비교했다. ordinary patterns·negation·nested files·info/exclude·tracked/forced tracked 결과는 같았다. `core.ignoreCase`에서 CLI가 ignored로 허용하는 forced tracked 파일의 대소문자 alias는 같은 tracked index 파일로 판단해 거부한다. 승인된 untracked-only 복사 조건을 유지한다.
- Setup mouse 시나리오의 첫 실패는 overlay가 그대로 있는 상태의 클릭이었다. 비동기 foreground 준비 시트가 만료된 입력 field 대신 자신의 focus를 가져야 함을 확인해 수정했다. Escape로 시트가 닫힌 뒤 badge의 실제 mouse click에서 menu x=20 / badge x=1290의 고정 좌표 RED를 확인했다. 측정한 badge bounds 아래에 메뉴를 붙이고 window edge에서 clamp한다.
- 메뉴 9 tests, Setup UI 8 tests, launch UI 5 tests 통과. Enter·Space의 확인 press/release가 새 raw CLI에 전달되지 않는 실제 PTY 검사를 포함한다. 640×600 Setup bounds 검사 통과. core Setup 11 tests 통과.
- 첫 전체 workspace: UI 192/195 통과. 기존 두 managed restart 검사는 permission group의 Enter를 Start로 가정했고, plain-folder 검사는 열린 메뉴에서 shell 명령을 보냈다. 명시적 Start와 메뉴 Escape를 사용하도록 검사 동작을 맞추며 기존 종료·복원·Git·입력 assertion은 유지했다. 전체 최종 UI·clippy·CI 결과는 아래에 별도로 기록한다.
- native macOS 독립 3회 검수, 새 제품 screenshot과 최종 Pages desktop/narrow 검수, 서명 ZIP 및 공개는 아직 통과 근거가 없다.

- 메뉴 최종 로컬 전체 UI 실행은 193/195 통과, 가짜 forge 테스트 2개 실패였다. 처음 로컬 URL rewrite는 forge 식별도 바꾸어 부적합하므로 제거했다. 원래 URL을 유지하고 검수 저장소의 HTTP proxy만 loopback의 닫힌 포트로 격리한 뒤 두 테스트가 모두 통과했다. 앞선 네트워크 대기 실행은 중단했으며 통과로 계산하지 않는다.
- 기존 sidebar reveal 시나리오의 display-frame 대기를 명시하고 Sessions 실제 접힘을 확인했다. 전체 실행에서 reveal 시나리오가 통과했으며 제품의 reveal 로직은 변경하지 않았다.
- 최종 메뉴 변경의 workspace clippy, fmt, 제품 문서 테스트 6개, 생성 문서 일치 검사가 통과했다. 정확한 PR HEAD의 전체 workspace/세 OS CI 및 최종 native 검수는 별도 확인한다.

### 메뉴 교차 리뷰 처리

- `38d0d35` 교차 리뷰의 이동 실패 입력 유실과 Usage 닫기에서 새로 선택한 pane을 잃는 상태를 각각 RED로 확인했다. 모든 메뉴 명령은 원래 focus를 먼저 복원하고 성공한 동작이 목적지로 옮긴다. Usage는 popup focus를 가진 경우에만 원래 focus를 복원한다.
- 수정 뒤 메뉴 시나리오 11개와 UI clippy 통과. 리뷰는 명령 실행을 하지 않은 source review이며 native 검수를 대체하지 않는다. 실제 source 수정 뒤 최종 CI를 다시 확인한다.
## 영역 5 · 경로와 diff

- 승인 설계 1.0 및 #210/#212에 맞춰 기존 OSC 8/URL/인용/외부 괄호 우선순위를 유지했다. 파일명 안의 괄호는 외부 prose 경계로 해석하지 않고 명시적 시작점의 가장 긴 existing path부터 확인한다. soft wrap만 연결하고 hard newline·인접 explicit path·hard delimiter는 분리한다.
- 실제 수정 전 bare-path logic 2개는 기존 짧은 prefix를 선택하거나 soft wrap의 전체 경로를 찾지 못해 실패했다. 실제 Git 저장소 diff는 UTF-8 파일명을 octal escape로 표시해 실패했다. compile 오류를 RED로 계산하지 않았다.
- 수정 뒤 links 전체 16개, real Git diff 1개, path UI 시나리오 9개 통과. 기존 경로 8개와 새 절대/상대·Unicode·escaped-space·괄호·wrapped 경로의 start/middle/end hover, click, 실제 menu-item 마우스 선택을 검증했다. 표시한 대상을 삭제한 뒤 클릭해 다른 prefix를 열지 않음을 확인했다.
- 새 후보는 최대 16개/확인으로 제한하고 한도 초과 시 legacy cut-prefix fallback을 제외했다. frame generation·cwd·구간 cache를 hover/click/menu에서 재사용하고 action은 같은 대상의 metadata만 재확인한다. 누락된 대상은 해당 cache 결과를 비운다.
- 최초 path UI 실행 8/9는 메뉴 종료 후 새 hover 없이 삭제한 fixture 상태에서 실패했다. 표시된 target을 다시 hover하고 이벤트를 안정화한 실제 삭제-클릭 순서로 보완했으며 해당 실행을 통과로 계산하지 않았다.
- workspace clippy 통과. native #212 검수와 최종 HEAD 전체 CI/Pages 검수는 아직 수행하지 않았다.

### 경로 교차 리뷰 수용

- 첫 읽기 전용 교차 리뷰는 다섯 지적을 반환했다. 커서보다 앞에서 끝난 prefix가 인접 파일을 숨기는 문제, segment 전체의 negative cache, prose 괄호 경로의 잘못된 제외, 파일 생성·재생성 뒤 negative cache를 수용했다.
- RED: `cp /tmp/a.txt README.md`의 뒤쪽 README 대상이 앞 prefix로 해석되는 terminal 회귀가 실패했고, `Cache ./not-yet-created README.md`에서 첫 missing hover 뒤 README를 찾지 못하는 실제 UI 회귀가 실패했다. 수정 뒤 terminal links 19 tests·path UI 10 tests·term/UI all-targets clippy 통과.
- longest-first 후보 16개를 넘는 경우 짧은 token을 자동 선택하자는 지적은 승인된 overflow 무링크 조건과 충돌해 반영하지 않았다. trailing prose도 같은 logical line의 후보 한도에 포함된다는 제한을 CONTRIBUTING에 명시하고 기존 16회 회귀를 유지했다.
- 교차 리뷰는 source 검토이며 실제 macOS 인수조건·최종 원격 CI·릴리스 검수를 대신하지 않는다.
