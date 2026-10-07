#!/usr/bin/env bash
# Embed Sparkle and the process that hosts it. The outer app is signed afterwards.
set -euo pipefail
cd "$(dirname "$0")/.."
app="$1"
if [[ -n "${CHDA_SIGN_IDENTITY:-}" && -z "${CHDA_SPARKLE_PUBLIC_KEY:-}" ]]; then
  echo 'A Developer ID release requires CHDA_SPARKLE_PUBLIC_KEY.' >&2
  exit 1
fi
scripts/fetch-sparkle.sh >/dev/null
sparkle=target/sparkle-2.10.0
mkdir -p "$app/Contents/Frameworks" "$app/Contents/Helpers" "$app/Contents/Resources"
cp "$sparkle/LICENSE" "$app/Contents/Resources/Sparkle-LICENSE.txt"
ditto "$sparkle/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"
xcrun clang -fobjc-arc -Wall -Wextra -Wno-unused-parameter -Werror \
  -arch arm64 -arch x86_64 -mmacosx-version-min=14.0 \
  -F "$sparkle" -framework Cocoa -framework Sparkle \
  -Wl,-rpath,@executable_path/../Frameworks \
  crates/chda-ui/src/platform/update/updater.m -o "$app/Contents/Helpers/chda-updater"
python3 - "$app/Contents/Info.plist" <<'PY'
import base64, os, plistlib, sys
path = sys.argv[1]
with open(path, 'rb') as f:
    info = plistlib.load(f)
info.update(SUFeedURL='https://github.com/magicsih/chda/releases/latest/download/appcast.xml',
            SUEnableAutomaticChecks=False, SUAutomaticallyUpdate=False, SUSendProfileInfo=False)
key = os.environ.get('CHDA_SPARKLE_PUBLIC_KEY', '')
if key:
    if len(base64.b64decode(key, validate=True)) != 32:
        raise SystemExit('CHDA_SPARKLE_PUBLIC_KEY must encode a 32-byte Ed25519 public key')
    info['SUPublicEDKey'] = key
with open(path, 'wb') as f:
    plistlib.dump(info, f)
PY
sign=(--force --sign "${CHDA_SIGN_IDENTITY:--}")
if [[ -n "${CHDA_SIGN_IDENTITY:-}" ]]; then sign+=(--options runtime --timestamp); fi
framework="$app/Contents/Frameworks/Sparkle.framework"
for nested in Versions/B/XPCServices/Installer.xpc Versions/B/XPCServices/Downloader.xpc \
  Versions/B/Updater.app Versions/B/Autoupdate; do
  codesign "${sign[@]}" "$framework/$nested"
done
codesign "${sign[@]}" "$framework"
codesign "${sign[@]}" "$app/Contents/Helpers/chda-updater"
