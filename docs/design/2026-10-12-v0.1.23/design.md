# chda 0.1.23 — 상세 설계 1.0

- 설계 ID: `2026-10-12-v0.1.23`
- 버전: **1.0**
- 상태: **정확한 설계 버전 승인 대기**. 계획 구현 요청은 수신했으며, 계획에 포함된 설계 버전 승인 단계는 아직 실행하지 않았다.
- 기준: main `f09aa62873a4cab100ae8fd3dbba272674a9550e`, 공개 버전 `v0.1.22`, 2026-10-12 조사.
- 시각 검토: [동작과 화면 시안](review.html). 합성 데이터로 만든 설계 자료이며 실행 제품이나 검수 통과 근거가 아니다.
- 인수조건 정본: [16개 조건과 회귀 범위](acceptance.md). 실행 상태: [검증 기록](validation.md).
- 적용 근거: 사용자가 2026-10-12에 이 계획의 구현과 0.1.23 공개를 요청했다. 아래 설계의 승인 대상은 이 문서·시각 검토·인수조건을 포함하는 정확한 Git 커밋이다.

## 1. 목적·범위와 선택

v0.1.22 실제 검수에서 남은 문제와 사용자가 요청한 활성화·종료 규칙을 함께 해결한다. 208–231 중 실제 미해결 이슈는 16개이며, 나머지 #213, #214, #215, #218, #221, #222, #223, #225는 이미 병합된 PR이다. 병합된 기능은 회귀 검증하며 재구현하지 않는다.

| 순서 | PR 범위 | 인수조건 |
| --- | --- | --- |
| 1 | 엔진 갱신·근본 수정·빌드 준비 | ISSUE-224 |
| 2 | 활동·복원·자식 종료 확인·종료 출력 보존 | ISSUE-208, ISSUE-226, ISSUE-227, ISSUE-231 |
| 3 | 정렬·대비·분할 번호·브랜치 활성화 | ISSUE-209, ISSUE-216, ISSUE-228, ISSUE-230 |
| 4 | tooltip·포커스·키보드·Setup 오류 | ISSUE-211, ISSUE-217, ISSUE-219, ISSUE-220 |
| 5 | diff 파일명·전체 경로 인식 | ISSUE-210, ISSUE-212 |
| 6 | 공유 복사 원문 대응 | ISSUE-229 |

사용자 선택: 영역별 PR을 한 버전에 통합; main 우선·이름순; 파괴적 확인의 기본은 취소; 공유 복사는 원문과 유일한 대응이 있을 때만 보정; 엔진 근본 수정까지 포함하며 chda 안에서 관리; 완료 범위는 공개 후 확인까지.

제외: 다른 미해결 이슈의 신규 구현, GPUI 교체·fork, 새 GitHub fork, 자격증명 교체, 사용자 설치본·저장 초기화, 자동 에이전트 재시작, 새로운 Linux/Windows 제품 기능. Linux/Windows 빌드는 유지한다.

## 2. 화면·상호작용

### 2.1 PROJECT·STARRED와 Sessions

PROJECT는 기존 저장소 순서를 보존하고 각 저장소의 main worktree를 먼저, 나머지는 branch 이름순으로 정렬한다. branch 없는 항목은 경로순으로 뒤에 둔다. 동일 branch 이름은 경로순으로 구분한다. 배경 상태·활동·transcript·Git 갱신은 행 순서, scroll offset, collapse, 선택을 바꾸지 않는다. 생성·삭제처럼 목록 자체가 바뀌면 이름순 위치에 추가·제거한다. 기존 repository drag 순서는 유지한다.

