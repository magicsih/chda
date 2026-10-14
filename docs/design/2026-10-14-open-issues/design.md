# 열린 이슈 일괄 처리 (Windows/Linux 제외)

- 설계 ID: `2026-10-14-open-issues`
- 버전: 1.0
- 상태: **승인 · 구현 진행 중**. [승인 기록](approval.md)

## Context

열린 이슈 31건 중 Windows/Linux 티켓 17건(#14, #185–#200)을 뺀 14건을 처리한다. 사용자는 이슈가 계속 쌓이는 것을 원하지 않는다. 이번 묶음은 **PR 없이 main에 직접 반영**하라고 명시했다. AGENTS.md의 "Branch + pull request for every change"에 대해 이번 묶음에만 적용하는 예외다. main에는 branch protection과 ruleset이 없다. CI는 push마다 Conventional Commits 검사와 macOS/Linux/Windows 빌드·테스트를 실행한다.

사용자 결정(2026-10-14):
- 상위 차단(#173)과 재현되지 않는 현상(#241)은 **근거를 남기고 닫는다**. 재발하거나 상위가 바뀌면 다시 연다.
- 끝 지점은 **main 반영과 CI 통과까지**다. release-plz가 v0.1.24 릴리스 PR을 열어 두고, 공개는 나중에 따로 지시받는다.
- 검수는 **가볍게** 한다. 이슈마다 자동 시나리오 테스트를 돌리고, 작성자가 실제 앱에서 확인한 뒤 스크린샷을 남긴다. 묶음 끝에 다른 에이전트가 화면 변경분을 실제 앱에서 **독립 검수 1회** 한다. 정책 기본값(새 화면은 독립 검수 3회)보다 낮춘 사유는 "사용자의 빠른 처리 요청"으로 설계 기록에 남긴다.
- #246은 **부모 행 끝에 붙이는** 모양으로 한다.

조사 결과 분류(기준 main `b491670`):

| 분류 | 이슈 | 처리 |
|---|---|---|
| 이미 구현·공개됨 | #158 Copy for sharing(#174·#214·#240), #161 하위 에이전트 표시(#172·#221·#227) | 감사 코멘트를 남기고 completed로 닫음 |
| 상위 차단 | #173 Codex `-c` 경고 | 0.163.0-alpha.11(오늘 공개)도 `codex-rs/tui/src/daemon_startup.rs`가 0.162.1과 바이트 단위로 같음. not planned로 닫음 |
| 버그 | #243 PATH 캡처, #244 Claude 사용량, #241 macOS Space | 실패 테스트부터 만들고 수정 |
| 기능 | #242, #250, #247, #246, #249, #248, #251 | 구현 |
| 연구 | #252 실행 중인 대화에 프롬프트 전달 | 시제품, 권고, 결과가 긍정이면 ADR 제안 |

## 진행 방식

- main에서 이슈를 하나씩 순서대로 진행한다. 거의 모든 이슈가 큰 공유 파일 `crates/chda-ui/src/workspace_view.rs`를 건드리기 때문이다. #252 연구만 별도 에이전트가 레포 밖의 임시 폴더에서 병렬로 진행한다.
- 이슈마다 다음 순서로 진행한다.
  1. 요구사항 때문에 실패하는 테스트를 확인한다.
  2. 최소한으로 수정한다.
  3. `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, 관련 테스트를 실행한다.
  4. push 전에 `cargo test --workspace`를 실행한다.
  5. `docs/product-features.json`을 갱신하고 `python3 scripts/sync-product-docs.py`를 실행한다.
  6. 영어 Conventional Commit으로 커밋한다.
  7. `git fetch` 후 fast-forward로 main에 push한다.
  8. CI를 확인한다.
  9. 이슈에 커밋 링크와 검증 내용을 코멘트로 남기고 닫는다.
- 첫 커밋으로 이 계획을 `docs/design/2026-10-14-open-issues/design.md`와 `approval.md`에 옮긴다(레포 관례). 같은 커밋에서 `docs/design/README.md` 표에 한 줄을 추가한다.
- 대체된 코드는 같은 커밋에서 지운다. 예: #246의 `child_collapsed`.
- 실제 앱 확인은 격리 HOME으로 띄운 debug 빌드로 한다. 사용자가 실행 중인 chda는 건드리지 않는다. 확인하는 동안 실제 커서와 포커스를 잠깐 사용한다. 방식은 HID 이벤트와 `screencapture -l<windowID>`다.

## 이슈별 설계

### 0. 바로 닫기 — #158, #161, #173
- #158: 구현 커밋 #174·#214·#240과 시나리오 `sharing::*`를 적는다. v0.1.22/0.1.23 실제 검수에서는 부분 관찰만 했다는 점도 그대로 적고 닫는다.
- #161: #172·#221·#227과 CHILD-161 재정의(Codex 하위 에이전트는 실행 중 Unknown)를 적고 닫는다. v0.1.22 검수에서 "Codex 하위 작업이 끝난 뒤에도 Unknown"이 관찰됐다. 이 확인은 #246에서 이어서 한다.
- #173: 오늘 날짜 상위 소스 비교 결과와, 차단이 풀리는 상위 변경 3가지(validation.md 174–178행)를 적는다. not planned로 닫는다.

### 1. #243 셸 PATH를 2초 넘게 걸려도 찾기
- `Environment`(`crates/chda-ui/src/environment.rs`)의 고정 PATH를 공유 상태 `Rc<RefCell<ShellPath>>`로 바꾼다. 상태는 `Pending | Ready(String) | Failed{error, searched}` 세 가지다.
  - `search_path()`와 새 `pane_env()`가 이 상태를 읽는다.
  - `pane_env.clone()`을 직접 읽던 세 곳(terminal_view.rs:246, workspace_view/children.rs:125, preparation.rs:513)을 `pane_env()`로 바꾼다.
- 시작 시 동기 2초 캡처는 유지한다. 시간이 넘으면 백그라운드에서 다시 캡처한다. 마감은 20초이고, 실패하면 30초·2분·10분 간격으로 재시도한다. 성공하면 모든 창에 알린다. 캡처 함수는 `latest_release`처럼 주입할 수 있게 해서 테스트에서 느린 셸을 흉내 낸다.
- PATH를 확인하는 중(`Pending`)이면 에이전트 실행, 복원, 재개는 캡처가 끝날 때까지 기다린 뒤 조회한다.
- Codex 사용량에 `CodexQuotaError::PathUnavailable{searched}`를 추가한다. 라벨은 "Could not read the shell PATH"다. `NotInstalled`의 상세 문구에는 검색한 경로를 적는다. 사용량 확인 루프는 캡처가 갱신되면 바로 다시 확인하고, 조회는 어댑터의 `executable()`로 통일한다.
- 테스트: harness의 `gui-path`에 `sleep 3` 후 PATH를 추가하는 `.zprofile`을 두면 지금은 실패한다. 이 상태에서 Codex 사용량, 실행, 재개를 확인하는 시나리오와 `quota.rs` 라벨 테스트를 추가한다.
- README의 333–337행("2초 안에 못 읽으면 상속 PATH 유지")을 고친다.

### 2. #244 Claude 사용량 "Waiting for usage"
- `QuotaSnapshot`에 `#[serde(default)] config_dir`를 추가한다. `statusline::main`이 이 값을 `CLAUDE_CONFIG_DIR` 또는 `~/.claude`로 채운다. `existing_status_line`과 같은 방식으로 결정한다.
- `ProviderQuotas::provider`(`status_bar.rs`)는 포커스된 pane의 config_dir를 기준으로 고른다. 같은 config_dir 안에서 사용량이 있는 가장 최근 보고를 고르고, 없으면 그 pane 자신의 보고를 쓴다. config_dir가 다른 보고는 절대 쓰지 않는다. 상세 문구에는 출처 세션과 경과 시간을 적는다.
- 빈 상태 문구는 세 경우로 나눈다.
  - 응답 전: "after the next Claude response"
  - 브리지 보고가 없음: 실행 중인 Claude pane에서 보고가 한 번도 오지 않은 경우
  - 구독 한도가 없는 로그인: 턴이 끝난 뒤에도 `rate_limits`가 없는 경우
- 재시작과 업데이트 뒤에도 config_dir별 마지막 사용량을 데이터 폴더 `quota-cache.json`에 저장해 이어 보여 준다. 값은 숫자, 리셋 시각, 관찰 시각뿐이다. "previous run · Nm ago"처럼 오래된 값으로 표시한다.
- 실제 재현: Claude Code 2.1.299 세션 두 개로 위 시나리오 1·2를 확인한다. status line 입력에 `rate_limits`가 실리는지도 기록한다.
- 테스트: `status_bar.rs` 단위 테스트를 추가한다(새 빈 보고와 그보다 오래된 사용량 보고, 포커스 우선, 다른 config_dir 배제, 재시작 후 캐시). `empty_claude_reports_preserve_usage...`의 "다른 세션은 물려받지 않음" 단언은 새 규칙에 맞게 바꾼다. `scenarios/quota.rs`의 재시작 시나리오도 고친다. `docs/status-bar.md`를 갱신한다.

### 3. #241 macOS 전체 화면 뒤 창 크기와 Space
- `note_bounds`(workspace_view.rs:2402)는 `window.is_fullscreen()`일 때 저장된 창 크기를 갱신하지 않는다. 마지막 창 모드 크기를 유지하고, 전체 화면 크기를 창 모드 크기로 저장하지 않는다. 다시 실행하면 이전 창 모드 크기로 복원한다(전체 화면으로 다시 열지 않음).
- 테스트는 `scenarios/restore.rs`에 둔다. test window에서 `toggle_fullscreen`과 크기 변경을 하면, 수정 전에는 저장 크기가 바뀌어 실패한다. 이전 형식 `session.json`을 계속 읽는지도 확인한다.
- 실제 앱에서 다음 조합의 재현을 시도한다.
  - 초록 버튼, `ctrl-cmd-f`로 전체 화면 진입·해제
  - Rectangle 사용
  - 전체 화면 상태에서 종료한 뒤 재실행
  - 전환 중 세션 유지 업데이트
  - 시도마다 이슈에 적힌 AX, CGWindow, SkyLight 수집 명령을 실행한다.
- 결과를 이슈에 기록하고 닫는다. 재현되지 않으면 재발 시 수집 절차를 남긴다.

### 4. #242 PROJECT 저장소 한 번에 펼치기·접기
- PROJECT 머리글의 `+ repo` 옆에 아이콘 버튼 하나를 둔다. 하나라도 펼쳐져 있으면 "Collapse all repositories", 모두 접혀 있으면 "Expand all repositories"를 보여 준다. 아이콘과 툴팁도 그 상태를 따른다. 저장소가 2개 미만이면 숨긴다.
- #152가 없앤 2아이콘 머리글은 되살리지 않는다. 예전 `collapse_all`(315f879)을 `set_all_repos_collapsed`로 되살린다. 이 함수는 `reveal`/`pending_reveal`을 비우고 툴팁을 닫고 터미널로 포커스를 돌린다.
- View 메뉴와 팔레트에 "Expand All Repositories"와 "Collapse All Repositories"를 추가한다.
- 조사해 보니 이슈 본문과 달리 저장소별 접힘(`RepoEntry.collapsed`)은 지금 메모리에만 있다. 접힌 저장소 경로를 `config.toml`(`collapsed-repos`)에 저장해, 개별 토글과 일괄 토글 모두 재시작 뒤에도 유지되게 한다.
- 테스트:
  - `scenarios/sidebar.rs:497-499`의 "버튼 없음" 단언을 뒤집는다.
  - `header_buttons_show_their_own_tooltips` 표에 새 버튼을 추가한다.
  - 상태가 섞여 있을 때, 클릭 후 툴팁(#217), 개별·sticky 토글 유지, PROJECT 섹션 토글과의 분리, 재시작 후 유지를 확인한다.

### 5. #250 OSC 9 / OSC 777 데스크톱 알림
- 고정된 `libghostty-vt`(rev d2036ba)에 이미 `Terminal::on_desktop_notification`이 있다. OSC 9;4 진행률은 파서가 따로 처리한다.
  - `vt.rs`의 `Effects`에 알림 목록을 추가하고 콜백을 등록한다. 글자는 손실 허용 UTF-8로 읽고, 제어문자를 없애고, 제목 80자·본문 200자로 자른다.
  - `session.rs`에서 `Event::DesktopNotification`으로, `terminal_view`에서 `TerminalEvent`로 넘긴다.
- workspace(벨 처리 근처, 2749행)에서는 세 가지를 한다.
  - 탭에 표시를 단다.
  - pane 정보와 함께 `record_once`로 알림 기록 항목을 만든다.
  - pane이 보이지 않을 때 OS 알림을 보낸다. 클릭하면 정확한 pane으로 이동한다.
- 알림 클릭이 마지막에 열린 창으로만 가는 기존 문제(macos.rs:131의 처리기가 하나)는 registry에서 pane을 소유한 창을 찾게 고친다.
- 훅으로 상태를 보고하는 에이전트가 살아 있는 pane에서 온 OSC 알림은 무시한다(중복 방지). pane마다 1초 간격, 분당 10건으로 제한한다. `notifications = false`면 OSC 알림을 전부 무시한다.
- ADR 0012의 "터미널 내용은 수집하지 않는다"를 고친다. 프로그램이 명시적으로 요청한 알림 문구(OSC 9/777)만 예외로 하고, 자른 평문으로 표시한다.
- 테스트:
  - `vt.rs` 단위: OSC 9, OSC 777, 9;4는 알림 아님
  - 시나리오: `printf`로 기록 항목, RecordingSystem 알림, 클릭 시 pane 포커스, 속도 제한, 에이전트 중복 방지, 설정 꺼짐
  - 실제 앱에서 `printf` 확인

### 6. #247 macOS 밝은/어두운 모드에 따른 테마 쌍
- chda-config에 GPUI와 무관한 `Appearance {Light, Dark}`를 추가한다. `theme_name(value, appearance)`와 `load(paths, theme, appearance)`로 바꾼다. Ghostty 설정과 chda `theme` 모두 `light:X,dark:Y`를 받는다. 이름이 하나면 지금과 같다.
- chda-ui는 창마다 `cx.observe_window_appearance`를 등록해 `reload_config`를 호출한다. 시작할 때는 `cx.window_appearance()`를 쓴다. `load`를 호출하는 네 곳에 외형을 넘긴다.
- 테마 선택기에 "Select Light Theme…"와 "Select Dark Theme…"를 추가한다. 고르면 반대쪽 테마를 유지한 채 `light:X,dark:Y`로 저장한다. 미리보기는 고르는 테마를 보여 주고, 취소하면 원래대로 돌아간다.
- 테스트:
  - 파싱 단위 테스트를 추가한다. 기존의 "쌍이면 dark" 단언은 바꾼다.
  - 외형을 직접 넘기는 시나리오를 만든다. test window는 외형 변경을 공개 API로 흉내 낼 수 없다.
  - 기존 3:1 대비 검사는 두 테마에서 유지한다.
- 실제 앱 확인: `osascript`로 시스템 외형을 바꿔 확인한 뒤 원래대로 되돌린다.

### 7. #246 하위 에이전트를 부모 행 끝에 한 줄로(사용자 선택)
- 하위 행, 그룹 머리글, 펼치기 토글을 없앤다. `child_collapsed`도 handoff 필드와 registry까지 함께 지운다. 예전 handoff JSON에 이 필드가 있어도 읽히는지 테스트한다.
- 에이전트 행의 종류 글자("Claude Code"/"Codex")를 과거 세션 행에서 쓰는 색 배지 `[CC]`/`[CX]`로 바꾼다(`AgentLabel.short/color`). 하위 에이전트가 있으면 배지 뒤에 숫자를 붙인다.
  - `▶ 실행`
  - `⏸ 입력 대기`: 대기 색, 0이면 숨김
  - `? 미확인`: 0이면 숨김
  - `⑂ 전체`
  - 미확인이 있으면 `▶ 0`을 숨긴다. 기존 "0 active라고 주장하지 않기" 규칙을 지킨다.
- 좁은 폭에서는 다음 순서로 줄인다: 전체 → 대기 → 배지. label 최소 48px, 분할 번호, 경과 시간은 자르지 않는다. 툴팁에는 에이전트 이름, 각 개수, "Right-click for details"를 적는다.
- 오른쪽 클릭은 workspace `ContextMenu`에 하위 에이전트마다 "label — state · 2m ago"를 보여 준다. 고르면 기존 `open_child_details`를 그대로 쓴다. 메뉴를 닫으면 포커스는 터미널로 돌아간다. 왼쪽 클릭은 지금 행 클릭과 같다.
- v0.1.22에서 관찰된 문제를 실제 Codex 0.162.1 하위 에이전트로 재현해 본다. 하위 작업이 끝난 뒤에도 Unknown으로 남으면 실패 테스트와 함께 고친다.
- 테스트:
  - `scenarios/children.rs` 재작성: 실행·대기·미확인·종료 하위, 오른쪽 클릭 상세, 하위 없는 부모
  - `sessions.rs:~577` 단언 수정
  - 좁은 폭과 밝은/어두운 테마 bounds 검사
- README·Pages의 Sessions 스크린샷(docs/media)을 다시 찍는다. ADR 0015와 0019를 갱신한다.

### 8. #249 `chda mcp` 읽기 전용 조정 도구
- `control.rs`의 `Request`에 `ListSessions{cwd}`, `ReadOutput{cwd, pane, lines}`, `WaitForStatus{cwd, pane, states, timeout_ms}`를 추가한다. 호출자 cwd와 같은 저장소의 worktree에 있는 세션만 보인다. 다른 저장소 접근은 이번 범위 밖이다.
- `list_sessions`는 모든 창에서 다음을 모은다: pane id, provider, 상태(Working / Waiting for input / Turn complete / Idle / Ended / Terminal), worktree, branch, note, 탭·분할 번호, 마지막 활동, 하위 개수. 모르는 값은 unknown으로 둔다.
- `read_output`은 아직 입력 중인 내용을 절대 담지 않는다.
  - 에이전트 pane: ADR 0016의 정확한 대화 transcript 읽기를 재사용해 최근 assistant 평문을 돌려준다.
  - 셸 pane: OSC 133 마크로 현재 프롬프트 입력 앞까지의 화면과 스크롤백 평문만 돌려준다.
  - 기본 100줄, 최대 500줄, 64KB로 제한한다.
- `wait_for_status`: 이미 원하는 상태면 바로 답한다. 아니면 응답을 보류 목록에 두었다가 상태가 바뀌거나 pane이 닫히면 답한다. 제한 시간은 최대 55초다(IPC 응답 제한 60초보다 짧게). 더 길게 기다려야 하면 다시 호출하라고 설명에 적는다.
- 포커스, 행, 읽음 상태는 바꾸지 않는다. `docs/decisions/0006`을 갱신하고 `control.rs`의 오래된 doc 주석을 고친다.
- 테스트:
  - `mcp.rs` 도구 목록과 스키마
  - `control.rs` 전송 형식
  - 시나리오: 한 에이전트가 다른 에이전트가 끝날 때까지 기다림, 입력 중인 글자 제외, 다른 저장소 차단
  - 실제 Claude Code와 Codex에서 세 도구 호출

### 9. #248 탭 순서 바꾸기와 pane 이동
- chda-core `Workspace`에 다음을 추가한다: `move_tab`, `take_pane`, `insert_pane_as_tab`, `insert_pane_into_tab`(대상 탭의 포커스된 pane을 긴 쪽으로 분할). focused와 zoomed를 보정하고, 마지막 pane을 빼면 그 탭을 닫는다. 두 pane 바꾸기(선택 항목)는 넣지 않는다.
- 탭 드래그는 저장소 드래그 방식(`RepoDrag`, `on_drag`/`drag_over`/`on_drop`)을 따른다. 보이는 그룹의 인덱스를 전역 `tabs` 인덱스로 바꿔 쓴다.
- 탭과 터미널의 오른쪽 클릭 메뉴, 팔레트에 다음을 추가한다: "Move to New Tab", "Move to Tab…", "Move to Window…"(새 창 포함).
- 같은 창 안에서는 `TerminalView`를 그대로 두고 트리만 옮긴다. 창 사이에서는 기존 세션 유지 업데이트 경로를 쓴다. `Session::detach()`로 `DetachedSession`을 만들고, 대상 창의 `TerminalView::plan`이 pane id로 넘겨받는다. `PaneInfo`(에이전트 상태, 대화 식별자, 하위 활동)와 registry도 함께 옮긴다.
- pane id는 프로세스 전체에서 유일하다. 그래서 Sessions 행, 알림, "기다리는 에이전트로 이동"은 그대로 새 위치를 가리킨다.
- 테스트:
  - core 단위 테스트
  - 시나리오: 드래그 재정렬과 재시작, 탭 간 이동, 다른 창으로 이동(같은 셸 PID, 스크롤백 유지), 복원과 handoff
- ADR 0010을 갱신한다.

### 10. #251 사용자 정의 명령
- `config.toml`의 `[[commands]]`에 `name`, `run`, `open = "tab" | "split" | "floating"`(기본 tab), `key`(선택)를 둔다. 이 파일에서만 읽는다. 프로젝트별 설정이나 `repo_preparation` 가져오기로는 명령을 추가할 수 없다.
- 잘못된 항목은 `reload_config`의 문제 목록에 한 번만 기록하고, 나머지 명령은 불러온다. 문제의 예: 빈 이름이나 명령, 중복, 알 수 없는 open, 해석할 수 없는 키, 기본 단축키나 다른 명령과 충돌하는 키. 충돌하면 그 키만 버린다.
- 실행: `$SHELL -l -c "<run>"`을 #243의 `pane_env()`와 함께 쓴다. 위치는 포커스된 pane의 worktree, 또는 오른쪽 클릭한 worktree다. 실행 경로는 팔레트, worktree 메뉴, 단축키다. 단축키는 설정을 다시 읽을 때 기본 바인딩과 사용자 바인딩을 다시 등록한다.
- tab과 split은 `PaneRun`(ADR 0014)으로 종료 상태를 보여 준다.
- floating은 창 위에 뜨는 모달 터미널이다(약 80%).
  - 탭 밖에 등록된 pane이며, 세션 저장·복원과 Sessions 목록에서 뺀다.
  - 성공적으로 종료하면 닫힌다. 실패하면 종료 코드를 보여 주며 남는다.
  - Esc는 프로그램에 전달한다(lazygit이 Esc를 씀). 실행 중에 닫으려면 `cmd-w`를 누르고 확인해야 한다.
- 테스트:
  - `chda.rs` 파싱과 검증
  - 시나리오: 세 열기 방식, 설정 오류, 실시간 다시 읽기, 단축키 충돌
  - 실제 앱에서 lazygit(floating)

### 11. #252 연구(병렬 에이전트)
- 레포 밖 임시 폴더의 버리는 시제품으로 진행한다. 레포에는 넣지 않는다.
  - 경로 1: Claude Code 2.1.299 channels. stdio MCP 서버 + `--dangerously-load-development-channels`.
  - 경로 2: Codex 0.162.1 app-server의 `turn/start`와 `turn/steer`. daemon에 붙은 순정 `codex`와, chda가 덧붙이는 `-c` 때문에 embedded로 도는 경우 두 가지.
  - 경로 3: bracketed paste. 기준선으로만 본다.
- 이슈의 질문 6개를 실제로 관찰한다: 정확한 대상, 입력 중인 글자 보존, 승인 대기·작업 중·대기 상태, 표시, 버전 조건, 주입 방지. 모델 호출은 짧은 합성 프롬프트로 최소화한다.
- 비교와 권고는 이슈 코멘트로 남긴다. 결과가 긍정이면 ADR 0017을 고치는 `docs/decisions/0020-*.md`(Status: Proposed, 사용자 결정 필요)와 후속 기능 이슈를 만든다. 그 뒤 연구 이슈를 닫는다.

## 순서

0(닫기) → 설계 기록 커밋 → #243 → #244 → #241 → #242 → #250 → #247 → #246 → #249 → #248 → #251 → (#252 병렬, 끝나면 반영) → 묶음 독립 검수 1회 → 지적 수정.

#243을 먼저 하는 이유: #251과 에이전트 실행이 `pane_env()`를 쓴다. #246은 #249의 `list_sessions` 하위 개수와 같은 데이터를 쓴다.

## 검증

- 자동: 이슈마다 위 테스트를 수정 전 실패 → 수정 후 통과로 확인한다. push 전에 `cargo test --workspace`(macOS), clippy, fmt, `python3 scripts/sync-product-docs.py` 결과 일치를 확인한다. push 뒤에는 CI 7개 잡(Linux/Windows 빌드 포함)이 녹색인지 확인한다.
- 실제 앱(작성자): 격리 HOME의 debug 빌드로 이슈별 "running app" 인수조건을 확인하고 스크린샷을 남긴다.
  - #243: Finder 실행 흉내(launchd PATH + 느린 `.zprofile`)
  - #244: Claude 세션 2개
  - #241: 전체 화면 조합
  - #242: 보통·좁은 폭, 밝은/어두운 테마
  - #250: `printf '\e]9;done\a'`, `printf '\e]777;notify;T;B\a'`
  - #247: 시스템 외형 전환
  - #246: 실제 Claude·Codex 하위 에이전트
  - #249: 실제 Claude·Codex에서 도구 호출
  - #248: 드래그와 창 간 이동 후 같은 PID
  - #251: lazygit floating
- 독립 검수 1회: 묶음 끝에 다른 에이전트가 화면 변경분(#242, #246, #247, #248, #250, #251)을 실제 앱에서 재현하고 판정한다. 실패는 범위 안에서 고친다.
- 문서: README·Pages의 생성 섹션과 스크린샷을 데스크톱(1280px)과 좁은 폭(390px)에서 렌더해 확인한다.

## 정지 조건

- 범위 안에서 고칠 수 없는 실패
- CI 차단
- 실제 앱이나 CLI를 쓸 수 없음
- #252 결과가 ADR 0017 변경을 요구함(제안까지만 하고 결정은 사용자에게)
- 상위(GPUI/libghostty) 결함 때문에 인수조건을 못 지킴(근거를 남기고 보고)
