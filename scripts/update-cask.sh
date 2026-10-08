#!/usr/bin/env bash
# Fill packaging/homebrew/chda.rb with a version and the zip's sha256 and
# push it to magicsih/homebrew-tap. Usage:
#   scripts/update-cask.sh 0.1.3 target/bundle/chda-0.1.3-macos-universal.zip
# Needs GH_TOKEN with contents:write on the tap repository.
set -euo pipefail
version="$1"
zip="$2"
tap="${CHDA_TAP_REPO:-magicsih/homebrew-tap}"
path="Casks/chda.rb"

[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
test -f "$zip"
latest=$(gh api repos/magicsih/chda/releases/latest --jq .tag_name)
test "$latest" = "v$version"
sha256=$(shasum -a 256 "$zip" | cut -d' ' -f1)
template="$(dirname "$0")/../packaging/homebrew/chda.rb"
rendered=$(sed -e "s/__VERSION__/$version/" -e "s/__SHA256__/$sha256/" "$template")
metadata=$(mktemp)
trap 'rm -f "$metadata"' EXIT
# An authentication/network error must not be mistaken for a missing cask.
gh api "repos/$tap/contents/$path" > "$metadata"
current=$(python3 -c 'import base64,json,sys; print(base64.b64decode(json.load(open(sys.argv[1]))["content"]).decode(), end="")' "$metadata")
if [ "$current" = "$rendered" ]; then
  printf 'Homebrew already matches chda %s and its archive checksum\n' "$version"
  exit 0
fi
content=$(printf '%s\n' "$rendered" | base64)
current_sha=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["sha"])' "$metadata")
test -n "$current_sha"

args=(-f message="chda $version" -f content="$content" -f sha="$current_sha")
gh api -X PUT "repos/$tap/contents/$path" "${args[@]}" --jq .commit.sha
confirmed=$(gh api "repos/$tap/contents/$path" --jq .content | base64 --decode)
test "$confirmed" = "$rendered"