| 동작 | PROJECT·STARRED | Sessions |
| --- | --- | --- |
| 한 번 클릭 | row 선택·강조만 변경; terminal focus·tab·launch sheet는 변경하지 않음 | 기존 pane/tab으로 이동 |
| 더블클릭 | 기존 open 동작을 정확히 한 번 실행; 최근 pane 재사용 또는 설정된 기본 action 및 준비·권한 절차 | 기존 단일 클릭 탐색 유지 |
| 우클릭 | 해당 row의 메뉴만 표시; Open terminal은 shell, 명시적 agent action은 기존 launch sheet | 기존 session menu 유지 |
| disclosure·status·diff·unstar | 고유 동작만 실행; 부모 row 활성화로 전파하지 않음 | 기존 동작 유지 |

클릭 직후 다른 session focus event가 오기 전까지 row 선택을 유지한다. 누락·미체크아웃 STARRED branch는 설명을 표시하고 terminal/worktree를 자동 생성하지 않는다. context menu는 안정된 worktree path/ID를 캡처하며 백그라운드 갱신으로 다른 대상을 실행하지 않는다.

분할 agent가 같은 tab에 여러 개 있으면 `#1`, `#2`를 별도의 축소되지 않는 영역에 표시한다. tooltip은 `Claude Code · pane 2`처럼 전체 의미를 제공한다. 단일 agent tab은 번호를 추가하지 않는다. 공간이 부족하면 label·provider 이름을 먼저 줄인다. 최소 180px, 18pt에서도 번호·핵심 상태·닫기 접근을 유지한다.

star·Git badge는 실제 normal/selected/waiting row 배경에 대해 3:1, `+N −M` text는 4.5:1 대비를 요구한다. 기존 hue family와 PR badge 색은 유지한다. 대비 조정은 기존 색 계산을 재사용한다.

### 2.2 메뉴·tooltip·입력 포커스

공통 context menu 열기 경로에서 활성 tooltip을 즉시 닫고, 열려 있는 동안 바탕 요소의 tooltip 생성을 억제한다. sidebar·tab·terminal·link·title-bar menu를 같은 규칙으로 다룬다. menu 종료 후 hover tooltip이 다시 동작한다. section toggle 클릭은 tooltip을 닫으며 같은 포인터 위치에서 다시 나타날 때 최신 Collapse/Expand action을 표시한다.

Usage popup은 Escape·×로 닫힌다. popup close, Star, Refresh, Copy, Open URL, Reveal처럼 새 pane/tab을 열지 않는 명령은 직전의 유효한 typing pane을 복원한다. 새 pane/tab을 여는 action은 새 대상에 focus를 둔다. pane이 사라졌으면 현재 유효한 focused pane에 복원한다. 배경 결과는 사용자의 새 focus를 빼앗지 않는다.

입력 overlay는 자신만 키를 소비한다. launch sheet의 Up/Down은 permission group 선택을, Tab/Shift-Tab은 group·Cancel·Start 이동을 수행한다. 기본 permission은 CLI configuration이다. Enter/Space는 focused button을 실행하며 permission group의 Space는 해당 option을 선택한다. Start 버튼 활성화 후 press/release 어느 쪽도 새 terminal에 전달하지 않는다.

### 2.3 종료 확인과 종료된 terminal

Working/Waiting parent 또는 Working/Waiting/Unknown child가 있는 parent의 pane/tab 종료는 확인한다. child 상태는 요청 클릭 시 다시 읽으며 provider+parent session과 대상 pane을 정확히 연결한다. Turn complete/Ended child만 있으면 idle parent를 즉시 닫을 수 있다. ×는 유지한다. session row·pane shortcut·tab ×가 같은 종료 판단을 사용한다.

확인에는 parent 상태와 child 개수·상태, 대상 pane을 표시한다. 처음 focus는 Cancel이다. Enter는 기본 취소; Tab으로 destructive button을 선택한 뒤 Enter/Space로 확정; Escape는 취소한다. 대상 ID와 원래 focus를 보존하며 오래된 확인이 replacement tab을 닫지 않는다. Delete worktree도 같은 키 규칙을 사용한다.

