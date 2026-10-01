#!/usr/bin/env bash
# Check that commit subjects (and, given as arguments, extra strings such
# as a pull request title) follow Conventional Commits:
#   <type>(<scope>)?!?: <description>
# Usage: scripts/check-commits.sh <range> [extra subject...]
# release-plz derives versions and the changelog from these prefixes.
set -euo pipefail

pattern='^(feat|fix|perf|refactor|docs|test|build|ci|chore|revert)(\([a-z0-9._/-]+\))?!?: [^ ].*$'
range="$1"
shift

subjects=()
if [ -n "$range" ]; then
  while IFS= read -r line; do
    subjects+=("$line")
  done < <(git log --format=%s --no-merges "$range")
fi
subjects+=("$@")

bad=0
for subject in "${subjects[@]}"; do
  if [[ "$subject" =~ $pattern ]]; then
    echo "ok   $subject"
  else
    echo "FAIL $subject"
    bad=1
  fi
done

if [ "$bad" -ne 0 ]; then
  cat >&2 <<'EOF'

Commit subjects must look like "fix: detect squash merges" or
"feat(sidebar): jump to the waiting agent". Allowed types: feat, fix, perf,
refactor, docs, test, build, ci, chore, revert. Append "!" for breaking
changes. See AGENTS.md.
EOF
  exit 1
fi
