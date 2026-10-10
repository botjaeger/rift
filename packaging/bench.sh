#!/usr/bin/env bash
# Repeatable performance and compatibility baseline (docs/performance.md).
#
#   packaging/bench.sh            # release build, then every scenario
#   SHIFT_BIN=path packaging/bench.sh   # measure an existing binary
#
# Every scenario drives the real app through `--script` (no OS input), in
# an isolated config dir, and prints one markdown table row. Numbers are
# wall-clock on the current machine; compare runs on the same host only.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
source packaging/lib.sh

if [[ -z "${SHIFT_BIN:-}" ]]; then
    cargo build --release -p shift-terminal >&2
    SHIFT_BIN="$(shift_default_target_dir)/release/shift-terminal"
fi
[[ -x "$SHIFT_BIN" ]] || shift_die "no binary at $SHIFT_BIN"

WORK="$(mktemp -d)"
if [[ -n "${BENCH_KEEP:-}" ]]; then
    echo "work dir: $WORK" >&2
else
    trap 'rm -rf "$WORK"' EXIT
fi
mkdir -p "$WORK/cfg/shift" "$WORK/data"
# isolated config, data (session file, history) and a window that never
# shows or takes focus (CLAUDE.md: every --script run)
export XDG_CONFIG_HOME="$WORK/cfg" XDG_DATA_HOME="$WORK/data" SHIFT_NO_NOTIFY=1

now_ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time()*1000'; }

# A headless window presents no frames, so key-to-frame latency needs a shown
# one: BENCH_VISIBLE=1 runs that scenario visibly (it takes focus; do not type
# while it runs). Everything else always runs headless.
HEADLESS=--headless

# run_script <name>: runs $WORK/<name>.shift, captures stderr and peak RSS (KiB)
run_script() {
    local name=$1
    if [[ "$(uname)" == Darwin ]]; then
        /usr/bin/time -l "$SHIFT_BIN" --script "$WORK/$name.shift" $HEADLESS >"$WORK/$name.log" 2>&1
        awk '/maximum resident set size/ { printf "%d\n", $1 / 1048576 }' "$WORK/$name.log"
    else
        /usr/bin/time -v "$SHIFT_BIN" --script "$WORK/$name.shift" $HEADLESS >"$WORK/$name.log" 2>&1
        awk -F: '/Maximum resident set size/ { gsub(/ /, "", $2); printf "%d\n", $2 / 1024 }' "$WORK/$name.log"
    fi
}

# duration_ms <dump> <command substring>: the finished block's duration
duration_ms() {
    grep "state=done" "$1" | grep -F "cmd=\"$2" | head -1 |
        sed -E 's/.*duration_ms=Some\(([0-9]+)\).*/\1/'
}

echo "| Scenario | Result | Peak RSS |"
echo "|---|---|---|"

# 1. startup to first prompt (best of 3)
cat >"$WORK/startup.shift" <<'EOF'
wait_prompt
quit
EOF
best=""
for _ in 1 2 3; do
    t0=$(now_ms)
    rss=$(run_script startup)
    t=$(($(now_ms) - t0))
    if [[ -z "$best" || $t -lt $best ]]; then best=$t; fi
done
# the script driver waits 1.5 s before its first step; subtract it
echo "| Startup to first prompt (best of 3) | $((best - 1500)) ms | ${rss} MiB |"

# 2. high-volume output: 500k short lines, and a 20 MiB file
{ seq 1 4000000 || true; } | head -c 20971520 >"$WORK/big.txt" # seq gets SIGPIPE
# each dump right after its command: older blocks leave the scrollback
cat >"$WORK/volume.shift" <<EOF
wait_prompt
type seq 1 500000\n
wait 300
wait_prompt
dump $WORK/volume-seq.txt
type cat $WORK/big.txt\n
wait 300
wait_prompt
dump $WORK/volume.txt
quit
EOF
rss=$(run_script volume)
seq_ms=$(duration_ms "$WORK/volume-seq.txt" "seq 1 500000")
cat_ms=$(duration_ms "$WORK/volume.txt" "cat ")
echo "| \`seq 1 500000\` | ${seq_ms:-?} ms | ${rss} MiB |"
echo "| \`cat\` 20 MiB file | ${cat_ms:-?} ms | (same run) |"

# 3. scrollback memory with a 100k-line history
printf 'scrollback = 100000\n' >"$WORK/cfg/shift/config.toml"
cat >"$WORK/scrollback.shift" <<'EOF'
wait_prompt
type seq 1 300000\n
wait 300
wait_prompt
quit
EOF
rss=$(run_script scrollback)
echo "| 300k lines into 100k scrollback | done | ${rss} MiB |"
rm "$WORK/cfg/shift/config.toml"

# 4. key-to-frame latency of raw keystrokes (cat echoes each key)
{
    echo "wait_prompt"
    printf '%s\n' 'type cat\n'
    echo "wait 500"
    for _ in $(seq 1 40); do
        echo "key x"
        echo "wait 60"
    done
    echo "key ctrl+c"
    echo "wait 200"
    echo "quit"
} >"$WORK/latency.shift"
[[ -n "${BENCH_VISIBLE:-}" ]] && HEADLESS=
SHIFT_LATENCY_LOG=1 RUST_LOG=info run_script latency >/dev/null
HEADLESS=--headless
{ grep -o 'key-to-frame [0-9.]*' "$WORK/latency.log" || true; } | awk '{print $2}' | sort -n >"$WORK/lat.txt"
n=$(wc -l <"$WORK/lat.txt" | tr -d ' ')
if [[ $n -gt 0 ]]; then
    p50=$(awk -v n="$n" 'NR == int((n + 1) / 2)' "$WORK/lat.txt")
    p95=$(awk -v n="$n" 'NR == int(n * 0.95 + 0.999)' "$WORK/lat.txt")
    echo "| Key-to-frame latency ($n keys) | p50 ${p50} ms, p95 ${p95} ms | |"
else
    echo "| Key-to-frame latency | skipped: headless presents no frames (BENCH_VISIBLE=1) | |"
fi

# 5. TUIs enter and leave the alternate screen and keep keyboard input
tui() {
    local name=$1 cmd=$2 quit=$3
    command -v "${cmd%% *}" >/dev/null || { echo "| TUI: $name | not installed | |"; return; }
    cat >"$WORK/tui.shift" <<EOF
wait_prompt
type $cmd\n
wait 1200
dump $WORK/tui-in.txt
$quit
wait 800
dump $WORK/tui-out.txt
quit
EOF
    run_script tui >/dev/null
    if grep -q "alt=true" "$WORK/tui-in.txt" && grep -q "editor_active=true" "$WORK/tui-out.txt"; then
        echo "| TUI: $name | enter + quit ok | |"
    else
        echo "| TUI: $name | FAILED | |"
    fi
}
tui vim "vim -u NONE -N" $'type :q\\n'
tui less "less $WORK/big.txt" "key q"
tui top "top" "key q"
