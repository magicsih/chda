# Sessions 사이드바와 폴더 선택 창 — 상세 설계 1.3

- 설계 ID: `2026-10-09-sessions-and-picker`
- 버전: **1.3**
- 상태: **설계 1.3 승인 — 구현 진행**. 승인자 사용자, 승인 커밋 `371f938`, 2026-10-09. [승인 기록](approval.md)
- 조사 기준: `main`의 `8ea18dd10e505bf78c00d98294e02eb4ca940243`, 공개 버전 `v0.1.21`
- 승인 범위: [#181](https://github.com/magicsih/chda/issues/181), [#182](https://github.com/magicsih/chda/issues/182)의 구현과, 원래 14개 이슈를 포함한 v0.1.22의 검수·공개 계약. 이 문서를 포함하는 Git 커밋을 승인 대상으로 고정한다.
- 시각 검토: [화면 예시와 흐름](review.html). 합성 데이터로 만든 **설계 시안**이며 실제 구현이나 실행 검수 결과가 아니다.
- 연결: [설계 1.0](../2026-10-08-macos-next-release/design.md), [범위 1.2 승인](../2026-10-09-release-scope/approval.md), [v0.1.21 예외 기록](../../releases/v0.1.21-native-input-exception.md)

## 1. 목적·범위·근거

| 이슈 | 사용자 결과 | 현재 원인 |
| --- | --- | --- |
| [#181](https://github.com/magicsih/chda/issues/181) | Add repo에서 macOS가 선택 창을 만들지 못해도 앱과 세션이 살아 있고, 짧은 안내 뒤 다시 시도할 수 있다 | GPUI가 분리 실행한 foreground 작업이 `NSOpenPanel::openPanel`을 호출한다. objc2-app-kit 0.3.2는 반환값을 non-null로 선언하고, objc2 0.6.4는 NULL을 받으면 panic한다. 호출하는 쪽에서는 이 panic을 잡을 수 없다 |
| [#182](https://github.com/magicsih/chda/issues/182) | 세션을 눌러도 세션 목록이 다른 곳으로 튀지 않는다. 세션 목록과 프로젝트 목록을 따로 스크롤하고, 누른 세션의 브랜치는 PROJECT 안에서만 강조·표시한다 | ACTIVE·IDLE·PROJECT가 `ScrollHandle` 하나를 공유한다. 세션 클릭이 `focus_active` → `sync_sidebar_selection(true)`를 거쳐 PROJECT를 펼치고 워크트리 reveal을 예약한다 |
| v0.1.22 | 원래 14개 이슈(#149–#155, #158–#164)와 위 두 이슈를 실제 macOS 검수를 통과한 뒤 공개한다 | v0.1.21은 자동 입력이 막혀 실제 검수 0회로 공개됐다. 예외 기록이 미완료 이슈를 닫지 않기로 정했다 |

사용자가 2026-10-09에 고른 결정:

1. Sessions의 한 행은 **에이전트 pane 하나**다. 일반 터미널과 Git 트리·리뷰 탭은 탭 하나가 한 행이다.
2. PROJECT 강조는 **현재 포커스한 세션의 브랜치를 따라간다**. 세션을 누르면 PROJECT가 그 브랜치를 강조하고, 화면 밖이면 PROJECT 영역 안에서만 최소로 움직인다. 세션 목록 사이를 오가기 어려운 문제를 풀려는 것이 #182의 목적이므로, 움직이지 않아야 하는 것은 Sessions 영역이다(검토 의견으로 처음 선택을 바꿈).
3. Sessions 높이는 **내용만큼, 사이드바의 최대 40%** 다.
4. 예전 ACTIVE·IDLE 접기 설정은 **둘 다 접혀 있었을 때만** Sessions를 접힌 상태로 이전한다.

제외: Windows/Linux 기능 #14(하위 이슈로 분해), Codex 공유 서버 경고 #173(0.162.0 재검증은 별도 기록), GPUI fork·`[patch]`, 다른 사이드바 재설계.

## 2. 사용자 흐름·화면 상태

### 2.1 Sessions 목록

- 사이드바 첫 수준은 **Sessions**와 **PROJECT** 둘이다. ACTIVE·IDLE·Idle Agents라는 제목은 없어진다.
- Sessions 제목 줄: 접기 화살표, `Sessions`, 개수, 기존 Branch/Alias 전환. 접혀 있어도 개수를 보여 준다.
- 행 종류:
  - **에이전트 행** — 에이전트가 살아 있거나 chda가 실행한 기록이 있는 pane마다 한 행. split 안의 에이전트도 각자 한 행이다. 같은 탭에 에이전트 행이 둘 이상이면 보조 문구로 `Codex · pane 2`처럼 표시한다.
  - **보기 행** — Git 트리, diff 리뷰처럼 터미널이 아닌 탭마다 한 행.
  - **터미널 행** — 에이전트가 없는 터미널 탭마다 한 행. 목록 맨 아래에 두고 `✕`로 그 탭만 닫는다(#154 유지).
- 순서: 에이전트·보기 행을 탭 순서와 탭 안의 pane 순서로, 그 뒤에 터미널 행을 탭 순서로. **상태가 바뀌어도 행이 움직이지 않는다.** 대기 중인 에이전트가 두 번 나오지 않는다.
- 상태 표시: 기존 아이콘과 색(작업 중 회전 고리, 입력 필요 주황 `!`와 옅은 행 배경, 턴 완료 초록 점, 대기 노란 점, 에이전트 없음 회색 점)을 유지한다. 색만으로 구분하지 않도록 아이콘과 행 tooltip에 `Working`, `Waiting for input`, `Turn complete`, `Idle`, `No live agent`, `Terminal`, `Git tree`, `Diff review`를 보여 준다.
- 대기 중인 에이전트 행과 터미널 행에는 닫기 버튼이 있다. 기존 닫기 확인 절차를 그대로 탄다. 대기 에이전트 행은 그 pane을, 터미널 행은 그 탭을 닫는다.
- 하위 에이전트(#161)는 부모 에이전트 행 아래에 그대로 펼친다. 부모가 대기로 바뀌어도 같은 행 아래에 남는다.
- 비어 있으면 `No open sessions`를 보여 준다.

### 2.2 두 영역과 높이

```text
┌ Sessions 5 ─────────── Branch ┐  ← 제목은 스크롤 밖
│ ◌ app / feature-a     Claude  │
│ ● app / feature-a  Codex·pane2│  ← Sessions 스크롤 (최대 40%)
│ ! web / fix-login     Claude  │
├ PROJECT ─────────────── + repo┤  ← 제목은 스크롤 밖
│ app            (고정 헤더)     │
│   feature-a                   │  ← PROJECT 스크롤 (남은 높이)
│   main                        │
└───────────────────────────────┘
```

- Sessions는 내용 높이만큼 커지다가 사이드바 높이의 40%에서 멈추고 그 안에서 스크롤한다. PROJECT는 남은 높이를 모두 쓴다.
- PROJECT를 접으면 Sessions가 남은 높이를 모두 쓴다. Sessions를 접으면 PROJECT가 제목 바로 아래부터 시작한다.
- 낮은 창(높이 360px)에서도 세션 약 5행과 PROJECT가 함께 보인다. 두 제목은 스크롤 밖에 있어 접기와 `+ repo`에 늘 닿을 수 있다.
- 휠·트랙패드는 포인터 아래의 목록만 움직인다. 한쪽 끝까지 스크롤해도 다른 쪽이 움직이지 않는다.
- 저장소 드래그 자동 스크롤과 고정 저장소 헤더(#149)는 PROJECT 영역 안에서만 동작한다. 사이드바에 폴더를 끌어다 놓아 추가하는 동작은 두 영역 어디서나 된다.

### 2.3 선택과 탐색

| 사용자 동작 | 터미널 포커스 | Sessions 강조·스크롤 | PROJECT 강조·스크롤 |
| --- | --- | --- | --- |
| Sessions 행 클릭 | 그 pane/탭 | 그 행, **스크롤은 그대로** | 그 브랜치 강조. 화면 밖이면 PROJECT 안에서만 최소 이동, 접힌 저장소는 펼침 |
| cmd-1…9, 탭 클릭, pane 이동, split | 바뀐 곳 | 바뀐 곳, 스크롤 그대로 | 위와 같음 |
| cmd-shift-a(기다리는 에이전트로), 알림 클릭 | 그 pane | 그 행, 스크롤 그대로 | 위와 같음 |
| 하위 에이전트·리뷰에서 pane으로 이동, resume·restore | 그 pane | 그 행, 스크롤 그대로 | 위와 같음 |
| PROJECT 워크트리 클릭, 상태 점 클릭, STARRED 클릭, 팔레트 Go to worktree, 워크트리 생성 직후 여는 탭 | 그 워크트리의 최근 pane | 그 행, 스크롤 그대로 | 위와 같음. PROJECT 섹션이 접혀 있으면 펼침 |
| 배경 출력, 상태 변화, refresh, 하위 에이전트 갱신, 다른 창의 MCP 요청 | 그대로 | 그대로 | 그대로 |

- PROJECT 안의 이동은 #149 규칙을 따른다. 이미 완전히 보이면 offset을 바꾸지 않고, 위·아래로 잘렸으면 그 가장자리까지만 움직이며, 저장소 이름을 고정 헤더로 함께 보여 준다.
- 세션 클릭·탭 전환은 PROJECT **섹션**을 접어 둔 사용자의 선택은 바꾸지 않는다. 강조만 갱신되고, 펼치면 그 브랜치가 보인다. PROJECT 쪽에서 직접 탐색할 때만 섹션을 펼친다.
- **Sessions는 스스로 스크롤하지 않는다.** 사용자가 보고 있던 세션 목록의 위치는 클릭·탭 전환·PROJECT 탐색·배경 갱신 어느 경우에도 그대로다. 이것이 #182의 핵심 요구다.

### 2.4 Add repo

- 사이드바 `+ repo`, 메뉴 Add Repository…, `cmd-shift-o`, 팔레트 Add repository가 모두 같은 흐름을 탄다.
- 성공: 기존처럼 폴더를 여러 개 고를 수 있고, 저장소·일반 폴더 등록과 `git init` 제안이 그대로다.
- 취소: 아무 메시지도 남기지 않는다.
- 실패(macOS가 선택 창을 만들지 못함): 앱은 계속 실행되고, 제목 표시줄 알림 기록에 오류 한 건이 남는다.
  `Couldn't open the folder picker. Try Add repository again, or drop the folder on the sidebar.`
  터미널 포커스와 입력 중인 글자, 설정, 세션은 바뀌지 않는다. 바로 다시 시도할 수 있다.

## 3. 구조·책임·의존성

| 경계 | 추가·변경 책임 | 유지할 제한 |
| --- | --- | --- |
| `chda-core` | 새 `sessions.rs`: `SessionKey`, `SessionKind`, `SessionRow`, 행 생성·정렬·focus 계산. `Sidebar.active_tabs`·`idle_agents`·`ActiveTab`·`IdleAgent` 제거 | GPUI/VT/gix 타입 없음 |
| `chda-config` | `sessions-collapsed` 하나로 통합, 예전 두 키 읽기 | 기존 TOML 관례, unknown 키 무시, 다른 설정 보존 |
| `chda-ui/sidebar_view` | Sessions·PROJECT 두 스크롤 영역(`sessions_scroll`, `project_scroll`). reveal·고정 헤더·드래그 자동 스크롤은 `project_scroll`만 사용 | 다시 그리기 범위(#134) 유지 |
| `chda-ui/workspace_view` | 포커스 동기화가 PROJECT 강조·reveal을 계속 요청하되 PROJECT 섹션은 직접 탐색에서만 펼침, `add_repo`가 `System::pick_folders` 사용 | 터미널 포커스·세션 저장·업데이트 operation 규칙 유지 |
| `chda-ui/platform` | `FolderPick`, `pick_folders`; macOS는 NULL을 받을 수 있는 생성과 `beginWithCompletionHandler` | macOS 코드는 `platform/macos.rs`에만, Linux/Windows는 GPUI `prompt_for_paths` 유지 |
| `chda-ui/environment` | `System::pick_folders` 추가, `RecordingSystem`에 준비한 응답 | 시나리오는 OS 입력과 외부 경로를 쓰지 않음 |

구현 순서는 #181 PR → #182 PR이다. 둘 다 Ready PR로 열고 Copilot 리뷰를 요청한다. 검수 조건 ID 추가(`scripts/macos-release.py`)는 #182 PR에 함께 넣는다.

## 4. 데이터·인터페이스·실패 처리

| 데이터/인터페이스 | 최소 필드·동작 | 저장·수명·실패 |
| --- | --- | --- |
| `SessionKey` | `Pane(PaneId)` 또는 `Tab(TabId)` | 화면 수명. 상태가 바뀌어도 같은 키 |
| `SessionRow` | key, tab, kind, agent, 살아 있음 여부, 상태, since, 제목, branch, 저장소, cwd, 탭 제목, pane 번호, 활동 시각 | 저장하지 않는다. `sync_panes`가 다시 만들고, 활동 시각만 바뀌는 경우는 순서·notify 없이 갱신 |
| Sessions 강조 | 현재 focus `(TabId, PaneId)` → 그 pane 행, 없으면 그 탭의 첫 행 | 저장하지 않는다 |
| PROJECT 강조 | 현재 포커스한 pane의 워크트리 경로 | 저장하지 않는다. 워크트리가 사라지면 강조 없음. 포커스가 바뀔 때만 reveal을 예약하고 배경 refresh는 예약하지 않는다 |
| `sessions-collapsed` | bool | `config.toml`. 키가 없고 예전 `active-collapsed`·`idle-agents-collapsed`가 모두 true면 true, 아니면 false. 읽을 때 파일을 다시 쓰지 않고 다음 저장 때 예전 키가 사라진다 |
| `FolderPick` | `Chosen(Vec<PathBuf>)`, `Cancelled`, `Unavailable(String)` | 일회성. 채널이 끊기면 `Cancelled` |
| `System::pick_folders(prompt, cx)` | 선택 창을 열고 결과를 future로 돌려준다 | macOS는 foreground 작업으로 한 번 미룬 뒤 연다. App을 빌린 상태에서 AppKit 콜백이 다시 들어오는 일을 막기 위해서다 |

폴더 선택 창 생성은 `msg_send![NSOpenPanel::class(), openPanel]`을 `Option<Retained<NSOpenPanel>>`로 받는다. objc2 0.6의 `msg_send!`는 nullable 반환을 지원한다. NULL이면 panic 대신 `Unavailable`이다. 생성에 성공하면 GPUI와 같은 옵션(디렉터리만, 여러 개, 폴더 만들기 허용, prompt)으로 `beginWithCompletionHandler`를 호출한다. `crates/chda-ui/Cargo.toml`에는 지금 zed의 feature 통합으로만 켜져 있는 `NSOpenPanel`, `NSSavePanel`, `NSPanel`, `block2`, `NSArray`, `NSEnumerator` feature를 명시한다.

업데이트 operation token은 Add repo 작업이 끝날 때 drop되므로 성공·취소·채널 끊김·실패 모두에서 풀린다. "선택 창 열림" 플래그는 두지 않는다. 플래그가 걸린 채로 남으면 재시도를 막기 때문이다.

v0.1.21로 되돌리면 `sessions-collapsed`는 무시되고 ACTIVE·IDLE은 펼친 상태로 시작한다. 데이터 손실은 없다.

## 5. 품질·대안·결정

- 접근성: 상태를 색만으로 알리지 않는다(tooltip 문구). 닫기 버튼에는 `Close terminal tab`, `Close agent pane` tooltip을 둔다. 큰 글자 18pt와 640×600 창에서 두 영역의 제목·행이 잘리지 않아야 한다.
- 성능: 출력이 계속 나오는 배경 pane이 사이드바를 다시 그리지 않는다(#134 측정 유지). 행 생성은 탭·pane 수에 비례한다.
- 채택하지 않은 대안:
  - GPUI fork·`[patch]`·Zed 갱신: Zed main에도 NULL 처리가 없고, fork는 유지 비용이 크다.
  - `catch_unwind`로 panic 복구: GPUI가 분리 실행한 작업 안이라 잡을 수 없고, 이슈가 panic 복구에 기대지 말라고 한다.
  - 탭마다 한 행 + pane 하위 행: 하위 에이전트까지 겹치면 3단 들여쓰기가 된다(사용자 결정 1).
  - 세션 클릭 때 PROJECT를 그대로 두기: 처음 고른 안이었으나, 세션의 브랜치를 PROJECT에서 바로 확인하는 흐름이 필요하다는 검토 의견으로 바꿨다(사용자 결정 2).
  - 끌어서 조절하는 구분선: 구현·검수 범위가 커진다. 필요해지면 따로 제안한다(사용자 결정 3).
- 화면 스타일은 기존 chda를 따른다. 새 테마나 전체 재디자인은 없다.

## 6. 인수조건·테스트·검수

이슈의 원래 acceptance criteria를 함께 적용한다. 조건 ID를 PR·검수 보고서·이슈 종료 코멘트에서 참조한다.

| 조건 ID | 통과 기준·필수 실패 사례 | 검증 |
| --- | --- | --- |
| PICKER-181 | 생성 실패에서 앱 생존, 오류 알림 1건, 입력·포커스·설정·세션 불변, 즉시 재시도 성공. 취소는 무소음. 여러 폴더 선택·일반 폴더 등록 유지. 네 진입점이 같은 흐름 | 실패하는 시나리오 → 수정 + logic 테스트 + 실제 macOS 선택 창 성공·취소 + 실제 생성 실패 재현 |
| SESSIONS-182 | Sessions 한 제목, 에이전트 pane마다 한 행, 대기 중복 없음, 상태 변화에도 행 고정, 터미널 행 맨 아래·정확한 닫기, 하위 에이전트 유지, 두 영역 독립 스크롤·40% 상한, 세션 클릭·탭 전환이 Sessions offset을 바꾸지 않고 PROJECT 안에서만 그 브랜치를 강조·최소 reveal, PROJECT 섹션 접힘 유지, PROJECT 탐색이 Sessions를 움직이지 않음, 접기 설정 이전 | `scenarios/sessions.rs` 6개 + `reorder` 확장 + core·config 단위 테스트 + 실제 넘치는 두 목록·낮은 창·light/dark |
| TREE-151 (개정) | 첫 수준은 Sessions·PROJECT. PROJECT 아래 저장소·일반 폴더·STARRED, 접기 저장, 드래그 정렬 유지. **ACTIVE/IDLE 분리 요구는 #182가 대체** | sidebar·starred·reorder·folders 시나리오 + 실제 화면 |
| NAV-149 (개정) | 세션 클릭·탭 전환·PROJECT 탐색 모두 PROJECT 안에서 보이는 행은 그대로, 잘린 행만 최소 이동, 고정 저장소 헤더. **공유 스크롤로 세션 목록이 함께 움직이던 부분은 #182가 대체** | 세션·PROJECT 탐색 시나리오 + 실제 큰 목록 |

나머지 12개 조건(PATH-164, PR-150, REMOVE-152, QUOTA-153, CLOSE-154, NOTIFY-155, COPY-158, POLICY-159, RESTART-160, CHILD-161, REVIEW-162, PREP-163)은 설계 1.0 그대로다. CLOSE-154는 Sessions 안에서 같은 기준을 적용한다.

테스트(TDD):
- #181: `folders::add_repository_survives_an_unavailable_picker_and_retries`가 수정 전에 실패한다(`add_repo`가 GPUI를 직접 불러 준비한 실패를 소비하지 않음). `platform::tests::path_prompt_results_map_to_folder_picks`, `platform::macos::tests::missing_open_panel_reports_unavailable`.
- #182: `sessions_and_project_scroll_independently_when_both_overflow`, `session_clicks_keep_sessions_still_and_reveal_only_in_project`, `status_changes_keep_session_rows_in_place`, `project_navigation_does_not_scroll_sessions`, `closing_sessions_keeps_both_regions_in_place`, `sessions_collapse_persists_and_migrates_legacy_keys`. 새 규칙과 반대를 확인하던 `active_labels_and_clicks_follow_split_focus_across_branches`, `agent_sections_collapse_independently_and_persist`는 지운다.

#181의 실제 생성 실패 재현:
1. 실행 중인 개발 빌드의 실행 파일을 다른 빌드로 바꾼 뒤 Add repo를 누른다. 보고된 사고와 같은 조건이다.
2. 재현되지 않으면 **debug 빌드에서만** 컴파일되는 실패 주입(`CHDA_QA_FOLDER_PICKER=unavailable`)으로 `open_panel`이 `None`을 돌려주게 한다. 릴리스 빌드에는 들어가지 않는다.

### 실제 개발 실행 검수 3회 (v0.1.22)

후보는 release-plz가 만든 v0.1.22 PR의 최종 tree다. 매 회차 전에 후보 SHA, 설정, 격리 저장 경로, 16개 조건과 기대 결과, 검수자(구현자와 다른 리뷰 에이전트), 증거 위치를 기록한다.

1. 16개 조건 전체의 시작→사용→완료와 기존 작업 흐름
2. 실패·취소·복구, 640×600·18pt, 키보드, light/dark
3. 다중 창, split, 저장·재시작, 세션을 유지하는 업데이트, 기존 기능 회귀

입력 방식: v0.1.21에서는 화면 도구가 `noWindowsAvailable`을 냈고, 비활성 창에 보낸 마우스 이벤트가 효과가 없었다. 이번에는 개발 런타임을 PID로 활성화하고 맨 앞 창인지 확인한 뒤 CGEvent를 보낸다. 동작마다 창 캡처로 변화를 확인한다. 탭 전환 하나로 먼저 시험하고, 실패하면 사용자가 체크리스트대로 직접 클릭하고 에이전트가 캡처와 판정을 맡는다(사용자 선택). 3회 뒤에도 미완료면 추가 회차를 먼저 품의한다.

전체 기능 E2E, 빌드 전·공개 전 실행, 60분 유효기간, 증거 형식은 [릴리스 운영](../../releasing.md)의 정상 schema 1을 따른다.

## 7. 적용·운영·복구

- 기능 PR마다 `docs/product-features.json`과 README/Pages를 다시 생성한다. #182는 새 ADR `0019-unified-sessions`와 새 스크린샷을 포함하고 `idle-agents.jpg` 항목을 지운다.
- `scripts/macos-release.py`의 `CASES`에 `PICKER-181`, `SESSIONS-182`를 더하고 테스트와 `docs/releasing.md`를 함께 고친다. v0.1.21 예외 경로는 그 버전에만 묶여 있으므로 바꾸지 않는다.
- 버전과 CHANGELOG는 release-plz가 만든다. 세 회차를 통과한 뒤 release PR을 병합하고, 빌드 전 E2E → `stage=build` → draft 실제 검수 → 공개 전 E2E → `stage=publish` → `stage=cask` 순서로 공개한다.
- 공개 노트에는 검증된 변경만 적고, Codex 공유 서버 경고(#173)는 미해결로 표시한다.
- 공개 후 문제가 생기면 배포된 산출물을 바꾸지 않고, 이전 서명 버전 복구 안내와 fix-forward 릴리스로 대응한다.
- 통과한 조건의 이슈는 조건 ID와 검수 보고서 링크를 남기고 닫는다. #149와 #151은 #182가 대체한 부분과 검수한 남은 부분을 나눠 적는다.

## 8. 승인·추적과 실행 자원

본문 정본은 이 문서다. 색인은 [design README](../README.md), 계획·승인·회고 링크는 Obsidian 작업 기록에 둔다. 승인은 **이 문서를 포함하는 커밋 + #181·#182 구현 + v0.1.22 검수·공개 계약**에 묶는다. 범위·동작·구조·데이터가 실질적으로 바뀌면 다시 검토받는다.

필요 자원: macOS GUI 개발 런타임, 손쉬운 사용·화면 기록 권한(호스트 chda.app), 독립 리뷰 에이전트, 현재 Claude·Codex CLI, GitHub Actions(macOS, Linux/Windows build), 기존 Apple·Sparkle 서명 자격증명과 Homebrew tap 권한. 확인하지 못한 실제 연동은 blocked로 기록하며 완료로 닫지 않는다.

## 공식 근거

- [objc2 `msg_send!` — nullable `Option<Retained<T>>` 반환](https://docs.rs/objc2/0.6.4/objc2/macro.msg_send.html)
- [Apple `NSOpenPanel`](https://developer.apple.com/documentation/appkit/nsopenpanel), [`beginWithCompletionHandler:`](https://developer.apple.com/documentation/appkit/nssavepanel/begin(completionhandler:))
- [고정된 GPUI `prompt_for_paths`](https://github.com/zed-industries/zed/blob/f8c2cc844057540ca1eac7de4f19f50d7597dead/crates/gpui_macos/src/platform.rs#L816-L862)
- [현재 사이드바 렌더링](https://github.com/magicsih/chda/blob/8ea18dd10e505bf78c00d98294e02eb4ca940243/crates/chda-ui/src/sidebar_view.rs#L1725-L1820), [선택 동기화](https://github.com/magicsih/chda/blob/8ea18dd10e505bf78c00d98294e02eb4ca940243/crates/chda-ui/src/workspace_view.rs#L3290-L3345), [Add repo](https://github.com/magicsih/chda/blob/8ea18dd10e505bf78c00d98294e02eb4ca940243/crates/chda-ui/src/workspace_view.rs#L3671-L3691)