프로그램 종료와 terminal 닫기는 분리한다. 살아 있는 shell로 돌아오면 동일 cwd의 shell과 scrollback을 유지한다. 최상위 process가 끝나면 동일 pane/tab/window를 종료 상태로 보존하고 code/signal을 표시한다. 출력은 기존 read-only previous-run viewer에서 scroll·selection·copy할 수 있다. 입력을 살아 있는 process에 보낸 것처럼 처리하지 않는다. 종료 사건이 중복되거나 다른 tab에서 발생해도 대상만 변경한다. 현재 지원되는 정확한 restart/resume만 명시적 동작으로 제공한다. shell이나 agent를 자동 시작하지 않는다.

### 2.4 Setup·경로·공유 복사

Setup menu는 badge의 실제 bounds 아래에 붙이고 640×600에서도 window 안으로 clamp한다. reject message는 사용자 설정의 상대 entry와 이유를 인용한다: missing, tracked, not ignored, directory/nonregular, symlink, traversal/outside root. symlink는 열린 parent의 metadata로 검사하며 no-follow open을 race guard로 유지한다. no-follow 실패도 해당 entry와 이유로 변환한다. 파일 내용이나 credential은 오류에 넣지 않는다. 전체 검증 전에는 복사하지 않는다.

View diff pager는 기존 base·stat/patch·less 옵션을 유지하고 호출에 `core.quotePath=false`를 추가한다. 사용자 Git 설정은 변경하지 않는다.

전체 경로 우선순위: OSC 8 → URL → 인용/균형 괄호 → 새 bare-path 후보 → 기존 token. 새 후보는 `/`, `~/`, `./`, `../`로 시작하며 pointer를 포함하는 같은 logical line의 segment에서 만든다. soft wrap은 연결, hard newline은 분리한다. quotes, tabs, `<`, `>`, `|`와 별개의 명시적 path 시작은 경계다. 내부 공백·괄호·escaped space를 허용하고 기존 line/column·문장 끝 처리 규칙을 재사용한다. 가장 긴 existing candidate를 먼저 선택한다. frame generation·cwd·segment별로 결과를 공유하여 어느 continuation row를 눌러도 동일 target/cells를 얻는다. 새 후보는 최대 16회 filesystem 확인; 후보/경계 한도 초과 시 해당 bare-path의 짧은 prefix로 fallback하지 않는다. click/menu 실행은 그 target의 존재를 재확인하고 다른 경로로 바꾸지 않는다.

공유 복사는 정확한 provider+conversation의 assistant text만 사용한다. 현재 native 검사 대상은 Claude Code 2.1.296·Codex 0.162.1이며 확인된 과거 layout fixture도 유지한다. synthetic answer의 actual PTY/VT capture로 version별 gutter·code-fence 생략을 확인한다. 표시용 projection은 확인된 display gutter와 표시되지 않은 fence delimiter line만 제외하며 source offset을 유지한다. 부분 선택의 대응이 전체 bounded lookup에서 유일할 때 선택된 표시 구간의 원래 들여쓰기를 복원한다. 나머지 answer나 숨겨진 fence를 붙여 선택 범위를 확대하지 않는다. pipe·quote·table·본문 indent·실제 bullet을 일반 정규식으로 제거하지 않는다. unknown layout·ambiguous/incomplete lookup은 exact VT selection을 유지한다. preview는 original selection과 source-match 상태를 제공한다. ordinary Copy와 clipboard-before-confirm 규칙, pane/conversation 재검증은 유지한다. ADR 0016의 whole-answer 제한을 이 정확한 부분 대응 규칙으로 개정한다. 붙여넣기 확인은 전송 없이 수행한다.

## 3. 책임·데이터·빌드 계약

