# ADR 0022: Git fallback for the pinned Ghostty source

Status: accepted, 2026-10-20.

## Decision

`scripts/prepare-ghostty.py` still tries the pinned codeload archive first, so a
normal machine behaves as before. When that download fails with an `OSError`,
for example a 403 from codeload.github.com in a session that has not attached
`ghostty-org/ghostty`, the script fetches the pinned commit with
`git fetch --depth 1` from the public repository and exports it with
`git archive` under the same `ghostty-<commit>/` root. If git also fails, the
error reports both causes.

The pins in `resources/ghostty/source.json` do not change. Git names the commit
by its object ID, and the script requires it to resolve to `ghostty_commit`. The
extracted tree must still match `source_sha256` after `terminal-integrity.patch`
applies, and that patch must match `patch_sha256`. `archive_sha256` applies only
to the codeload bytes, since git's tar is not codeload's gzip.

Git runs with `core.autocrlf=false` and `core.eol=lf`, so a Windows or
user-level line-ending setting cannot change file bytes. Any byte change would
fail the tree digest. An archive that downloads but fails `archive_sha256`
remains a hard error and never falls back to git.

## Trade-offs

- The transport is no longer the trust root. The tree digest, not codeload or
  GitHub, decides whether the source is used, so both paths gate the same bytes.
- The fallback works in sessions where codeload is refused but public git
  access is allowed. It needs no credentials and no per-session repository
  attachment.
- The fallback downloads the commit's depth-1 object set (about 33 MB here) and
  needs `git` on `PATH`. A failed fetch surfaces as a preparation error, so
  nothing builds from an unverified source.
- For the pinned commit, `git archive` yields the same tree as codeload. The
  commit has no `export-ignore` or `export-subst` attributes, and the tree digest
  matches. A future pin that adds such attributes would make the digest check
  fail rather than pass silently.

Rejected alternatives: attaching the repository to every session (a manual step
per session), vendoring the Ghostty tree into this repository (size and
maintenance), and skipping the Ghostty build when the source is unavailable
(leaves `chda-term`, `chda-ui` and `chda` unbuildable).
