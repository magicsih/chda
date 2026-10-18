#!/bin/sh
set -eu
if [ -f /opt/chda-qa-mesa/manifest.json ]; then
    set -- /opt/chda-qa-mesa/share/vulkan/icd.d/lvp_icd*.json
    [ "$#" = 1 ] && [ -f "$1" ]
    export VK_ICD_FILENAMES="$1"
fi
mkdir -p /work/packages /work/payload /evidence
cp /packages/* /work/packages/
(cd /work/packages && sha256sum --check SHA256SUMS)
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
    'testHostDriver': json.loads(Path('/opt/chda-qa-mesa/manifest.json').read_text())
        if Path('/opt/chda-qa-mesa/manifest.json').exists() else 'distribution packages',
}, indent=2))
PY
chown -R chda-test:chda-test /work /evidence
run_session() {
runuser -u chda-test -- python3 /driver/native_session.py \
    --launcher "$launcher" --artifact "$artifact" --candidate-sha "$CANDIDATE_SHA" \
    --backend "${DISPLAY_BACKEND:-x11}" --round "${REVIEW_ROUND:-1}" \
    --evidence /evidence/session

}
if [ "${EXPECT_GPU_UNAVAILABLE:-0}" != 0 ]; then
    set +e
    run_session
    driver_exit=$?
    set -e
    DRIVER_EXIT="$driver_exit" python3 - <<'PYNEGATIVE'
import json, os
from pathlib import Path
root = Path('/evidence/session')
result = json.loads((root / 'session-result.json').read_text())
log = (root / 'candidate.log').read_text()
assert int(os.environ['DRIVER_EXIT']) != 0
assert result['startupStatus'] == 'failed'
assert 'No GPU adapter found that can configure the display surface' in log
if os.environ['EXPECT_GPU_UNAVAILABLE'] == 'mesa20':
    assert log.count('no compatible surface formats') >= 2
    reason = 'Stock Mesa20 software Vulkan and EGL expose no compatible X11 surface formats'
elif os.environ['EXPECT_GPU_UNAVAILABLE'] == 'mesa21':
    assert "Limit 'max_color_attachments' value 4 is better than allowed 2" in log
    reason = 'Stock Mesa21 software Vulkan adapter limits are below the graphics engine request'
else:
    raise AssertionError('Unknown negative graphics fixture')
assert 'GLIBC_' not in log and 'symbol lookup error' not in log
Path('/evidence/expected-graphics-failure.json').write_text(json.dumps({
    'negativeCheckStatus': 'passed', 'productStartupStatus': 'failed',
    'supportedEnvironment': False,
    'reason': reason,
    'candidateSha': result['candidateSha'], 'artifactSha256': result['artifactSha256'],
}, indent=2) + '\n')
PYNEGATIVE
else
    run_session
fi
