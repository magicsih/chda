#!/bin/sh
set -eu
mkdir -p /work/packages /work/payload /evidence
cp /packages/* /work/packages/
(cd /work/packages && sha256sum --check SHA256SUMS)
python3 - <<'PY'
import json, os
from pathlib import Path
metadata = json.loads(Path('/work/packages/linux-build.json').read_text())
assert metadata['sha'] == os.environ['CANDIDATE_SHA'], 'Package and requested candidate differ'
assert metadata['architecture'] == 'x86_64'
PY
set -- /work/packages/*-linux-x86_64.tar.gz
[ "$#" = 1 ] && [ -f "$1" ]
archive=$1
tar -xzf "$archive" -C /work/payload
case "$PACKAGE_FORMAT" in
    archive)
        set -- /work/payload/*/AppRun
        launcher=$1
        artifact=$archive
        ;;
    appimage)
        set -- /work/packages/*-linux-x86_64.AppImage
        launcher=$1
        artifact=$1
        chmod +x "$launcher"
        ;;
    *) exit 1 ;;
esac
python3 - <<'PY' > /evidence/environment.json
import json, os, platform, subprocess
from pathlib import Path
print(json.dumps({
    'image': os.environ['NATIVE_IMAGE'], 'runnerArchitecture': os.environ['RUNNER_ARCH'],
    'osRelease': Path('/etc/os-release').read_text(), 'uname': platform.uname()._asdict(),
    'libc': subprocess.check_output(['ldd', '--version'], stderr=subprocess.STDOUT).decode(),
    'format': os.environ['PACKAGE_FORMAT'], 'rendering': 'CPU Vulkan in a disposable container',
    'mountMode': 'extract-and-run for AppImage; no FUSE device supplied',
}, indent=2))
PY
chown -R chda-test:chda-test /work /evidence
exec runuser -u chda-test -- python3 /driver/native_session.py \
    --launcher "$launcher" --artifact "$artifact" --candidate-sha "$CANDIDATE_SHA" \
    --backend "${DISPLAY_BACKEND:-x11}" --round "${REVIEW_ROUND:-1}" \
    --compositor "${COMPOSITOR:-mutter}" --stage "${NATIVE_STAGE:-startup}" \
    --evidence /evidence/session
