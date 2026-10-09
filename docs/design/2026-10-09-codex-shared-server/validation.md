# 설계 1.1 보존 조건 조사 — 권한 보존 실패

2026-10-09. 설계 승인은 [approval.md](approval.md)에 기록했다.
제품 중계는 구현하거나 활성화하지 않았다. 다음 결과는 전체 기능 검수나
릴리스 E2E의 통과를 뜻하지 않는다.

## 확인한 사실

- 설치된 CLI는 Codex 0.161.0이다. 실제 chda 개발용 런타임은
  `a71f11f0b61cf43129baa2b77ef5c3693924a5f0`에서 빌드했다. 앱 설정과
  데이터는 별도 XDG 경로에 두고 설치된 사용자 앱과 구분했다.
- Codex가 TUI 연결 전에 업데이트 선택 화면을 표시했다. 이번 실행의
  `Skip`을 선택한 뒤 private endpoint의 WebSocket initialize 요청에
  도달했고, 테스트 endpoint의 의도적인 초기화 거부를 실제 화면에서 확인했다.
  이것은 공유 서버와의 정상 세션 사용이나 권한 보존의 증거가 아니다.
- 동일한 격리 프로젝트의 잘못된 TOML은 기본 실행과 `--remote --cd`
  실행 모두에서 오류로 표시됐다. `--remote`가 모든 프로젝트 설정 검사를
  생략한다고 해석하지 않는다.
- 기존 daemon 및 별도 소유 provider에 대한 조사용 연결은 initialize,
  account/read, config/read까지 도달했다. 폴더 신뢰가 필요한 단계에서
  종료했고 thread/turn 요청은 전달하지 않았다. 권한 보존은 아직 미검증이다.
- 화면 도구가 입력 후 오래된 화면을 반환한 구간이 있었다. 호출 성공만으로
  입력이나 검수 성공을 기록하지 않는다. 최신 창 연결과 실제 화면 변화가
  일치해야 다음 행동을 결정한다.
- 이 구간에서 테스트 폴더의 신뢰 항목 하나가 전역 Codex 설정에 저장됐다.
  해당 테스트 항목만 제거하고 다른 내용의 원래 바이트, 의미, 파일 권한을
  보존했는지 확인했다. 전역 config/auth 파일을 증거로 복제하지 않았다.
  이후 사용자가 같은 테스트 폴더에 한정한 임시 신뢰 등록을 승인했다.
  아래 검사가 끝날 때에도 해당 항목만 제거했고 다른 config의 바이트·의미·
  파일 권한 보존을 확인했다.

## 모델 요청 없는 권한 비교 결과

macOS에서 설치된 Codex 0.161.0 TUI와 별도 소유한 실제 Codex app-server를
사용했다. 합성 프로젝트의 `default_permissions = "chda_parity"`는
`:workspace`를 상속하며 합성 파일 `private.txt`에 `deny` 규칙을 둔다.
서버의 `config/read`와 `permissionProfile/list`에서 이 기본 선택, 규칙 및
프로필의 사용 가능 상태를 확인했다.

| 실행 또는 비교 | 서버가 확인한 결과 |
| --- | --- |
| 실제 TUI의 `--remote --cd` 초기 연결 | `thread/start`에 named permissions 없음, `sandbox = "workspace-write"`, approval은 `on-request`. 응답의 `activePermissionProfile` 없음 |
| 같은 서버의 기본 설정으로 빈 ephemeral thread 시작 | `activePermissionProfile.id = "chda_parity"`, approval은 `on-request` |
| 같은 서버에 named profile을 명시해 빈 ephemeral thread 시작 | `activePermissionProfile.id = "chda_parity"`, approval은 `on-request` |
| 실제 remote TUI와 같은 sandbox 인수로 빈 ephemeral thread 시작 | `activePermissionProfile` 없음. 일반 `workspaceWrite` 응답 |
| `command/exec`에 named profile을 명시해 합성 파일의 한 바이트 읽기 | exit 1, 읽기 거부 |
| `command/exec`에 실제 remote TUI가 반환한 sandbox를 적용해 같은 파일 읽기 | exit 0, 합성 바이트 읽힘 |

