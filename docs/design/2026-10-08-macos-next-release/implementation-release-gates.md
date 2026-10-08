# 설계 1.0 — macOS 릴리스 단계 구현

- 적용 근거: 승인 commit `5d14139`, 본문 6·7절. 새로운 배포 승인 버튼이나 모바일 마켓 형식은 추가하지 않는다.
- 범위: 태그 후보 등록, 명시적 build → signed draft → 별도 native E2E → 동일 draft publish,
  공개 asset 검증과 Homebrew repair. 실제 검수·서명·공개 실행은 별도 단계다.
- 정본 운영 안내: [macOS 릴리스 운영](../../releasing.md).

`scripts/macos-release.py`가 source tag/SHA/tree/version/설정과 실제 native 검수 metadata를
검사한다. 세 번의 독립 검수와 모든 승인 조건·기존 catalog 기능, 별도 E2E, 60분 유효기간,
정확한 signed artifact를 요구한다. 공개 metadata는 제한된 필드만 허용한다.
빌드 결과는 private draft에 보존하고 공개 때 다시 빌드하지 않는다. 기존 공개 tag의
파일 교체, 증거 교체, 더 낮은 버전의 latest 지정은 거부한다. Homebrew는 기존 cask에
대한 optimistic update와 readback을 사용하며 같은 내용이면 추가 commit이 없다.

## 검증 기록

- 실제 RED: 유효한 native metadata를 61분 후 재사용했을 때 거부하지 않음. freshness 검사를 추가한 뒤 GREEN.
- 16개 정책 테스트: stale/future·다른 후보/설정/tree·불완전 검수·작성자 자기 검수·
  headless/blocked·빠진 기능·비공개 필드·중복/초과 JSON·다른 ZIP·변경된 assets·
  신규 공개 한 번과 재시도·증거 교체 거부·wrong tag/version·자동 push 금지·큐 대기 만료·
  latest downgrade 거부·cask 무변경/오류/readback. macOS에서 실제 CryptoKit이 합성 archive의
  Ed25519 서명을 확인하고 변경 archive와 다른 공개 key를 거부한다. Linux에서는 이 항목을 skip한다.
- 합성 테스트의 Git identity, archive, GH/Apple 응답은 실제 제품 검수 증거가 아니다.
- Actions stable 태그를 공식 upstream에서 조회하고 release workflow 참조를 해당 commit에 고정했다.
  개인 공개 저장소의 GitHub-hosted macOS를 유지한다. ARC·budget·credential·ruleset 변경 없음.

실제 macOS 3회 검수, 두 E2E, signed draft, 공개 릴리스와 독립 다운로드 검증은 미실행이다.
추가 이슈 #173 설계 1.1은 아직 승인 대기이며 이 PR에서 구현하지 않는다.
