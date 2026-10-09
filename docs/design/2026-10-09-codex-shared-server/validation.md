# 설계 1.1 보존 조건 조사 — 진행 중

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
  신뢰 항목의 임시 등록을 이용하는 후속 검수에는 별도의 사용자 확인이 필요하다.

## 공식 소스의 비교 지점

| 비교 | 확인한 구현 | 아직 필요한 실행 증거 |
| --- | --- | --- |
| 실행 모드 | [0.161.0 daemon selection](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/daemon_startup.rs)의 allowlist는 chda title/notify 설정을 허용하지 않는다. | ordinary shell/menu/resume에서 실제 공유 서버 사용과 exact pane 연결 |
| 인증 | [0.161.0 target](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/lib.rs)의 remote client는 local auth restrictions를 제거하고 서버 정책에 맡긴다. | 같은 provider 서버의 managed authentication 결과가 기존 실행과 일치 |
| 권한 | [0.161.0 thread parameters](https://github.com/openai/codex/blob/rust-v0.161.0/codex-rs/tui/src/app_server_session.rs)는 Remote에서 초기 named profile을 생략하고 legacy sandbox를 보낸다. Remote resume는 permission overrides를 생략한다. | 사용자 지정 파일 접근 규칙과 captured resume 권한의 최종 결과 보존 |
| 알림 | 같은 thread request의 config allowlist에는 notify가 없다. | 기존 daemon의 user notify 실행 횟수, cwd 및 실행 환경 보존 |

이 차이 자체를 최종 권한 위반으로 단정하지 않는다. 서버가 적용한 최종
정책과 실제 CLI 동작을 대조해야 한다. 원본 통신을 그대로 전달하는 중계가
이 계약을 만족하지 못하면 승인된 조건에 따라 활성화하지 않는다.

[0.162.0 공개 릴리스](https://github.com/openai/codex/releases/tag/rust-v0.162.0)와
고정 소스도 조회했다. remote의 named profile 생략 및 client auth 분리 지점은
남아 있다. 0.162.0 바이너리는 설치하거나 실행 검수하지 않았다.

## 남은 gate

- 전역 설정 변경 없이 가능한 조사와, 테스트 폴더 신뢰를 필요로 하는 조사를 구분한다.
- 설정·승인·sandbox·managed authentication·notify·resume·동시 pane 보존을 실제로 확인한다.
- 보존한 경우에만 제품 raw relay, 소유 자원 수명, 상태 및 shell/menu/resume 연결을 구현한다.
- 제품 회귀 테스트와 CI, 독립 검수자를 포함한 macOS 검수 3회, 빌드 전 및 공개 전
  전체 E2E, 서명된 draft 검증은 아직 수행하지 않았다.
- 공개 버전은 v0.1.20이며 release-plz PR #167은 병합하지 않았다.