`command/exec` 비교는 모델 turn이나 TUI의 tool 실행이 아니다. profile 선택과
반환된 sandbox의 차이를 실제 macOS sandbox에서 확인한 별도 provider API
검사다. 이 API에서 permission 인수를 생략한 실행은 프로젝트 profile을
자동 선택하지 않았으므로 기본 TUI 실행의 대리 결과로 쓰지 않는다.

실제 remote TUI가 사용자의 기본 named profile을 보존하지 않는다는 결과와
해당 profile의 파일 거부 규칙이 일반 sandbox로 표현되지 않는다는 결과를
구분해 확인했다. 단순한 응답 표시 차이만으로 판단하지 않았다. 원본 통신을
그대로 전달하는 설계 1.1은 이 설정에서 보존 조건을 충족하지 못한다.

모델 turn 요청은 생성하거나 전달하지 않았다. 조사용 연결은 tool/turn/resume
등 범위 밖 요청을 거부했고 protocol 본문을 저장하지 않았다. 비교용 thread는
ephemeral로 만들고 연결을 해제했다. 실제 TUI의 빈 thread와 비교 API의
ephemeral thread를 전체 기능 검수나 실제 대화 검수로 세지 않는다.

## 공식 소스의 비교 지점

| 비교 | 확인한 구현 | 아직 필요한 실행 증거 |
| --- | --- | --- |
| 실행 모드 | [0.161.0 daemon selection](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/daemon_startup.rs)의 allowlist는 chda title/notify 설정을 허용하지 않는다. | ordinary shell/menu/resume에서 실제 공유 서버 사용과 exact pane 연결 |
| 인증 | [0.161.0 target](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/lib.rs)의 remote client는 local auth restrictions를 제거하고 서버 정책에 맡긴다. | 같은 provider 서버의 managed authentication 결과가 기존 실행과 일치 |
| 권한 | [0.161.0 thread parameters](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/app_server_session.rs)는 Remote에서 초기 named profile을 생략하고 legacy sandbox를 보낸다. Remote resume는 permission overrides를 생략한다. | 사용자 지정 파일 접근 규칙과 captured resume 권한의 최종 결과 보존 |
| 알림 | 같은 thread request의 config allowlist에는 notify가 없다. | 기존 daemon의 user notify 실행 횟수, cwd 및 실행 환경 보존 |

공식 소스의 차이를 먼저 조사하고 위의 실행 비교로 한 보존 실패를 확인했다.
다른 사용자 설정 전부나 managed authentication, notify, resume의 최종 결과를
검증했다고 주장하지 않는다. 하나의 필수 보존 조건이 실패했으므로 승인된
조건에 따라 중계를 구현·활성화하지 않는다.

