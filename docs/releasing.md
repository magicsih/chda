# 릴리스 운영

## Linux 릴리스

Linux x86_64 배포는 [ADR 0021](decisions/0021-linux-distribution.md)을 따른다.
workspace 버전과 stable 태그는 아래 macOS 절차와 마찬가지로 release-plz가 만든다.
태그 생성만으로 공개하지 않는다.

1. `Linux packages`의 main 후보 실행에서 AppImage와 portable archive를 만든다.
   Ubuntu 20.04 빌더의 glibc 2.31 ABI 검사, 두 형식의 동일 payload 검사,
   Ubuntu, Debian, Fedora, Arch 기동과 X11, Wayland 개발 검증을 모두 통과해야 한다.
2. `Linux native review`에 정확한 후보 SHA와 위 실행 번호를 전달한다.
   기존 산출물을 다운로드해 세 번의 독립 화면 검수에 사용하며 다시 빌드하지 않는다.
   Openbox X11, Mutter Wayland의 창 제어와 labwc Fcitx5 입력기 조합을 구분해서
   기록한다. 자동 실행 성공과 독립 화면 검수 완료는 따로 확인한다.
3. 최종 태그가 검수한 SHA를 가리키는지 확인하고 같은 산출물, dependency sources,
   `linux-build.json`, `SHA256SUMS`를 draft GitHub Release에 올린다.
   모든 SHA-256과 파일 목록을 다시 확인한 뒤 Linux 전용 stable 릴리스를
   `make_latest=false`로 공개한다. 기존 공개 파일을 덮어쓰지 않는다.
4. 공개 URL에서 독립 다운로드해 SHA-256을 확인하고 같은 패키지의 실제 실행을
   확인한다. Linux 설치 링크가 해당 공개 릴리스로 연결되는지 확인한다.
   macOS latest 릴리스, Sparkle appcast와 Homebrew cask가 유지되는지도 확인한다.

사용자 데이터는 패키지 밖의 XDG 경로에 남는다. Linux 업데이트는 해당 릴리스의
수동 다운로드이며 macOS installer를 실행하지 않는다. 렌더링 장치와 실제 확인한
배포판, 표시 서버, 입력기를 [Linux 설치 안내](linux.md)에 기록한다.
소프트웨어 렌더링 검수는 물리 GPU 검수를 뜻하지 않는다.

## macOS 릴리스 운영

release-plz PR이 workspace 버전과 CHANGELOG를 갱신하고, 병합 후 태그를 만든다.
버전과 CHANGELOG를 직접 수정하지 않는다. 태그 push는 후보만 등록하며 빌드나
공개를 실행하지 않는다. 공개 ZIP을 다시 빌드하거나 교체하지 않는다.