- `chda-term`: plain Rust `Event::Frame`에 output 여부를 추가한다. 실제 `Msg::Output`에서만 output flag를 올리고 frame publish에서 소비한다. UI가 직접 유발한 frame은 활동으로 보고하지 않는다. 실제 terminal input은 기존 input 경로에서 기록한다. VT·graphics·link candidate 생성·최종 output capture를 담당한다. GPUI·gix 타입은 추가하지 않는다.
- `chda-core`: PROJECT 이름순 정렬과 child-aware close 판단에 필요한 domain 값, pane process state·restore schema를 담당한다. 기존 `ManagedRun`을 공통 `PaneRun`으로 일반화하고 이중 종료 상태를 만들지 않는다. 저장 JSON의 `run` 필드 및 state 값은 유지한다.
- `AgentSessionRef`: optional `cwd`를 추가한다. save의 `agent_cwd`와 handoff에 전달하며 `serde(default)`로 과거 JSON을 읽는다. 동일 provider+session의 새 hook에서만 cwd를 갱신한다. 새로운 session은 이전 session의 cwd를 상속하지 않는다.
- restore cwd: 최신 exact-conversation report → captured launch cwd → transcript session cwd → legacy saved pane cwd. missing authoritative directory는 stopped/error pane으로 남기고 재개하지 않는다. managed permission/options는 캡처된 값을 유지하고 resolved cwd를 실제 process와 command에 일관되게 적용한다. 일반 shell의 기존 missing-folder 안내는 유지한다.
- 종료 output의 managed opt-in gate를 제거한다. existing private output 파일·viewer·reference pruning을 모든 process exit에 재사용한다. cold restart는 stopped pane을 다시 실행하지 않는다. session-preserving update는 live PTY만 인계하며 stopped layout/output도 보존한다.
- `chda-ui`: gesture·layout·overlay focus·contrast·read-only view를 담당한다. 특정 OS 코드는 `platform` 밖으로 옮기지 않는다.
- `chda-agents`: provider별 원문 lookup·확인된 표시 projection·fixture를 담당한다. response 본문은 로그·영구 문서에 남기지 않는다.

### 3.1 고정 엔진과 최소 패치

libghostty-rs `d2036baeb605d31d09a9e2ba6d0c923e04c34d54`, Ghostty `3425025e585a3403340d4dc6d65132f42e05e605`, Zig **0.16.0**를 고정한다. upstream code와 API를 가져오며 Rust crate dependency 허용은 해당 URL만 좁게 추가한다.

repo의 작은 patch·manifest와 `scripts/prepare-ghostty.py`가 pinned source archive를 검증·전개해 `target/ghostty-source`를 준비한다. archive SHA-256·upstream commit·patch SHA-256·적용 후 파일 hash를 manifest에 기록한다. 기존 source가 예상과 다르면 덮어쓰지 않고 실패한다. `.cargo/config.toml`의 relative `GHOSTTY_SOURCE_DIR`는 이 task-owned source를 가리키며 임의 환경 source로 바뀌지 않게 고정한다. 준비가 안 된 일반 cargo build는 기본 upstream으로 조용히 fallback하지 않는다. 로컬 안내·macOS bundle·세 OS CI·release 경로가 같은 준비 계약을 사용한다.

원인 수정은 PageList의 resizeWithoutReflow·resizeCols에서 변경된 rows/reflow로 viewport pin이 새 active 범위에 들어올 때 첫 grow 전에 정상화하는 것이다. 유효한 history pin은 보존한다. intermediate grow와 최종 상태의 integrity check를 제거하거나 약화하지 않는다. allocator failure·no-pull/reflow/preserved-cursor 경로도 검증한다. patch는 root correction과 regression test만 포함한다. upstream이 동등한 수정을 포함한 고정 revision으로 바뀌고 회귀 검증을 통과하면 로컬 patch와 필요 없어진 준비 코드를 제거한다.

API 변경은 Terminal::new(cols, rows)·scrollback setter, clipboard bytes callback, optional image data로 적용한다. Kitty temp-file medium은 canonical process temp directory에 제한한다. file/shared-memory medium의 기존 behavior와 storage bound는 유지한다. Zig는 task-local 설치를 사용하고 사용자 전역 toolchain을 교체하지 않는다.

