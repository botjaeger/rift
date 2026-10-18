#!/usr/bin/env bash
# Run only isolated scripted instances; never send OS-wide input.
#
#   scripts/ui-smoke.sh                  # every section, in RIFT_UI_SHARDS (default 2) parallel shards
#   scripts/ui-smoke.sh --only NAME[,NAME]  # only the groups holding these sections (RIFT_UI_ONLY too)
#   RIFT_UI_SHARDS=1 scripts/ui-smoke.sh # the whole file serially, top to bottom
#
# The sections and their groups: python3 scripts/ui-smoke.py BINARY --list
# (scripts/ui_smoke_sections.py). Each shard is its own process with its own
# scratch directory (printed as "UI artifacts:"); its output is in
# <target>/ui-shards/shard-K.log. Every shard runs to its end, so one run
# shows every failure; the run fails when any shard failed.
set -euo pipefail
cd "$(dirname "$0")/.."

only=${RIFT_UI_ONLY:-}
while [[ $# -gt 0 ]]; do
    case $1 in
        --only) [[ $# -ge 2 ]] || { echo "--only needs a section name" >&2; exit 2; }; only=$2; shift 2 ;;
        --only=*) only=${1#--only=}; shift ;;
        *) echo "unknown argument: $1 (scripts/ui-smoke.sh [--only NAME[,NAME]])" >&2; exit 2 ;;
    esac
done
shards=${RIFT_UI_SHARDS:-2}
[[ $shards =~ ^[1-9][0-9]*$ ]] || { echo "RIFT_UI_SHARDS=$shards: expected a positive number" >&2; exit 2; }

if [ -z "${RIFT_TEST_BIN:-}" ]; then
    cargo build -p rift-terminal --locked
    export RIFT_TEST_BIN="${CARGO_TARGET_DIR:-target}/debug/rift-terminal"
fi
# a scripted phone for the remote-access checks (crates/rift-remote/examples)
if [ -z "${RIFT_PHONE_BIN:-}" ]; then
    cargo build -p rift-remote --example phone --locked
    export RIFT_PHONE_BIN="${CARGO_TARGET_DIR:-target}/debug/examples/phone"
fi
# the relay for the Phase 2 checks (crates/rift-remote/src/bin)
if [ -z "${RIFT_RELAY_BIN:-}" ]; then
    cargo build -p rift-remote --bin rift-relay --locked
    export RIFT_RELAY_BIN="${CARGO_TARGET_DIR:-target}/debug/rift-relay"
fi

if [[ -n $only ]]; then
    exec python3 scripts/ui-smoke.py "$RIFT_TEST_BIN" --only "$only"
fi
if [[ $shards -eq 1 ]]; then
    exec python3 scripts/ui-smoke.py "$RIFT_TEST_BIN"
fi

target=${CARGO_TARGET_DIR:-target}
out="$target/ui-shards"
# a read-only checkout (the linux gate's UI container): a temporary folder
if ! mkdir -p "$out" 2>/dev/null || [[ ! -w $out ]]; then
    out=$(mktemp -d "${TMPDIR:-/tmp}/rift-ui-shards.XXXXXX")
fi
rm -f "$out"/shard-*.log "$out"/timings-*.json
# every shard plans from the same text and timings, whatever changes meanwhile
cp scripts/ui-smoke.py "$out/ui-smoke.source.py"
timings="$target/ui-smoke-timings.json"
if [[ -f $timings ]]; then cp "$timings" "$out/timings.in.json"; else echo '{}' >"$out/timings.in.json"; fi

pids=()
stop() {
    for p in ${pids[@]+"${pids[@]}"}; do kill -TERM "$p" 2>/dev/null || true; done
    exit 130
}
trap stop INT TERM
t0=$SECONDS
echo "ui: $shards shards in parallel; logs in $out/shard-K.log"
for ((k = 1; k <= shards; k++)); do
    RIFT_UI_SOURCE="$out/ui-smoke.source.py" RIFT_UI_TIMINGS="$out/timings.in.json" \
        RIFT_UI_TIMINGS_OUT="$out/timings-$k.json" \
        python3 -u scripts/ui-smoke.py "$RIFT_TEST_BIN" --shard "$k/$shards" >"$out/shard-$k.log" 2>&1 &
    pids+=($!)
done

# report each shard the moment it ends (no `wait -n`: macOS ships bash 3.2)
failed=0
declare -a done_at=()
left=$shards
while ((left > 0)); do
    for ((k = 1; k <= shards; k++)); do
        [[ -n ${done_at[k]:-} ]] && continue
        pid=${pids[k - 1]}
        kill -0 "$pid" 2>/dev/null && continue
        rc=0
        wait "$pid" || rc=$?
        done_at[k]=$((SECONDS - t0))
        left=$((left - 1))
        log="$out/shard-$k.log"
        if ((rc == 0)); then
            echo "-- shard $k/$shards pass ${done_at[k]}s ($(grep -m1 '^UI artifacts:' "$log" || true))"
        else
            failed=1
            echo "-- shard $k/$shards FAIL ${done_at[k]}s (exit $rc; $log)"
            if grep -q '^FAIL ' "$log"; then
                # each failed group: its name, section and the traceback tail
                grep -E '^(FAIL |  \| )' "$log" | sed 's/^/   /'
            else
                tail -n 25 "$log" | sed 's/^/   | /'
            fi
        fi
    done
    ((left > 0)) && sleep 1
done
trap - INT TERM

python3 scripts/ui_smoke_sections.py merge-timings "$timings" "$out"/timings-*.json 2>/dev/null || true
if ((failed)); then
    echo "ui: FAIL after $((SECONDS - t0))s (rerun one: scripts/gates.sh ui --only <section>)"
    exit 1
fi
echo "UI regressions passed: $shards shards in $((SECONDS - t0))s"
