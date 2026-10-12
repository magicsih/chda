# 설계 1.0 검증 기록

## 현재 상태

- 설계 원문·시각 시안·인수조건 작성: 완료.
- 정확한 설계 commit 승인: `f58906aa9b158176a42b3b455187f745de34dba5` 사용자 승인 확인. [승인 기록](approval.md).
- 엔진 구현·RED/GREEN: 아래 기록. PR code review·원격 CI·native 검수·서명 artifact·공개: 미실행.
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
