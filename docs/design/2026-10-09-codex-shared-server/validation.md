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