이 절차는 [승인된 설계 1.0의 적용·운영](design/2026-10-08-macos-next-release/design.md#7-적용운영복구)을 구현한다.
개인 공개 macOS 앱이며 Seorilabs 모바일 마켓의 증거 형식을 사용하지 않는다.

v0.1.21에 한해 차단 상태를 전달받은 뒤 사용자가 공개를 재지시했다.
[후속 예외 기록](releases/v0.1.21-native-input-exception.md)에 범위와 미완료 항목을
보존한다. schema 2는 정확한 후보와 main의 승인 기록에만 묶이며 결과를
`blocked`로 유지한다. 기존 서명·공증·ZIP·동일 draft 공개 검사는 유지한다.
아래의 정상 schema 1 조건을 통과했다는 뜻이 아니다.

v0.1.22는 사용자가 장시간 검수를 줄이도록 요청한 뒤, 반복 검수와 두 전체 E2E
생략을 선택했다. [한정 예외 기록](releases/v0.1.22-native-validation-exception.md)은
부분 검수와 CHILD-161 완료 후 Unknown 관찰을 보존한다. schema 2의
`native_result: blocked`를 유지하고 실제 signed ZIP의 4개 검사는 여전히
공개 필수 조건이었다. 실제 격리 업데이트의 installer 실패와 업데이트 후 보존
미확인을 전달받은 뒤 사용자가 “공개해”를 재지시한
[추가 예외](releases/v0.1.22-signed-update-exception.md)는 기존 승인과 서명 ZIP을
그대로 유지하고, 공개 증거에 `failed`·`unknown`을 남긴다. 추가 승인은 v0.1.22
공개 직전에만 적용하며 서명·공증·파일 검사는 유지한다. 다른 후보나 미래 버전에
적용하는 skip 옵션은 없다.

v0.1.23은 검수 중단과 미공개 상태를 전달받은 뒤 사용자가 PR 병합·공개를 명시했다.
[이번 공개 지시](releases/v0.1.23-publication-approval.md)와 정확한 후보의 새 승인 기록에만
schema 2를 적용한다. 추가 native 검수와 서명 ZIP의 GUI 검수는 생략하고 미완료를
보존한다. 기존 서명·공증·universal·ZIP·업데이트 서명과 동일 draft 공개는 유지한다.
과거 예외를 재사용하거나 미래 버전에 적용하는 skip 옵션은 없다.

v1.0.25는 부분 검수와 화면 조작 차단을 전달한 뒤 사용자가 공개를 재지시했다.
[이번 버전의 승인 기록](releases/v1.0.25-publication-approval.md)에 정확한 후보와
미완료 범위를 보존하며 `native_result: blocked`를 유지한다. 정상 43조건·13영역의
통과를 뜻하지 않는다. 기존 서명·공증·universal·ZIP·업데이트 서명·동일 draft 공개
검사는 유지하며 다른 후보나 다음 버전에 적용할 수 없다.

For v1.3.0 only, the owner selected publication of candidate `068ef5f` with the
enumerated native checks incomplete. The [candidate-specific approval](releases/v1.3.0-publication-approval.md)
retains `native_result: blocked`; the limited isolated-launcher check does not
establish the normal 43-case and 13-area coverage. Signing, notarization,
universal architectures, update signatures and immutable draft/file checks remain
required. This approval cannot authorize another candidate or future version.

For v1.5.2 only, the owner approved omitting the three full native review rounds and both full native E2E runs for candidate `3e69a85`. The [bounded approval](releases/v1.5.2-publication-approval.md) keeps `native_result: blocked`. The exact signed archive must still pass signed launch, app-owned session-preserving update, GUI IPC and original shell-PID preservation. Signing, notarization, universal architecture, update signatures and immutable draft checks remain required. No other candidate or future version is covered.

v1.6.1의 `443d00f` 후보에 한해 사용자가 전체 native 검수 3회, 전체 E2E 2회와
정확한 signed ZIP의 GUI 실행, 업데이트 뒤 세션 보존, GUI IPC와 shell PID 검사를
미검수로 공개하는 데 동의했다. [이번 승인 기록](releases/v1.6.1-publication-approval.md)은
`native_result: blocked`를 유지한다. CI, 서명, 공증, Gatekeeper, 두 CPU 구조,
업데이트 서명, checksum과 동일 draft 공개는 유지하며 다른 후보에는 적용하지 않는다.

## 버전 정책

`v0.1.24` 다음 릴리스부터 `v1.0.25`로 전환한다. 마지막 자리 `25`는 기존
릴리스 순서를 이어가기 위한 선택이며, 이미 공개한 `0.x` 태그와 기록은 보존한다.

- 호환되는 오류 수정은 patch를 올린다: `1.0.25` → `1.0.26`.
- 호환되는 새 기능은 minor를 올린다: `1.0.25` → `1.1.0`.
- 기존 사용자 인터페이스와 호환되지 않는 변경은 major를 올린다: `1.x` → `2.0.0`.
  호환성 판단에는 CLI 옵션, 설정 파일과 저장된 세션 형식을 포함한다.

버전 전환도 기존 release-plz PR에서 처리한다. 자동 제안과 다른 버전이 필요하면
[공식 `set-version` 명령](https://release-plz.dev/docs/usage/set-version)으로 workspace
버전, `Cargo.lock`과 공유 `CHANGELOG.md`를 함께 갱신한다. 각 크레이트의
`version.workspace = true` 상속은 유지한다. 공유 변경 이력 경로는 모든 크레이트에
지정한 뒤 실행한다. 실제 빌드와 공개에는 아래 릴리스 절차를 그대로 적용한다.

## 후보와 실제 검수

기능 구현과 CI가 통과한 후보를 실제 macOS 개발 실행 환경에서 세 번 검수한다.
각 회차 시작 전에 계획, 기대 결과, 격리 데이터, 설정, 구현자와 다른 리뷰 에이전트,
증거 위치를 기록한다. 모든 신규 조건과 기존 기능을 매 회차 연결한다. headless
scenario, Copilot 리뷰, 단위 테스트는 실제 화면·조작·외부 CLI 증거를 대신하지 않는다.
세 회차 이후 미완료면 승인된 추가 회차를 받기 전 다음 회차를 실행하지 않는다.

release PR의 최종 tree에서 검수한 뒤 squash 병합하면 commit SHA가 달라질 수 있다.
이 경우 각 검수의 원래 SHA를 유지하고 Git의 실제 tree가 최종 후보와 같은지 확인한다.
소스, 에셋, 버전, 의존성, 설정이 달라진 검수는 재사용하지 않는다. 빌드 직전과 공개
직전 E2E는 각각 **최종 후보 commit SHA**의 별도 실행이어야 한다.

private 원본에는 계획·관찰 결과·화면·재현 과정·환경·격리 경로를 보존한다.
공개 JSON에는 허용된 버전, 식별자의 SHA-256, 원본 증거 SHA-256과 통과한 조건만 넣는다.
계정, 개인 경로, prompt, 응답, 환경값, credential은 넣지 않는다. 검증기가 metadata를
통과시켰다는 사실만으로 실제 검수 완료를 주장하지 않는다. 원본을 독립 리뷰한다.

## Release workflow

GitHub Actions의 `Release`를 **main**에서 `workflow_dispatch`로 실행한다.

| 입력 | 내용 |
| --- | --- |
| `stage` | `build`, `publish`, `cask` |
| `tag` | 정확한 stable 태그, 예: `v0.1.21` |
| `sha` | 태그의 전체 40자리 commit SHA. origin/main의 ancestor여야 한다 |
| `config_sha256` | 검수에 사용한 동일 격리 설정의 SHA-256 |
| `native_evidence` | 해당 단계의 공개용 JSON. `cask`에서는 기존 공개 증거를 읽는다 |

기존 Apple certificate, Developer ID, notarization API key, Sparkle key와 공개 key,
Homebrew용 `RELEASE_TOKEN`을 사용한다. 새 credential을 만들거나 교체하지 않는다.
필수 서명 입력이 없으면 빌드가 중단되며 ad-hoc 앱을 릴리스하지 않는다. Actions는
공개 저장소의 GitHub-hosted macOS를 사용한다. runner 용량이나 과금 설정을 바꾸지 않는다.

1. `build`: 세 회차와 별도 `before-build` E2E를 검증한다. runner 큐 대기와 toolchain
   준비 후, bundle 실행 직전에 60분 유효기간을 다시 검사한다. exact tag/SHA의 universal
   앱을 기존 도구로 서명·공증하고 Developer ID, bundle 버전·업데이트 설정, Gatekeeper,
   stapled ticket와 arm64/x86_64를 검사한다. signed bundle의 공개 key로 CryptoKit을 사용해
   ZIP의 실제 Ed25519 업데이트 서명도 확인한다. ZIP, appcast, `before-build.json`,
   `manifest.json`, `SHA256SUMS`를 draft GitHub Release에 보존한다. latest·Homebrew는
   바꾸지 않는다. 이미 존재하는 tag의 assets를 다시 빌드하거나 덮어쓰지 않는다.
2. draft가 생성된 뒤, 동일 후보 SHA의 개발 runtime에서 전체 `before-deploy` E2E를
   다시 실행한다. 다운로드한 signed ZIP의 실제 실행, 업데이트, GUI IPC, 원래 shell PID
   보존을 별도로 검수한다. 사용한 ZIP SHA-256을 기록한다.
3. `publish`: draft assets를 다운로드해 모든 hash·size·appcast·서명·공증을 검증한다.
   동일 검수 세 회차, 별도 두 E2E, build 이후 실행 시점, 정확한 ZIP의 실제 검수와
   fresh `before-deploy`를 요구한다. 공개 직전에 60분 유효기간을 다시 검사한다.
   `before-deploy.json`을 보존하고 **같은 draft**를 공개한다. 최신 stable 버전보다 낮은
   후보를 latest로 지정하지 않는다. latest readback 후 기존 Homebrew cask를 갱신한다.
4. 공개 후 독립 다운로드로 ZIP·appcast·Developer ID·공증·universal·Homebrew hash·Pages와
   실제 GUI/IPC·shell PID를 다시 확인한다. workflow 성공과 공개 후 실제 검증은 따로 기록한다.

## 공개 증거 schema 1

검증기 `scripts/macos-release.py`가 엄격한 필드 목록을 검사한다. 알 수 없는 필드,
중복 JSON key, 64 KiB 초과, 중복 coverage, failure/blocked, future timestamp는 거부한다.
원본 경로나 긴 자유형 설명을 공개 JSON에 넣지 않는다.

| 위치 | 필드 |
| --- | --- |
| 최상위 | `schema: 1`, `platform: macos`, `stage`, `candidate`, `implementer_sha256`, `rounds`, `run` |
| `candidate` | `tag`, `sha`, `tree`, `config_sha256` |
| 모든 실행 | `kind: native-development-runtime`, `result: passed`, UTC `started_at`·`completed_at`, `reviewer_sha256`, `plan_sha256`, `evidence_sha256`, `cases`, `features`, `environment` |
| 각 `rounds` 항목 | 모든 실행 필드와 `number: 1/2/3`, 원래 `sha`, 동일 `tree`, `config_sha256` |
| `environment` | `os: macos`, 숫자로 된 `os_version`·`claude_version`·`codex_version`, `architecture: arm64/x86_64`, `isolated: true` |
| 공개 단계 추가 | `signed_artifact`: 정확한 ZIP `sha256`, 원본 `evidence_sha256`, `result: passed`, `checks` |

`cases`는 승인 설계의 PATH-164, NAV-149, PR-150, TREE-151, REMOVE-152, QUOTA-153,
CLOSE-154, NOTIFY-155, COPY-158, POLICY-159, RESTART-160, CHILD-161, REVIEW-162,
PREP-163, PICKER-181, SESSIONS-182 전체다. v0.1.23 이후에는 승인 설계의 ISSUE-208, ISSUE-209, ISSUE-210, ISSUE-211, ISSUE-212, ISSUE-216, ISSUE-217, ISSUE-219, ISSUE-220, ISSUE-224, ISSUE-226, ISSUE-227, ISSUE-228, ISSUE-229, ISSUE-230, ISSUE-231을 세 회차와 두 별도 E2E에 모두 추가한다. v0.1.21·v0.1.22의 정확한 기존 coverage 계약은 보존하며 과거 예외는 0.1.23에 적용할 수 없다. [16개 인수조건](design/2026-10-12-v0.1.23/acceptance.md)을 실제 실행에 연결한다. `features`는 후보의 product catalog 전체 area다. 새로운 승인 조건이나
catalog area가 생기면 증거 검증 계약도 함께 갱신한다. `signed_artifact.checks`는
`signed-launch`, `session-preserving-update`, `gui-ipc`, `shell-pid-preservation` 전체다.
검증기 입력을 맞추기 위한 가상 통과 JSON을 작성하지 않는다.

`v1.0.25`부터는 위 32개에 ISSUE-241, ISSUE-242, ISSUE-243, ISSUE-244,
ISSUE-246, ISSUE-247, ISSUE-248, ISSUE-249, ISSUE-250, ISSUE-251,
ISSUE-257을 추가한 **43개 조건**을 세 회차와 두 별도 E2E에서 모두 요구한다.
[10월 14일 승인 설계](design/2026-10-14-open-issues/design.md)의 창 크기 복원,
저장소 일괄 접기, 느린 셸 PATH, Claude 사용량 범위, 하위 활동 요약, 테마 쌍,
pane 이동, MCP 읽기·대기, 프로그램 알림, 사용자 명령과
[Codex 시작 수정 #257](https://github.com/magicsih/chda/pull/257)을 각각 연결한다.
이전 태그의 정확한 조건 목록과 기존 예외 범위는 유지한다.

ISSUE-231의 `v1.0.25` 이후 기대 결과는 사용자가 승인한
[10월 15일 종료 동작 개정](decisions/0014-managed-process-restart.md#2026-10-15-개정-종료하면-pane을-닫는다)을 적용한다.
셸은 종료 코드와 무관하게 닫고, 관리 프로그램은 성공하면 닫으며 실패했을 때만
종료 정보·이전 출력·지원되는 정확한 재시작을 유지한다. 기존 Stopped pane의 복원과
살아 있는 셸로 돌아온 에이전트의 동작은 계속 확인한다. 과거 0.1.23의 인수조건 문서는
그대로 보존하되 이후 승인으로 대체된 성공 종료 동작을 회귀 실패로 판정하지 않는다.


For the pending sidebar release, initially prepared as `v1.6.0` at `0857c4b`,
the owner approved replacing COPY-158's
Slack composer check with actual clipboard pastes into both a new TextEdit
plain-text document and an independent browser input. The
[release-specific validation amendment](releases/sidebar-release-clipboard-validation.md)
preserves every other clipboard check and all 43 cases, 13 areas, three native
rounds, both full E2E runs and signed-artifact gates. Record Slack as untested.

## 실패와 재시도

증거가 오래되면 새로운 E2E 뒤 해당 단계를 다시 요청한다. 공개 단계는 빌드를 실행하지
않는다. 공개된 tag의 재실행은 동일 assets·기존 deploy 증거·latest를 확인하며 중복 공개를
하지 않는다. Homebrew만 실패하면 `cask` 단계가 기존 공개 assets와 증거를 다시 검증하고
해당 ZIP으로 cask만 갱신한다. 내용이 이미 일치하면 새 commit을 만들지 않는다.

GitHub asset upload가 일부만 완료된 draft, checksum 불일치, 누락된 증거는 공개하지 않는다.
원본 assets와 정확한 후보를 보존하고 실패를 보고한다. 기존 asset을 `--clobber`로 바꾸거나
증거를 교체하지 않는다. 공개 artifact 문제는 이전 signed 버전 복구 안내와 fix-forward
릴리스로 처리하며 feed/cask rollback 같은 외부 변경은 영향을 확인한다.

로컬 절차 검증:

```sh
python3 scripts/test-macos-release.py
actionlint .github/workflows/release.yml
bash -n scripts/update-cask.sh
```

단위 테스트는 합성 Git 저장소·assets·GitHub/Apple stand-in을 사용한다. 실제 서명,
공증, macOS 실행, 업데이트, 공개 릴리스를 수행하거나 증명하지 않는다.
