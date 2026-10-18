# Approval

- Date: 2026-10-18.
- Design: version 1.0 of [design.md](design.md), proposed against
  `8f6f82224ecb7cbb1b2623737d8e17c04335d4c2`, with [test-plan.json](test-plan.json).
- User direction: prioritize broad distribution support.
- User instruction: "구현 진행하고 배포까지 가자."
- Scope: implement the Linux x86_64 design, run the L3 plan, create and merge
  reviewed PRs, prepare verified AppImage and portable packages, and publish
  the Linux release. The previous PR-only endpoint is superseded.
- Keep code, automated tests, native Linux execution, package verification and
  public download verification as separate evidence. Do not reuse earlier
  macOS native-validation exceptions for this candidate.
- No direct-main exception was requested. Existing Windows scope is unchanged.
