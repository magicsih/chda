#!/bin/sh
set -eu
# Test-host Vulkan driver, never part of the application payload. Retain glibc
# 2.31 while exercising the documented compatible-driver requirement.
[ "$(uname -m)" = x86_64 ]
export DEBIAN_FRONTEND=noninteractive
apt-get install -y --no-install-recommends \
    build-essential pkg-config cmake ninja-build curl xz-utils binutils \
    llvm-12-dev libexpat1-dev zlib1g-dev libzstd-dev libelf-dev bison flex \
    libx11-xcb-dev libxcb-dri2-0-dev libxcb-dri3-dev libxcb-present-dev libxcb-randr0-dev \
    libxcb-sync-dev libxcb-xfixes0-dev libxcb-shm0-dev libxshmfence-dev
curl --fail --location --retry 3 -o /tmp/qa-python.tar.gz \
    https://github.com/astral-sh/python-build-standalone/releases/download/20261013/cpython-3.13.16%2B20261013-x86_64-unknown-linux-gnu-install_only.tar.gz
echo 'e588e50db882fa6a89f926d9a9ad64e445183af0eda5bda226a2c151b17db8a6  /tmp/qa-python.tar.gz' | sha256sum --check
tar -C /opt -xzf /tmp/qa-python.tar.gz
export PATH="/opt/python/bin:/usr/lib/llvm-12/bin:$PATH"
python3 -m pip install --require-hashes --only-binary=:all: -r /driver/mesa-build-requirements.txt
# X11 Vulkan needs a newer libdrm than focal provides. Link a private static
# copy into this QA driver so the distribution's shared libraries stay intact.
curl --fail --location --retry 3 -o /tmp/qa-libdrm.tar.xz \
    https://dri.freedesktop.org/libdrm/libdrm-2.4.120.tar.xz
echo '3bf55363f76c7250946441ab51d3a6cc0ae518055c0ff017324ab76cdefb327a  /tmp/qa-libdrm.tar.xz' | sha256sum --check
tar -C /tmp -xf /tmp/qa-libdrm.tar.xz
meson setup /tmp/libdrm-2.4.120/build /tmp/libdrm-2.4.120 \
    --prefix=/opt/chda-qa-mesa --libdir=lib --buildtype=release \
    --default-library=static --wrap-mode=nofallback \
    -Dauto_features=disabled -Db_staticpic=true -Dtests=false -Dinstall-test-programs=false
ninja -C /tmp/libdrm-2.4.120/build -j2
meson install -C /tmp/libdrm-2.4.120/build
export PKG_CONFIG_PATH=/opt/chda-qa-mesa/lib/pkgconfig
curl --fail --location --retry 3 -o /tmp/qa-mesa.tar.xz https://archive.mesa3d.org/mesa-23.3.6.tar.xz
echo 'cd3d6c60121dea73abbae99d399dc2facaecde1a8c6bd647e6d85410ff4b577b  /tmp/qa-mesa.tar.xz' | sha256sum --check
tar -C /tmp -xf /tmp/qa-mesa.tar.xz
meson setup /tmp/mesa-23.3.6/build /tmp/mesa-23.3.6 \
    --prefix=/opt/chda-qa-mesa --libdir=lib --buildtype=release --wrap-mode=nofallback \
    -Dplatforms=x11 -Dvulkan-drivers=swrast -Dgallium-drivers=swrast \
    -Dopengl=false -Degl=disabled -Dglx=disabled -Dgles1=disabled -Dgles2=disabled \
    -Dgbm=disabled -Dglvnd=false -Dgallium-rusticl=false \
    -Dllvm=enabled -Dshared-llvm=enabled \
    -Dbuild-tests=false -Dvideo-codecs= \
    -Dvalgrind=disabled -Dlibunwind=disabled
ninja -C /tmp/mesa-23.3.6/build -j2
meson install -C /tmp/mesa-23.3.6/build
python3 - <<'PY'
import json, re, subprocess
from pathlib import Path
prefix = Path('/opt/chda-qa-mesa')
icds = list(prefix.glob('share/vulkan/icd.d/lvp_icd*.json'))
assert len(icds) == 1, icds
libc = subprocess.check_output(['ldd', '--version'], text=True).splitlines()[0]
assert '2.31' in libc, libc
driver = prefix / 'lib/libvulkan_lvp.so'
versions = subprocess.check_output(['readelf', '--version-info', str(driver)], text=True)
floor = max(tuple(map(int, item.split('.'))) for item in re.findall(r'GLIBC_([0-9.]+)', versions))
assert floor <= (2, 31), floor
(prefix / 'manifest.json').write_text(json.dumps({
    'scope': 'test host only; absent from release payload', 'mesaVersion': '23.3.6',
    'sourceUrl': 'https://archive.mesa3d.org/mesa-23.3.6.tar.xz',
    'sourceSha256': 'cd3d6c60121dea73abbae99d399dc2facaecde1a8c6bd647e6d85410ff4b577b',
    'privateLibdrm': '2.4.120; static; test-host driver only',
    'libdrmSourceSha256': '3bf55363f76c7250946441ab51d3a6cc0ae518055c0ff017324ab76cdefb327a',
    'libc': libc, 'driverRequiredGlibc': '.'.join(map(str, floor)),
    'icd': str(icds[0]), 'llvm': '12',
}, indent=2) + '\n')
PY
