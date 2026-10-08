# chda macOS 다음 릴리스 — 상세 설계 1.0

- 설계 ID: `2026-10-08-macos-next-release`
- 버전: **1.0**
- 상태: **사람 검토 대기 — 제품 구현 승인 전**
- 조사 기준: `main`의 `15860466e8f2bce168eebe8e975cbb40084f1382`, 공개 버전 `v0.1.20`
- 승인 범위: 아래 14개 이슈와 이들을 검증·배포하는 절차. 이번 설계 문서를 포함하는 Git 커밋을 승인 대상으로 고정한다.
- 승인자·승인 커밋·근거: 아직 없음. 사용자의 계획 실행 요청은 설계 준비와 다음 macOS 공개 릴리스를 허용하며, 이 문서의 상세 설계 승인은 별도로 받는다.
- 시각 검토: [화면 예시와 구조·상태 흐름](review.html). 합성 데이터로 만든 **설계 시안**이며 실제 구현·실행 검수 결과가 아니다.

## 1. 목적·범위·근거

열린 GitHub 이슈 중 Windows·Linux 플랫폼 지원 #14를 제외한 14개를 완료하고 다음 macOS 버전을 한 번 공개한다. 기존 Linux·Windows 빌드 CI는 유지한다. Rust/GPUI를 유지하며 앱 재작성, 네이티브 채팅, 에이전트 스케줄러, 계정 전환, GitHub 리뷰 게시, 모바일 마켓 출시는 포함하지 않는다.

