#!/bin/bash
# SessionStart hook for Claude Code cloud sessions. Prepares what chda needs to
# lint and test: the system libraries CI installs, the pinned Zig, the pinned
# Ghostty source and the Cargo dependencies. Each step checks before it acts, so
# a rerun in a cached container is quick. Runs only when CLAUDE_CODE_REMOTE=true.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

# Build output goes to a log; stdout carries only the summary that Claude sees.
LOG="$HOME/.cache/chda/session-start.log"
mkdir -p "$(dirname "$LOG")"
exec 3>&1 >"$LOG" 2>&1
trap 'echo "chda session hook: a step failed; see $LOG" >&3' ERR

cd "$CLAUDE_PROJECT_DIR"
export DEBIAN_FRONTEND=noninteractive

# System libraries from the Linux CI job (.github/workflows/ci.yml).
APT_PACKAGES=(libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libfontconfig-dev libvulkan1 libasound2-dev zsh fish git)
missing=()
for package in "${APT_PACKAGES[@]}"; do
  status=$(dpkg-query -W -f='${Status}' "$package" 2>/dev/null || true)
  if [ "$status" != "install ok installed" ]; then
    missing+=("$package")
  fi
done
if [ "${#missing[@]}" -gt 0 ]; then
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends \
    -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold \
    "${missing[@]}"
fi

# Zig 0.16.0 comes from PyPI rather than ziglang.org: pip verifies the wheel hash,
# and pypi.org is on the default Trusted allowlist.
ZIG_VERSION=0.16.0
ZIG_HOME="$HOME/.cache/chda/zig-$ZIG_VERSION"
if [ "$("$ZIG_HOME/bin/zig" version 2>/dev/null || true)" != "$ZIG_VERSION" ]; then
  python3 -m venv "$ZIG_HOME"
  "$ZIG_HOME/bin/pip" install --quiet --disable-pip-version-check "ziglang==$ZIG_VERSION"
  printf '#!/bin/sh\nexec "%s/bin/python" -m ziglang "$@"\n' "$ZIG_HOME" > "$ZIG_HOME/bin/zig"
  chmod +x "$ZIG_HOME/bin/zig"
fi
export PATH="$ZIG_HOME/bin:$PATH"
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo "export PATH=\"$ZIG_HOME/bin:\$PATH\"" >> "$CLAUDE_ENV_FILE"
fi

# The Ghostty archive comes from codeload.github.com, which a session may use only
# when ghostty-org/ghostty is attached to it. Without it, report that and continue:
# chda-config, chda-git and chda-core build without libghostty.
ghostty_status=ready
if ! ghostty_log=$(python3 scripts/prepare-ghostty.py 2>&1); then
  ghostty_status="NOT prepared ($(echo "$ghostty_log" | tail -n 1))"
fi

# rust-toolchain.toml pins Rust 1.98.1 with rustfmt and clippy; rustup installs them on first use.
cargo fetch --locked

echo "chda session hook: ready. zig $ZIG_VERSION, cargo dependencies fetched, Ghostty $ghostty_status." >&3
