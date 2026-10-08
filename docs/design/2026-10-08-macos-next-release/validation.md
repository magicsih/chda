# 상세 설계 1.0 문서 검증

2026-10-08, 기준 소스 `15860466e8f2bce168eebe8e975cbb40084f1382`.
검증 대상은 상세 설계와 합성 화면 시안이다. 제품 구현, 실제 앱 검수,
사람의 상세 설계 승인, 빌드·배포 전 E2E는 아직 수행하지 않았다.

- GitHub의 열린 이슈 15개를 다시 조회했다. #14를 제외한 14개가 설계와
  인수조건에 연결되어 있다.
- 사용자가 기존 chda 화면·사이트 스타일을 선택했다. 공유 스타일 설정은
  변경하지 않았다.
- diagram-design `self_check.py`: 통과. 두 SVG의 title/description,
  고유 ID, 문서 탐색 링크를 브라우저에서도 확인했다.
- 공용 Playwright의 세션 전용 탭에서 HeadlessChrome 154.0.0.0으로
  1440px와 640px 화면을 렌더링했다. 페이지 전체의 가로 넘침은 없다.
  작은 화면에서는 도표를 가로로 스크롤하고 리뷰 파일 목록은 선택기로
  접는다. 사용량 팝업은 창의 좌측 상태줄 위에 붙는다.
- 화면을 직접 읽어 작업 화면, 사용량, 권한·재시작, 리뷰·복사,
  준비 설정·실패, 구조·상태 흐름을 비교했다. 시안의 화면 검증은 실제
  GPUI 앱 검수를 대신하지 않는다.
- `python3 scripts/test-product-docs.py`: 6개 통과.
- `python3 scripts/test-appcast.py`: 1개 통과.
- `python3 scripts/sync-product-docs.py --check`: 통과.
- `git diff --check`: 통과. Rust와 제품 설정은 변경하지 않아 로컬 Cargo
  검증은 수행하지 않았다. PR의 저장소 CI 상태는 별도로 확인한다.
- GitHub Wiki 원격이 존재하지 않아 목록은 `docs/design/README.md`를
  사용한다. Wiki에 게시했다고 주장하지 않는다.

화면 증거는 저장소 밖의 세션별 QA 디렉터리에 보존한다. 다음 hash는
합성 설계 시안의 이미지이며 제품 실행 증거가 아니다.

| 증거 | SHA-256 |
| --- | --- |
| `approved-style-design-1440.png` | `71de24897d9729b139e8a6876f94d0d06c778644ee0126d8c24a0a2e6d6860f0` |
| `approved-style-design-640.png` | `d3613fea988f02d963fe8e2b364ce089dcd84c4e1fae6db50e3a9ccec3d5cf68` |

파일명의 `approved-style`은 사용자의 스타일 선택만 가리킨다. 상세 설계의
승인 상태는 여전히 **사람 검토 대기**다.