[0.162.0 공개 릴리스](https://github.com/openai/codex/releases/tag/rust-v0.162.0)와
고정 소스도 조회했다. remote의 named profile 생략 및 client auth 분리 지점은
남아 있다. 0.162.0 바이너리는 설치하거나 실행 검수하지 않았다.

## 남은 gate

- #173은 미해결로 유지한다. 전역 설정 변경, 자동 embedded 선택, 경고 숨김,
  raw 요청의 권한 변조로 이 결과를 우회하지 않는다.
- 다른 연결 구조를 쓰려면 새 설계와 실제 보존 증거가 필요하다. 설치된 CLI를
  임의 업그레이드하거나 별도 Codex 배포판으로 교체하지 않는다.
- 다음 릴리스에서 #173을 보류할지, 해결될 때까지 공개를 기다릴지는
  [범위 재검토 1.2](../2026-10-09-release-scope/design.md)의 승인 대상이다.
- 제품 회귀 테스트와 CI, 독립 검수자를 포함한 macOS 검수 3회, 빌드 전 및 공개 전
  전체 E2E, 서명된 draft 검증은 아직 수행하지 않았다.
- 공개 버전은 v0.1.20이며 release-plz PR #167은 병합하지 않았다.

## Codex 0.162.0 재검증 — 2026-10-09

사용자가 승인한 범위에서 설치된 Codex 0.162.0을 다시 확인했다. 공식 소스의
네 비교 지점을 0.161.0과 대조하고, 모델 요청 없이 실제 TUI의 시작 화면만
확인했다. 권한 비교 실행은 다시 하지 않았다.

### 공식 소스 비교

고정 tag `rust-v0.161.0`과 `rust-v0.162.0`의 소스를 직접 비교했다.

| 비교 지점 | 0.161.0 | 0.162.0 | 변화 |
| --- | --- | --- | --- |
| 실행 모드 | [daemon 선택 허용 목록](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/daemon_startup.rs#L58-L108)은 `-c` 중 `suppress_unstable_features_warning`, `tui.fullscreen_transcript`, 지정된 `features.*`만 허용한다. 나머지는 [embedded 경고](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/startup_orchestration.rs#L592-L603)로 이어진다. | [같은 허용 목록](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/daemon_startup.rs#L90-L140), [같은 경고](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/startup_orchestration.rs#L599-L610) | 허용 목록 변화 없음. `tui.terminal_title`과 `notify`는 여전히 제외 사유다. 추가된 것은 [WSL DrvFS 제외](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/daemon_startup.rs#L18-L49)뿐이다. |
| 인증 | [remote client](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/lib.rs#L364-L373)는 `forced_login_method`, `forced_chatgpt_workspace_id`, `managed_auth_policy`를 비우고 서버 정책에 맡긴다. | [같은 처리](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/lib.rs#L368-L377) | 변화 없음 |
| 권한 | Remote에서 [초기 named profile을 생략](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/app_server_session.rs#L2063-L2075)하고, [resume의 approval·sandbox·permissions override를 제거](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/app_server_session.rs#L2179-L2189)한다. | [초기 생략](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/app_server_session.rs#L2128-L2140)과 [resume 제거](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/app_server_session.rs#L2239-L2249)가 같다. | 변화 없음 |
| 알림 | [thread request의 config 허용 목록](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/app_server_session.rs#L1848-L1875)에 `notify`가 없다. | [같은 허용 목록](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/app_server_session.rs#L1935-L1962) | 변화 없음 |

[0.162.0 릴리스 노트](https://github.com/openai/codex/releases/tag/rust-v0.162.0)에서
관련 가능성이 있는 항목도 소스로 확인했다.

- #50140 *Use the server permission catalog for TUI permission shortcuts*: 변경
  범위는 permission shortcut과 picker의 catalog 조회다. thread 시작·resume의
  권한 인수를 만드는 위 함수는 그대로라서 remote TUI의 named profile 생략과
  무관하다.
- #50803 *Use the managed daemon for eligible remote-control launches*:
  `codex remote-control` 하위 명령의 경로다.
  [daemon_eligible](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/cli/src/remote_control_cmd.rs#L148-L166)은
  `-c` override가 하나라도 있으면 foreground 경로를 쓴다. TUI 쪽 변경은
  `uses_wsl_drvfs` 공개뿐이므로 chda의 일반 TUI 실행과 무관하다.
- #50013, #50811, #50913: daemon·remote 연결의 새 thread에서 서버의
  model·reasoning 기본값을 쓰는
  [변경](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/tui/src/app/startup_bootstrap.rs#L9-L56)이다.
  권한 인수는 바꾸지 않는다.

### 실행 관찰

- 환경: macOS, Homebrew로 설치된 `codex-cli 0.162.0`. 시작 전에 0.162.0
  managed daemon(updater와 app-server 프로세스 각 1개)이 이미 실행 중이었다.
- 방법: 비어 있는 새 임시 폴더에서 pseudo-terminal(120x40)로 TUI를 약 11초
  실행했다. 터미널 capability 질의에만 응답했고, 시작 7초 뒤 경고 목록을 여는
  F2를 한 번 보냈다. prompt나 모델 요청은 보내지 않았고, 이후 직접 시작한
  프로세스 그룹만 종료했다.
- 두 실행 모두 업데이트 화면이나 폴더 신뢰 화면은 나오지 않았다.

| 실행 | 관찰 |
| --- | --- |
| `codex -c 'tui.terminal_title=["activity","app-name","run-state"]'` (`CODEX_TITLE_CONFIG`와 같은 값) | 하단에 `⚠ 1 warning · f2 to view`가 표시됐다. 경고 목록 `Warnings · 1 of 1 · Startup`에 아래 문장이 나왔다. chda 형식의 제목(`codex \| Starting`, `codex \| Ready`)은 출력됐다. 직접 시작한 프로세스 아래에 MCP 등 별도 하위 프로세스가 생겼다. |
| 같은 방법, override 없음 | 경고 표시가 없었고 F2 목록은 `No warnings`였다. 직접 시작한 프로세스 아래에는 하위 프로세스가 없었고, 기존 daemon이 임시 폴더를 cwd로 하는 하위 프로세스를 시작했다. |

관찰한 경고 문장:

```text
Running without the shared background server: command-line configuration overrides (-c, --enable, --disable, or --search) requires embedded mode.
```

정리와 한계:

- 직접 시작한 프로세스와 관찰한 하위 프로세스가 모두 종료된 것을 PID로
  확인했다. override 없는 실행에서 daemon이 시작한 하위 프로세스는 연결 종료
  뒤 1분 넘게 남아 있다가 다음 확인 때 모두 종료돼 있었다. daemon 두 프로세스의
  PID와 시작 시각은 실행 전후 같았다.
- 전역 `config.toml`의 hash·수정 시각·권한과 auth 파일의 수정 시각은 실행
  전후 같았다. 신뢰 항목을 추가하지 않았다. 최근 session 파일 중 임시 폴더를
  참조하는 파일은 없었다.
- `notify` override는 chda hook 경로가 필요해 넣지 않았다. 소스상 허용 목록
  밖의 `-c` 하나만으로 같은 제외가 걸린다. 다만 title과 notify를 함께 넣은
  실제 chda 실행(shell wrapper, 메뉴, resume)은 이번에 실행하지 않았다.
- 화면 텍스트는 escape sequence를 제거해 읽었다. 전체 화면 기록은 저장소에
  넣지 않았다.
- 합성 profile과 임시 신뢰 등록이 필요한 권한 비교 실행은 0.162.0으로 다시 하지
  않았다. 소스의 권한 경로가 바뀌지 않았다는 판단이며 실행 증거를 대신하지
  않는다.

### 결론

- 0.162.0에서도 chda의 title override 하나만으로 일반 실행이 embedded 방식으로
  바뀌고 경고가 나온다. 같은 환경에서 override가 없으면 경고 없이 기존 daemon을
  사용했다.
- remote TUI가 named permission profile과 resume 권한을 보내지 않는 코드는
  0.161.0과 같다. 설계 1.1 중계의 권한 보존 실패 원인은 0.162.0에도 남아 있다.
- #173은 미해결로 유지한다. 해제하려면 upstream에서 다음 중 하나가 필요하다.
  daemon 선택 허용 목록이 client 전용 `tui.terminal_title`과 client별 notify
  대체 경로를 받아들이거나, remote TUI가 named profile과 resume 권한을
  보존하거나, local client용 상태 API가 생겨야 한다. 그 전에는 경고 숨김,
  자동 embedded 선택, 전역 설정 변경으로 우회하지 않는다.
