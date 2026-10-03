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

sha256=$(shasum -a 256 "$zip" | cut -d' ' -f1)
template="$(dirname "$0")/../packaging/homebrew/chda.rb"
content=$(sed -e "s/__VERSION__/$version/" -e "s/__SHA256__/$sha256/" "$template" | base64)
current_sha=$(gh api "repos/$tap/contents/$path" --jq .sha 2>/dev/null || true)

args=(-f message="chda $version" -f content="$content")
if [ -n "$current_sha" ]; then
  args+=(-f sha="$current_sha")
fi
gh api -X PUT "repos/$tap/contents/$path" "${args[@]}" --jq .commit.sha
