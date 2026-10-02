#!/usr/bin/env bash
# Build chda.app from a release binary. Signing and notarization happen
# only when the Apple variables are set; otherwise the bundle is ad-hoc
# signed so it still launches locally.
#
#   scripts/bundle-macos.sh [version]
#
# Environment (all optional):
#   CHDA_SIGN_IDENTITY   "Developer ID Application: Name (TEAMID)"
#   Notarization, either an App Store Connect API key:
#     APPLE_API_KEY_P8 (path), APPLE_API_KEY_ID, APPLE_API_ISSUER_ID
#   or an Apple ID:
#     APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD
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
# Shell integration scripts and fonts are embedded in the binary; the font
# licenses travel with the app.
cp crates/chda-ui/assets/fonts/*.txt "$app/Contents/Resources/"

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

notarize=()
if [[ -n "${APPLE_API_KEY_P8:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER_ID:-}" ]]; then
  notarize=(--key "$APPLE_API_KEY_P8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID")
elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
  notarize=(--apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD")
fi
if [[ -n "${CHDA_SIGN_IDENTITY:-}" && ${#notarize[@]} -gt 0 ]]; then
  xcrun notarytool submit "$zip_path" "${notarize[@]}" --wait
  xcrun stapler staple "$app"
  rm -f "$zip_path"
  ditto -c -k --keepParent "$app" "$zip_path"
fi

shasum -a 256 "$zip_path"
echo "$zip_path"
