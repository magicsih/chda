# 설계 1.0 검증 기록

## 현재 상태

- 설계 원문·시각 시안·인수조건 작성: 완료.
- 정확한 설계 commit 승인: 대기.
- 제품 구현·RED/GREEN·PR code review·CI·native 검수·서명 artifact·공개: 미실행.
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
