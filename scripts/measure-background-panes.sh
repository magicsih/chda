#!/bin/zsh
# Window redraw cost with busy background panes (#134), macOS only.
#
#   scripts/measure-background-panes.sh <path/to/chda> [tabs]
#
# Starts the given chda binary in a throwaway home with 17 repositories,
# opens `tabs` (default 16) tabs through `chda mcp`, each running a
# Codex-like load (animated OSC title, spinner line, output), then reports
# the app's CPU use and how the main thread spent a 10-second `sample`.
# Compare a release build against the previous release under similar host
# load; run each a few times and alternate them.
set -eu
BIN=${1:?path to a chda binary}
TABS=${2:-16}
ROOT=$(mktemp -d /tmp/chda-bg.XXXX)
trap 'kill $APP 2>/dev/null; pkill -f "$ROOT/home/load.py" 2>/dev/null; rm -rf "$ROOT"' EXIT

mkdir -p $ROOT/home $ROOT/config/chda $ROOT/data $ROOT/src
cat > $ROOT/home/load.py <<'EOF'
import sys, time, itertools
frames = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
for i, f in enumerate(itertools.cycle(frames)):
    sys.stdout.write(f"\033]0;{f} codex | Working\007\r{f} Working ({i // 10}s)")
    if i % 5 == 0:
        sys.stdout.write(f"\r\033[Kline {i}: editing src/module_{i % 7}.rs\n")
    sys.stdout.flush()
    time.sleep(0.1)
EOF
cat > $ROOT/home/.zshrc <<EOF
PS1='%# '
if [[ -f ~/load-on ]]; then exec python3 $ROOT/home/load.py; fi
EOF
repos=()
for i in $(seq -w 1 17); do
  r=$ROOT/src/repo$i
  git init -q -b main $r
  git -C $r -c user.name=m -c user.email=m@m commit -q --allow-empty -m init
  git -C $r worktree add -q -b feat/a-$i $r-a
  git -C $r worktree add -q -b feat/b-$i $r-b
  repos+=("\"$r\"")
done
print "repos = [${(j:, :)repos}]\nrestore-session = false\nupdate-check = false" \
  > $ROOT/config/chda/config.toml

export HOME=$ROOT/home XDG_CONFIG_HOME=$ROOT/config XDG_DATA_HOME=$ROOT/data
cd $ROOT/src
$BIN > $ROOT/app.log 2>&1 &
APP=$!
sleep 6
touch $ROOT/home/load-on
meta='"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"measure","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}'
for i in $(seq -w 1 $TABS); do
  d=$ROOT/src/repo$i-a
  print "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"open_tab\",\"arguments\":{\"path\":\"$d\"},$meta}}" \
    | (cd $d && $BIN mcp) > /dev/null
  sleep 0.3
done
rm -f $ROOT/home/load-on
sleep 15

seconds() { ps -o cputime= -p $APP | awk -F: '{print $1 * 60 + $2}'; }
c0=$(seconds); t0=$(date +%s)
sample $APP 10 1 -file $ROOT/sample.txt > /dev/null 2>&1
c1=$(seconds); t1=$(date +%s)
awk '/main-thread/{f=1} /^    [0-9]+ Thread_/{if (n++ > 0 && f) exit} f' $ROOT/sample.txt > $ROOT/main.txt
largest() { grep -E "$1" $ROOT/main.txt | sed -E 's/^[ +!:|]*//' | awk '{print $1}' | sort -n | tail -1; }
total=$(head -1 $ROOT/main.txt | awk '{print $1}')
idle=$(largest '__CFRunLoopServiceMachPort')
printf "cpu: %.1f%% of one core\n" $(( (c1 - c0) * 100.0 / (t1 - t0) ))
print "main thread: ${total} samples, $(( total - idle )) busy, $(largest '6Window4draw ' || print 0) drawing, $(largest 'compute_layout' || print 0) in layout"
print "host load: $(uptime | awk -F'averages: ' '{print $2}')"
