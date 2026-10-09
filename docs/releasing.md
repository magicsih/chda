# macOS 릴리스 운영

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
PREP-163, PICKER-181, SESSIONS-182 전체다. `features`는 후보의 product catalog 전체 area다. 새로운 승인 조건이나
catalog area가 생기면 증거 검증 계약도 함께 갱신한다. `signed_artifact.checks`는
`signed-launch`, `session-preserving-update`, `gui-ipc`, `shell-pid-preservation` 전체다.
검증기 입력을 맞추기 위한 가상 통과 JSON을 작성하지 않는다.

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
