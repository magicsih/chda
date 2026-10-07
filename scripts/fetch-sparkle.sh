#!/usr/bin/env bash
# Official, checksum-pinned Sparkle distribution. No signing keys are generated.
set -euo pipefail
cd "$(dirname "$0")/.."
version=2.10.0
sha=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
out="target/sparkle-$version"
if [[ ! -f "$out/.verified-$sha" ]]; then
  mkdir -p "$out"
  archive="$out/Sparkle.tar.xz"
  curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
    "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz" -o "$archive"
  printf '%s  %s\n' "$sha" "$archive" | shasum -a 256 --check
  tar -xJf "$archive" -C "$out"
  touch "$out/.verified-$sha"
fi
printf '%s\n' "$out"