| 묶음 | GitHub 이슈 | 사용자 결과 |
| --- | --- | --- |
| 경로 | [#164](https://github.com/magicsih/chda/issues/164) | 공백 있는 괄호 경로도 Command-click으로 열기 |
| 사이드바 | [#149](https://github.com/magicsih/chda/issues/149), [#150](https://github.com/magicsih/chda/issues/150), [#151](https://github.com/magicsih/chda/issues/151), [#152](https://github.com/magicsih/chda/issues/152), [#154](https://github.com/magicsih/chda/issues/154) | 안정적인 탐색, 저장소 문맥, 일관된 PR 색상, 터미널 직접 닫기 |
| 사용량·알림 | [#153](https://github.com/magicsih/chda/issues/153), [#155](https://github.com/magicsih/chda/issues/155) | 제어 가까이의 사용량 막대와 놓치지 않는 알림 목록 |
| 실행·상태 | [#159](https://github.com/magicsih/chda/issues/159), [#160](https://github.com/magicsih/chda/issues/160), [#161](https://github.com/magicsih/chda/issues/161) | 명시적 권한 선택, 같은 대화 재시작, 하위 에이전트 활동 |
| 복사·리뷰 | [#158](https://github.com/magicsih/chda/issues/158), [#162](https://github.com/magicsih/chda/issues/162) | 정확한 초안 복사와 줄별 리뷰 메모 전달 |
| 준비 | [#163](https://github.com/magicsih/chda/issues/163) | 새 워크트리에서 필요한 로컬 파일·설정 명령 준비 |

현재 단일 `status_line`은 연속 메시지를 덮어쓴다. 사이드바 탐색은 보이는 행도 위로 정렬한다. `open_diff`는 `git diff`를 pager 터미널에 보여준다. 직접 실행한 자식이 종료되면 UI는 pane을 닫고, shell prompt 복귀 시 대화 ID를 지운다. 설정에는 준비 작업이 없으며 저장 상태에는 실행 옵션·권한이 없다. 이 경계들을 재사용·확장한다.

사용자가 선택한 결정은 **전용 리뷰 탭**, **앱 내 준비 설정 편집**, **실행 중에만 유지하는 알림**, **검증된 직접 전달과 복사·대상 이동 병행**이다.

## 2. 사용자 흐름·화면 상태

### 2.1 사이드바와 사용량

- 첫 수준은 항상 `ACTIVE`, `IDLE`, `PROJECT` 순서다. 비어 있는 섹션도 제목·0개를 표시한다. ACTIVE와 IDLE의 기존 독립 접기·개수·저장 규칙을 유지한다. PROJECT도 독립 접기 상태를 저장하고 명시적 탐색 때만 필요한 저장소를 펼친다.
- `+ repo`는 PROJECT 제목 오른쪽에 둔다. STARRED는 PROJECT 안의 소제목으로 옮긴다. 두 전체 펼침/접기 버튼과 전용 미사용 핸들러는 제거한다. 제목 표시줄·메뉴·`cmd-b`의 사이드바 토글은 유지한다.
- ACTIVE는 에이전트 관련 항목과 일반 터미널 항목을 안정적으로 나눠 표시한다. 실제 terminal tab이며 모든 split pane에 live agent가 없는 행만 일반 터미널이다. 그래프·리뷰 탭은 일반 터미널로 분류하지 않는다. top tab 순서를 바꾸지 않는다.
- 일반 터미널의 `X`는 안정적인 `TabId`로 기존 tab-close 절차를 호출한다. 클릭 전파를 막고 닫기 시 live agent 상태를 다시 확인한다. 배경 tab을 먼저 활성화하지 않는다.
- 완전히 보이는 선택 행은 scroll offset을 그대로 유지한다. 아래·위·부분 잘림은 해당 가장자리까지 최소 이동한다. PROJECT의 현재 저장소 제목을 worktree 영역 위에 고정해 큰 저장소에서도 이름과 선택 행을 함께 보여준다. 배경 출력·refresh는 사용자의 스크롤·접기를 바꾸지 않는다.
- PR 배지는 merged 보라색, open 초록색, closed 빨간색을 사용한다. check 결과는 별도 모양·tooltip으로 표시한다. light/dark Primer 의미 색상을 사용하며 번호·링크·상태 설명을 유지한다.
- Claude/Codex details는 상태줄 위 왼쪽에 붙인다. 창 안으로 크기를 제한하고 내용만 스크롤한다. 알려진 quota마다 0–100% 막대·창 이름·수치를 표시한다. loading/unknown/error를 0%로 만들지 않고 stale 값에는 관측 시각·범위·상태를 표시한다.

### 2.2 알림

아이콘은 제목 표시줄 IDE/app 선택기 바로 왼쪽이며 선택기·사이드바가 없어도 남는다. 클릭 → 최신순 목록 → 닫기 → 이전 terminal focus 복원이다. 항목에는 시각·본문·severity·알 수 있는 repository/tab/pane 문맥을 표시한다.

목록을 여는 순간 존재하는 event ID까지 읽음 처리한다. 열린 뒤 새로 도착한 항목도 읽지 않은 개수에 포함하며, 항목을 선택하거나 다시 열 때 읽음 처리한다. 닫아도 기록은 유지하고 **정상 앱 재시작 시 비운다**. 세션을 유지하는 앱 업데이트는 동일 실행의 연속으로 취급하여 창별 기록과 읽음 기준을 handoff에 유지한다.

운영 메시지는 발생 지점에서 한 번 추가한다. 진행 단계는 동일 operation ID의 항목을 갱신하고 독립 사건은 같은 문구라도 별도 기록한다. Escape는 목록을 닫으며 기록을 삭제하지 않는다. 기존 OS 알림과 provider/resource 상태줄은 유지한다. 알림 수명을 메모리에서 500개로 제한하고 오래된 항목 제거 수를 목록에 표시한다.

### 2.3 권한과 재시작

Claude/Codex launch sheet에서 `Use CLI configuration`을 기본 선택하고 명시적 `Bypass`를 제공한다. Codex에는 approvals와 sandbox가 모두 꺼짐을 바로 표시한다. 실행 전 실제 적용 옵션을 보여주되 상속된 실효 권한은 관측하지 못했으면 `Inherited — effective policy unobserved`로 표시한다.

기존 preset 인수는 보존한다. preset의 명시적 권한과 selector가 충돌하면 시작하지 않고 충돌 옵션을 알려준다. hook/statusline 인수는 별도 결합한다. plain shell과 다른 provider의 기존 실행은 유지하며 manually typed CLI는 변경하지 않는다.

직접 관리한 agent 자식의 정상 종료·crash는 pane을 남기고 exit 정보·`Restart`를 표시한다. 클릭 → `Starting` → 동일 pane/cwd에서 정확한 대화 resume → 성공 또는 복구 가능한 오류다. 반복 클릭은 한 번만 실행한다. 이전 runtime의 출력은 pane의 `Previous run` 읽기 전용 이력에서 확인한다. cwd/CLI/session이 없으면 오류를 보여주며 새 대화로 대체하지 않는다. 정확한 대화 ID가 아직 없으면 재시작을 비활성화하고 이유를 표시한다.

일반 shell exit는 기존 닫기 동작을 유지한다. shell 안에서 직접 타이핑한 CLI는 shell을 교체·종료하지 않는다. 실행 옵션까지 신뢰할 수 있게 기록되지 않은 typed launch에는 재시작 버튼을 제공하지 않는다. 살아 있는 agent의 turn 완료는 process 종료가 아니며 두 번째 프로세스를 시작하지 않는다.

### 2.4 하위 에이전트

확장 가능한 child rows를 정확한 부모 session 아래 표시한다. parent가 IDLE로 이동해도 진행 중인 child는 같은 부모 아래 남으며, parent 행에 active child 개수를 표시한다. 작업·입력 대기·turn 완료·종료·알 수 없음 상태를 구분한다. 종료된 child는 현재 실행의 이력으로만 남고 live count에서 빠진다.

같은 cwd에 있는 다른 conversation/window와 연결하지 않는다. known dedicated pane이 있을 때만 그 pane으로 이동하며 그렇지 않으면 상태 상세만 연다. provider-confirmed 관계·상태가 없으면 unknown을 표시한다. 현재 usage 계산을 그대로 사용하고 child 행의 토큰을 부모 합계에 다시 더하지 않는다. 냉기동 후 지난 transcript만으로 live를 만들지 않는다.

### 2.5 공유용 복사와 리뷰 탭

terminal selection context menu에 `Copy for sharing…`을 추가한다. 선택 pane/session과 출처가 확실한 assistant message를 찾을 수 있을 때 원문을 우선한다. 부분 선택을 전체 답변으로 확장하지 않는다. 대응이 없거나 정리가 애매하면 선택된 rendered text를 미리보기에서 비교·편집한 뒤 복사한다. 일반 Copy는 정확한 기존 동작을 유지한다.

provider/version별 재현 fixture가 확인한 gutter·border만 제거한다. 실제 `|`, 인용, 코드 들여쓰기, Markdown table/list, 링크, CJK/emoji를 보존한다. terminal의 soft-wrap 정보가 있을 때만 화면 줄바꿈을 합친다. v1 clipboard 결과는 plain text이며 rich Slack formatting 변환은 포함하지 않는다.

worktree menu에 `Review diff…`를 추가하고 기존 `View diff` pager를 유지한다. 전용 tab은 파일 목록 + 단일 unified diff + 줄/range 메모 편집을 제공한다. 한 worktree당 한 review tab을 재사용한다. 좁은 창에서는 파일 목록을 접어 diff와 메모의 가독성을 유지한다. binary diff는 메모할 줄이 없음을 표시한다.

메모는 added/deleted line과 rename 전후 파일 identity·base·revision·context를 기록한다. diff refresh 후 유일하게 일치하는 context에만 다시 연결하며 불일치·여러 후보는 `Stale — reattach`로 남긴다. 메모 편집·resolve·전달은 별도 상태다. 전송 성공은 resolve가 아니며 실패해도 메모가 남는다.

`Send review notes…` → 선택 메모·경로·old/new line range 미리보기 → 같은 worktree의 정확한 live target pane/session 선택 → 지원되는 직접 전달 또는 `Copy and go to agent`다. working/approval-waiting/unknown target에 자동 제출하지 않는다. TUI의 작성 중 입력을 지우거나 shell에 인수를 주입하지 않는다. 네이티브 session-addressed 전달이 안전함을 실제 CLI로 확인하지 못하면 직접 전달을 비활성화한다. 직접 전달의 timeout/불확실한 acknowledgement는 재시도 자동 실행 없이 `Delivery unconfirmed`로 표시하여 중복 제출을 막는다.

### 2.6 워크트리 준비

repository menu의 `Worktree preparation…`에서 **명령 순서**와 **복사할 primary checkout 상대 경로**를 편집한다. 기본값은 모두 비어 있다. credentials를 발견하거나 dependency directory를 공유하지 않는다. v1은 명시적으로 선택한 일반 파일만 복사하며 directory·symlink·special file은 설명과 함께 거부한다.

설정 저장 → 새 worktree checkout → 실제 준비 계획 확인 → 파일 복사 → 명령 순차 실행 → agent launch 순서다. 최초 사용·설정 변경 때 명령, 실제 cwd, 각 source/destination을 확인받는다. 아직 검토하지 않은 저장소 설정을 import했다고 명령을 실행하지 않는다. 계획 승인은 repo identity와 정확한 설정 내용에 묶인다.

source는 primary checkout 안의 gitignored이며 untracked인 파일이어야 한다. source/destination directory handle에 제한된 파일 접근을 사용하고 모든 path component의 symlink·`..`·절대 경로·root 이탈을 거부한다. destination은 exclusive create로만 열어 기존 파일을 덮어쓰지 않는다. 복사는 hardlink가 아닌 독립 파일이다.

명령은 새 worktree의 configured shell에서 실행한다. UI를 막지 않고 단계·exit code·원인·retry/cancel/skip을 표시한다. raw stdout/stderr·copied content·환경값을 chda 진단 로그나 notification에 쓰지 않는다. v1 출력은 exit 정보 중심이며 상세 확인은 보존된 worktree에서 사용자가 직접 수행한다.

실패·취소 후 worktree와 이미 생성된 파일은 남긴다. retry 화면은 이미 존재하는 destination을 자동 skip하고 처음부터 명령을 재실행할 범위를 명시·확인받는다. 수정된 파일은 덮어쓰지 않는다. cancel은 준비 process group을 종료·reap하고 agent launch를 차단한다. 사용자가 명시적 Skip을 선택한 경우만 준비 없이 agent를 시작한다. 배경 완료는 현재 tab/focus를 바꾸지 않는다.

## 3. 구조·책임·의존성

| 경계 | 추가·변경 책임 | 유지할 제한 |
| --- | --- | --- |
| `chda-config` | repo별 `PreparationPlan`, PROJECT 접기 설정 | 기존 TOML 관례·unknown 옵션 진단 유지; bypass 전역 기본값 없음 |
| `chda-agents` | `AgentLaunchContext`, `PermissionPolicy`, child activity, assistant content·delivery capability | provider 파싱·인수 결합·bounded helper; prompt/답변을 status hook에 넣지 않음 |
| `chda-git` | structured read-only diff, tracked/ignored 검증 | gix·Git 조회를 UI에 노출하지 않음 |
| `chda-core` | review anchor·notes, 준비 상태·순서, pane 실행/종료 상태, restore 모델 | GPUI/VT/gix 타입 없음 |
| `chda-term` / `chda-pty` | 선택 metadata, 같은 pane 새 runtime과 이전 출력 보존, exit status 전달 | VT/PTY 소유권과 prepare/commit 업데이트 계약 유지 |
| `chda-ui` | 화면·window별 알림·focus·비동기 operation 연결 | GPUI는 이 crate에만 의존 |

예정 구현 순서는 경로 → 사이드바/사용량 → 알림 → 권한/재시작/child → 복사/리뷰 → 준비 → 최종 통합 검수·릴리스다. 묶음별 PR은 Ready로 만들며 관련 범위만 포함한다. release-plz PR은 전체가 준비되기 전 병합하지 않는다.

필요한 구조 변경은 기존 실행/restore/handoff 경계의 확장이다. 무관한 WorkspaceView 분할·provider 재작성은 제외한다. 실제 구현 후 주요 결정은 ADR에 요약하며 설계 본문은 이 위치에서만 유지한다.

## 4. 데이터·인터페이스·실패 처리

| 데이터/인터페이스 | 최소 필드·동작 | 저장·수명·실패 |
| --- | --- | --- |
| `AgentLaunchContext` | provider, 실행 옵션, cwd, 선택 policy, exact session, chda-managed 여부 | pane와 SavedNode에 optional 저장. resume/restore는 기록된 옵션 사용. 로그에 args/env 값 출력 없음 |
| `PermissionPolicy` | `Inherit` / `Bypass`; preset의 explicit 옵션은 충돌 검사 | 새 launch만 Inherit 기본값. 기록된 Bypass는 해당 conversation 복원에만 사용 |
| `AgentActivity` | provider, parent session, child identity, state, event time, optional dedicated pane | app 단위 routing. late/duplicate event로 상태를 되돌리거나 중복 알림을 만들지 않음. 냉기동 live는 미복원 |
| `TabContent::DiffReview` | canonical worktree와 base identity | tab layout에 저장; diff는 다시 읽고 note store를 재사용. 사라진 worktree는 recoverable empty state |
| `ReviewNote` | stable note ID, file old/new path, revision, side·line/range·context, text, open/resolved/stale, delivery result | data dir의 versioned JSON에 atomic save. workspace tree에 파일을 만들지 않음. persistence failure를 표시하고 copy로 보존 가능 |
| `SharingCopyRequest` | pane/session identity, selection range, text·hard/soft wraps, optional message identity | 일회성 preview. 다른 pane 최신 답변으로 대체하지 않음. 내용은 diagnostic logs에 기록하지 않음 |
| `PreparationPlan` | repo-key, ordered shell commands, explicit relative file paths | 기존 config.toml의 repo별 설정. 승인 record는 local data dir. 설정이 바뀌면 다시 확인 |
| `PreparationRun` | stable operation ID, stage, command index, cancellation, result | terminal launch와 같은 worktree-key로 연결. in-flight task는 업데이트 quiescence에 포함 |
| `NotificationEntry` | ID, operation key, timestamp, severity, message, repo/tab/pane context | window 메모리 queue + read-through ID. 정상 재시작은 초기화; live handoff는 보존 |

기존 저장 데이터에서 새 optional 필드가 없으면 Inherit/unknown/빈 기록을 사용한다. 과거 policy를 추측하지 않는다. 기존 단일 창 session reader는 사용자 외부 호환성이라 유지한다. 새 review tab과 launch metadata는 handoff와 backup recovery에서도 serialize/deserialize하여 기존 shell PID를 바꾸지 않는다.

Git diff는 기존 merge-base 기준의 committed/staged/unstaged tracked 변경을 읽는다. untracked 파일은 기존 diff와 동일하게 포함하지 않는다. `--no-ext-diff`, `--no-textconv`로 사용자 외부 diff command를 실행하지 않으며 rename·파일명 parsing은 NUL-safe로 처리한다. 파일이 refresh 중 바뀌면 일관된 revision을 다시 읽고 stale로 표시한다.

path detector는 soft-wrapped logical line의 괄호 경계를 읽어 완전한 후보를 plain token 앞에 둔다. OSC 8 → URL → bounded quoted/parenthesized path → plain token 순서를 유지한다. hard newline이나 별개의 adjacent link는 합치지 않으며 hover·open·context actions가 같은 resolved target을 사용한다.

provider capability 조회는 installed version과 실제 protocol에 묶으며 네트워크 모델 호출 없이 bounded local helper를 사용한다. Codex의 별도 quota app-server가 CLI session을 소유한다고 가정하지 않는다. Claude `SubagentStop`는 child turn 완료로 해석하고 live 종료를 임의 추정하지 않는다. 내부 helper·대기중인 delivery·메모 편집·준비 작업도 업데이트 중 미완료 입력/operation 검사에 포함한다.

## 5. 품질·대안·결정

- 사용자 terminal/home/session과 독립된 test Environment를 유지한다. fixture에만 합성 secret·prompt를 사용한다. 실제 내용은 공개 PR·log·스크린샷에 담지 않는다.
- 파일 접근의 canonical path 검사만으로 동시 symlink 변경에 대한 root confinement을 주장하지 않는다. portable directory capability인 [cap-std `Dir`](https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html)를 준비 복사에 사용한다. 추가 crate의 허용 license·3개 OS build를 검증한다.
- CLI 인수는 argv로 전달하고 shell interpolation으로 이어 붙이지 않는다. 명시적으로 설정한 준비 shell 명령만 shell에서 실행한다.
- 사이드바·child refresh는 stable identity와 기존 event loop를 사용하고 background task마다 redraw하지 않는다. 기존 16-pane background 측정을 릴리스 빌드에서 비교한다.
- 좁은 창은 640×600, 기본 창은 1200×800, 큰 글자는 18pt로 검수한다. 정보를 글자 축소로 숨기지 않으며 popup/list는 창 안에서 스크롤한다. 아이콘 tooltip·severity/상태 텍스트·quota 단위/범위를 제공한다.
- 승인된 대안: pager 교체 대신 별도 review tab; native chat 재작성 대신 capability별 전달; 전역 bypass 대신 launch별 policy; 자동 secret 복사 대신 explicit file list; 재시작 후 알림 저장 대신 실행 메모리 기록.
- 선택 화면의 스타일은 기존 chda와 맞춘다. mockup은 합성 예시이며 새 테마나 제품 전체의 미적 재설계를 하지 않는다.

## 6. 인수조건·테스트·검수

아래 조건에 각 이슈의 원래 acceptance criteria를 함께 적용한다. 각 테스트·PR·검수 보고서에서 조건 ID와 원래 이슈 번호를 참조한다.

| 조건 ID | 통과 기준·필수 실패 사례 | 검증 |
| --- | --- | --- |
| PATH-164 | 공백/Unicode/soft-wrap 모든 위치에서 완전한 같은 경로; 기존 shorter prefix보다 우선; OSC8·URL·quoted/escaped·relative·line/column·hard newline 보존 | detector RED→GREEN + paths/input scenario + 실제 Command-hover/click/우클릭 |
| NAV-149 | fully visible은 offset 동일; above/below/partial은 최소 이동; 큰 repo에서도 이름과 selected row 함께 표시; background는 scroll/collapse/focus 유지 | viewport 시나리오 + 실제 큰 목록 |
| PR-150 | 모든 check 조합에서 open 초록; merged 보라; closed 빨강; 번호·tooltip·클릭 유지 | 색상 모델 회귀 + light/dark 실제 화면 |
| TREE-151 | ACTIVE/IDLE/PROJECT·빈 상태·STARRED·plain folder·count·접기·drag/reorder·restore 보존 | sidebar/folders/starred/reorder/restore 시나리오 |
| REMOVE-152 | bulk 버튼·targets·미사용 전용 코드 제거; +repo/cmd-b/menu/per-repo disclosure 유지 | navigation/menu 시나리오 + 실제 좁은 창 |
| QUOTA-153 | 두 provider에서 왼쪽 popup·막대; 0/near/exhausted·unknown/stale/error·긴 label·resize 정확 | quota scenarios + native 화면 |
| CLOSE-154 | shell last·split live agent·idle·graph/review 분류; stable target; 배경/현재/마지막 tab close; 렌더 이후 agent 시작 재검사 | mixed row 및 closing scenarios |
| NOTIFY-155 | 연속 사건·같은 operation 갱신·동일시각 순서·unread transition·open 중 arrival·empty/long·multi-window·sidebar hidden·Escape focus | queue unit/scenario + 실제 popup |
| COPY-158 | 확인된 장식만 제거; 실제 pipe/table/quote/code/list·CJK/emoji·부분선택·soft wrap 보존; 다른 conversation 혼입 없음 | Claude/Codex 버전별 rendered/copied fixture + clipboard + Slack 작성창 미전송 확인 |
| POLICY-159 | 기본 argv에 bypass 없음; 선택 Bypass mapping; preset 충돌 거부; hook 유지; defaults 변경 후 resume/restore policy 보존 | adapter RED→GREEN + launch/menus/restore scenarios + 실제 argv/화면 |
| RESTART-160 | managed normal/crash 종료·같은 pane/cwd/session/options; 반복 클릭 1회; history; missing CLI/cwd/session 오류; turn 완료와 shell 유지 | process/launch scenarios + 실제 독립 CLI fixture |
| CHILD-161 | parent 종료 turn과 child running/wait/finish/end 분리; shared cwd·multi-window·late event·dedup·no-pane focus·usage once·cold restart | provider fixtures + agents scenarios + 실제 nested rows |
| REVIEW-162 | added/deleted/rename/hunks/range·edit/resolve·stale/ambiguous·미리보기·정확한 대상; busy/approval/unsent input 불변; 실패 후 note 유지 | diff/anchor/dispatch scenarios + 실제 review/delivery |
| PREP-163 | 설정 확인→copy→setup→launch 순서; tracked/missing/existing/symlink/traversal/Unicode 거부·보존; cancel/retry/edit/background focus·로그 secret 부재 | temporary repo RED→GREEN + worktrees scenarios + native 성공/실패/retry |

TDD는 parser·state·identity·argv·filesystem·상호작용에 적용한다. 색상·배치·실제 clipboard paste·실제 provider runtime·서명/공증은 headless scenario가 pixels나 외부 runtime을 증명하지 못하므로 실제 실행 증거를 별도로 남긴다. 테스트 환경 오류를 요구사항 실패로 기록하지 않는다.

### 실제 개발 실행 검수 3회

매 회차 전에 후보 SHA, configuration, test Environment의 격리 저장 경로, 모든 핵심 기능·기대 결과, reviewer identity, 증거 위치를 기록한다. 구현자와 다른 리뷰 에이전트가 실제 자료를 읽고 재현한다. Copilot code review로 대체하지 않는다.

1. 전체 새 기능의 시작→사용→완료·상태 이해와 기존 작업 흐름.
2. 전체 핵심 재실행 + 실패·취소·복구·탐색·640px/18pt·키보드·접근성·light/dark.
3. 전체 핵심 재실행 + 다중 창·split·저장/재시작·세션을 유지하는 업데이트와 기존 기능 회귀.

각 회차에서 발견→수정→문제 재확인을 기록하며 무제한 반복하지 않는다. 3회 뒤 미완료면 추가 회차·범위·시간·비용·중단 영향을 먼저 품의한다. 보고서는 구현 후 `docs/qa`에 작성하며 지금 가상의 통과 기록을 만들지 않는다.

### 전체 기능 E2E

기존 catalog의 Terminal, Tabs/splits, App updates, Title bar, Sidebar, Worktrees, Git history, Agents, Status bar, Palette, Config와 이 설계의 새 동작을 하나 이상의 실제 macOS 개발-runtime 시나리오에 연결한다. 기능 inventory는 소스에 두고 final SHA의 실제 증거는 소스 밖에 보존한다.

검사: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, product-docs/appcast tests와 generated docs check, Linux/Windows release build CI. 새 screenshot과 Pages desktop/narrow도 실제 browser에서 비교한다.

배포용 build 직전과 공개 직전에 각각 전체 E2E를 새로 실행한다. 두 실행의 SHA·설정 fingerprint·OS/CLI 버전·격리 data·시나리오·결과·증거 hash를 남긴다. code/assets/config/dependencies가 바뀌면 무효이며 실행에서 해당 단계 시작까지 60분 이내여야 한다. 미실행·blocked·failure는 통과가 아니다.

## 7. 적용·운영·복구

기능 PR은 issue별 문서 catalog를 실제 구현에 맞게 갱신하고 generated README/Pages를 포함한다. 논리 변경에는 Copilot review를 요청하고 지적·required checks를 처리한다. 버전과 CHANGELOG는 수동 변경하지 않는다. 전체가 통과한 뒤 release-plz가 만든 release PR을 병합한다.

기존 tag→build→publish 한 job은 아래로 분리한다. 추가 승인 버튼을 만들기 위한 분리가 아니라 두 실제 검수의 실행 시점을 보장하기 위한 분리다.

1. tag 생성은 후보 등록만 한다. workflow dispatch의 `build` 단계가 exact tag/SHA와 `before-build` 실제 evidence를 검사한 뒤 기존 universal bundle/sign/notarize를 실행한다. queue 대기로 증거가 60분을 넘으면 새 E2E 뒤 build를 다시 요청한다.
2. build 산출물·checksum·manifest는 **draft GitHub Release**에 보존한다. 이때 public release/latest/appcast/Homebrew는 바꾸지 않는다. draft는 정확한 tag/SHA와 산출물 hash를 기록한다.
3. build 후 candidate SHA의 개발 runtime에서 별도의 `before-deploy` E2E를 수행한다. signed draft artifact의 실제 launch·update도 검증한다.
4. dispatch의 `publish` 단계가 두 evidence의 후보 일치·독립 검수·기능 누락·현재 설정·artifact checksum과 fresh before-deploy를 검사한다. 같은 draft를 공개하고 latest 상태를 확인한 뒤 기존 Homebrew update script를 실행한다. 공개 단계에서 rebuild하지 않는다.
5. 이미 공개한 tag의 재실행은 동일 asset hash를 확인해 중복 publish를 피한다. Homebrew만 실패하면 같은 공개 ZIP으로 cask 갱신만 재시도한다. 기존 자격증명을 교체하거나 budget/ruleset을 우회하지 않는다.

비Seorilabs GitHub macOS 직접 배포이므로 중앙 mobile-market schema에 App Store로 위장하지 않는다. chda용 `macos` evidence verifier를 구현하고 위 global 요구를 검사한다. artifact evidence는 릴리스에 보존하되 private path·prompt·환경값·secret을 공개하지 않는다.

공개 후 exact ZIP SHA-256, Developer ID signature, notarization/Gatekeeper, arm64/x86_64, appcast version/URL/size/signature, latest release, Homebrew version/hash, Pages, 실제 GUI/IPC와 shell PID 보존을 확인한다. 이전 릴리스의 관리자 소유 설치·인증 취소는 별도 미검증 항목으로 시작하며 실제 증거가 생긴 경우만 갱신한다.

심각한 공개 후 결함은 배포된 artifact를 교체하지 않고 기존 signed 이전 버전으로 복구하는 방법을 안내하며 fix-forward release를 준비한다. 잘못된 feed/cask 수정·이전 버전 재지정처럼 외부 공개 상태를 바꾸는 복구는 대상·사용자 영향부터 확인한다. 사용자 worktree·설정·메모를 삭제하지 않는다.

## 8. 승인·추적과 실행 자원

본문 정본은 이 문서다. Wiki는 날짜·버전·상태·정본/issue/PR 링크만 유지하고, 게시할 수 없으면 design index를 사용한다. Obsidian에는 계획/승인/회고와 링크만 기록한다. 본문을 복사하지 않는다.

승인은 **설계 1.0을 포함하는 커밋 + 14개 이슈와 chda 릴리스 검증 범위**에 묶는다. approval evidence와 approved commit은 승인 후 구현 기록에 연결하고 아직 승인되지 않았다고 표시한다. materially 다른 동작/구조/data/acceptance를 구현해야 하면 변경 설계를 먼저 검토받는다.

필요 자원: macOS GUI 개발 runtime, 독립 리뷰 에이전트, 합성 provider/temporary repo fixtures, 현재 Claude/Codex CLI, 미전송 Slack composer, 공유 Playwright, GitHub Actions macOS와 Linux/Windows build, 기존 Apple/Sparkle signing credential과 Homebrew tap 권한. 검증할 수 없는 실제 연동은 blocked로 기록하며 구현 완료로 닫지 않는다.

외부 영향은 `magicsih/chda`와 기존 `magicsih/homebrew-tap`으로 제한한다. Seorilabs 중앙 계약, 모바일 마켓, 프로덕션 서비스 생성은 해당 없음 — chda는 독립 데스크톱 오픈소스 앱이다.

## 공식 근거

- [Claude hooks — child identity와 lifecycle](https://code.claude.com/docs/en/hooks)
- [Claude permissions](https://code.claude.com/docs/en/permissions)
- [Codex app-server — thread identity·상태·child filters](https://learn.chatgpt.com/docs/app-server)
- [Codex CLI options](https://learn.chatgpt.com/docs/developer-commands?surface=cli)
- [GitHub Primer StateLabel](https://primer.style/product/components/state-label/)
- [Slack message formatting](https://slack.com/help/articles/202288908-Format-your-messages-in-Slack)
- [cap-std 4.0.3 directory capabilities](https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html)
- 기존 코드와 [testing](../../testing.md), [session-preserving updates](../../decisions/0011-session-preserving-updates.md)
