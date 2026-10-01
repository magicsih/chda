#!/usr/bin/env bash
# Print the CHANGELOG section for a version: scripts/release-notes.sh v0.1.2
set -euo pipefail
version="${1#v}"
awk -v v="$version" '
  /^## \[/ { in_section = ($0 ~ "^## \\[" v "\\]") ; if (in_section) next }
  in_section && /^\[/ { exit }
  in_section { print }
' "$(dirname "$0")/../CHANGELOG.md" | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'
