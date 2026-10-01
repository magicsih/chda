#!/usr/bin/env bash
# Build chda.app from a release binary. Signing and notarization happen
# only when the Apple variables are set; otherwise the bundle is ad-hoc
# signed so it still launches locally.
#
#   scripts/bundle-macos.sh [version]
#
# Environment (all optional):
#   CHDA_SIGN_IDENTITY   "Developer ID Application: Name (TEAMID)"
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD   for notarytool
set -euo pipefail

cd "$(dirname "$0")/.."
version="${1:-$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
out=target/bundle
app="$out/chda.app"

cargo build --release -p chda
rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/chda "$app/Contents/MacOS/chda"
sed "s/__VERSION__/$version/g" resources/Info.plist > "$app/Contents/Info.plist"
python3 scripts/make-icon.py "$out/icon" >/dev/null
cp "$out/icon/chda.icns" "$app/Contents/Resources/chda.icns"
# Shell integration scripts are embedded in the binary; nothing else to copy.

if [[ -n "${CHDA_SIGN_IDENTITY:-}" ]]; then
  codesign --force --options runtime --timestamp \
    --entitlements resources/entitlements.plist \
    --sign "$CHDA_SIGN_IDENTITY" "$app"
else
  codesign --force --sign - "$app"
  echo "ad-hoc signed (set CHDA_SIGN_IDENTITY for Developer ID)" >&2
fi

zip_path="$out/chda-$version-macos-$(uname -m).zip"
ditto -c -k --keepParent "$app" "$zip_path"

if [[ -n "${CHDA_SIGN_IDENTITY:-}" && -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
  xcrun notarytool submit "$zip_path" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_PASSWORD" --wait
  xcrun stapler staple "$app"
  rm -f "$zip_path"
  ditto -c -k --keepParent "$app" "$zip_path"
fi

shasum -a 256 "$zip_path"
echo "$zip_path"