## 4. 검증·문서·승인 추적

각 이슈는 수정 전 실패하는 logic/scenario/native evidence를 먼저 확인한다. [acceptance.md](acceptance.md)의 모든 ISSUE ID를 해당 test 및 실제 native 근거에 연결한다. headless scenario는 실제 화면 통과로 세지 않는다. 버그 재현 subprocess의 abort도 실패로 남긴다.

제품 변경마다 `docs/product-features.json`, 관련 scenario 안내·ADR을 함께 갱신하고 `scripts/sync-product-docs.py`로 README·Pages를 생성한다. 바뀐 screenshot은 실제 제품 실행에서 캡처하며 합성 설계 화면으로 대체하지 않는다. Pages의 desktop/narrow를 검증한다.

각 PR: 최종 HEAD 자체 검토·독립 code review·CI 후 squash merge. 코드/저장 형식·보안 민감 빌드 변경은 교차 회사 리뷰로 검토한다. reviewer request는 PR당 최대 2회, 1500줄 초과는 분할한다. UI 실행 검수는 구현자와 다른 reviewer가 격리 데이터에서 수행하며 3회 시작 전에 각각 계획·expected result·source/config fingerprint·evidence 위치를 기록한다. 세 회차 후 미완료면 추가 회차 승인 전 진행하지 않는다.

실제 native 3회는 release-plz의 최종 0.1.23 candidate tree에서 수행한다. source/assets/version/dependencies/config가 바뀌면 이전 evidence는 최종 후보 통과로 재사용하지 않는다. 모든 기존 catalog feature와 기존 release case를 함께 검수한다.

## 5. 공개·실패·복구

release-plz가 version/CHANGELOG/tag를 생성한다. 정확히 v0.1.23인지 검증한다. version/CHANGELOG를 손으로 바꾸지 않는다. 기존 Developer ID·notarization·Sparkle credential을 먼저 검증하고 재사용한다. credential 교체·신규 발급은 이번 범위에 포함하지 않는다.

release validator에는 0.1.23 이후의 신규 ISSUE case 16개를 추가한다. v0.1.21/v0.1.22의 이미 공개된 artifact/evidence와 cask readback은 당시 case 계약으로 계속 확인할 수 있게 한다. v0.1.23은 정상 schema 1의 모든 pass를 요구하며 이전 버전 exception·skip을 적용하지 않는다.

1. 최종 tag/SHA/tree/config에서 별도 before-build E2E, 60분 freshness 확인.
2. universal build·Developer ID·notarization/staple·Gatekeeper·Sparkle Ed25519를 검증하고 signed draft 보존.
3. 동일 candidate runtime의 별도 before-deploy E2E와 정확한 downloaded ZIP의 signed launch·session-preserving update·GUI IPC·shell PID preservation 검증. 관리자 소유 설치/시스템 승인·취소도 격리된 대상에서 확인.
4. 같은 draft bytes만 publish. 공개 전 freshness 재확인. 재빌드·asset 교체·검증 결과 변조 없음.
5. 인증 없는 독립 download로 hash·appcast·signature·certificate·universal·Homebrew·Pages·GUI/IPC/원래 shell PID를 다시 확인.

과거 v0.1.22에서 실패했던 signed update는 이 후보의 실제 성공 근거가 필요하다. 기존 결함이 gate를 막으면 실패 범위와 root cause를 보고하고 별도 범위 승인 없이 무관한 제품 변경이나 예외 공개를 하지 않는다. public artifact 문제가 생기면 immutable tag를 고치지 않고 검증된 이전 버전 안내와 fix-forward를 사용한다. 배포 중인 feed/cask rollback은 대상과 영향을 확인한다.

완료는 16개 인수조건·정적/시나리오/native/서명 artifact 검증 통과와 공개 후 확인까지다. git/CI/main/draft/published/native-proof 상태를 서로 대체하지 않는다.
