#!/usr/bin/env bash
# shellcheck disable=SC2317,SC2329  # gate_* functions are called as "gate_$g" (SC2317 on shellcheck <0.10)
# Rift quality gates: one runner for local checks and CI.
#
#   scripts/gates.sh                 # default set: fmt clippy test doc shell docs mobile
#   scripts/gates.sh all             # default set + release + ui + linux
#   scripts/gates.sh fmt clippy      # only the named gates
#   scripts/gates.sh ui --only NAME  # only these ui-smoke.py sections (scripts/ui-smoke.sh)
#   scripts/gates.sh --list          # describe every gate
#
# Every requested gate runs even after a failure, so one pass shows every
# problem; the exit status is non-zero when any gate failed. A gate whose
# tool is missing FAILS (it did not run) rather than passing silently.
#
# Output: one line per gate and the summary; each gate's full output is in
# target/gates/<gate>.log, and a failure prints the end of it.
# RIFT_GATES_VERBOSE=1 (and CI) streams every gate's output as well.
# Every gate has a wall-clock limit (see limit_for): a hung test fails its
# gate instead of stalling the run. RIFT_GATE_TIMEOUT=<seconds> overrides it.
#
# The compiler comes from rust-toolchain.toml, so local runs, CI and the
# Linux docker image all use the same clippy and rustfmt.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

DEFAULT=(fmt clippy test doc shell docs mobile)
ALL=(fmt clippy test doc shell docs mobile release ui leak linux)

describe() {
    cat <<'EOF'
fmt      cargo fmt --all --check, and on each vendored crate
clippy   cargo clippy --workspace --all-targets --locked -- -D warnings, on each vendored crate (all features), and on rift-remote without default features (the relay image)
test     cargo test --workspace --locked, and each vendored crate (all features); release-manifest.py --self-test;
         locally on macOS the workspace tests run throttled (background QoS, 32 threads) like a loaded CI runner
doc      cargo doc --workspace --no-deps --locked, rustdoc warnings denied
shell    shellcheck on packaging/*.sh, scripts/*.sh, .githooks/*, bash hooks; zsh -n and fish --no-execute on their hooks
docs     CLAUDE.md and AGENTS.md identical; every relative link in *.md resolves (vendored crates' upstream docs excepted)
mobile   the Expo companion in mobile/: pnpm install --frozen-lockfile, typecheck, eslint, vitest (incl. the shared protocol fixtures)
release  cargo build --release --locked
ui       isolated scripted UI regressions (display or xvfb-run required), RIFT_UI_SHARDS (default 2) parallel shards
         of scripts/ui-smoke.py's independent section groups; --only NAME[,NAME] runs only those sections' groups
         (python3 scripts/ui-smoke.py BINARY --list names them); RIFT_UI_SHARDS=1 runs the file serially
leak     memory soak on the release build: repeated identical work (a ticking program, output past scrollback, a command printing lines without end,
         opening and closing tabs, splits and overlays) must level off (scripts/leak-soak.py; ~5 min)
linux    clippy + test + doc + release build in packaging/linux-verify.Dockerfile, then shell + scripted UI (xvfb, RIFT_UI_SHARDS shards) + leak on the CI runner's packages in packaging/ci-ubuntu.Dockerfile (docker)
EOF
}

have() { command -v "$1" >/dev/null 2>&1; }

# The vendored vte and alacritty_terminal (patched in through
# [patch.crates-io]) are workspaces of their own: every gate that checks the
# workspace runs on them too, built into target/vendor.
VENDORED=(vendor/vte vendor/alacritty_terminal)
vendored() {
    local sub=$1 crate rc=0
    shift
    for crate in "${VENDORED[@]}"; do
        # CI checks out only tracked files: an untracked lockfile passes
        # `--locked` here and fails there
        if ! git ls-files --error-unmatch "$crate/Cargo.lock" >/dev/null 2>&1; then
            echo "$crate/Cargo.lock is not tracked by git (CI's --locked needs it)" >&2
            rc=1
            continue
        fi
        CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}/vendor" \
            cargo "$sub" --manifest-path "$crate/Cargo.toml" "$@" || rc=1
    done
    return "$rc"
}

gate_fmt() { cargo fmt --all --check && vendored fmt --check; }
gate_clippy() {
    cargo clippy --workspace --all-targets --locked -- -D warnings &&
        vendored clippy --all-targets --all-features --locked -- -D warnings &&
        # the relay image's build (packaging/relay.Dockerfile): no TLS
        cargo clippy -p rift-remote --no-default-features --all-targets --locked -- -D warnings
}
gate_test() {
    workspace_test && vendored test --all-features --locked &&
        python3 scripts/release-manifest.py --self-test
}

# CI's runners are a few slow cores: timing a fast Mac never misses (a 50 ms
# read timeout over a WebSocket upgrade) fails there. Locally on macOS the
# workspace tests run once, on the efficiency cores at background priority
# with many threads, which reproduces it (the build before them runs at full
# speed). CI and Linux run them plainly.
workspace_test() {
    if [[ -n "${CI:-}" || "$(uname)" != Darwin ]]; then
        cargo test --workspace --locked
        return
    fi
    cargo test --workspace --locked --no-run -q || return 1
    echo "-- workspace tests, throttled (taskpolicy -c background, 32 threads)"
    taskpolicy -c background cargo test --workspace --locked -q -- --test-threads=32
}
gate_doc() { RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked; }
gate_release() { cargo build --release --locked; }
gate_ui() { scripts/ui-smoke.sh; }
gate_leak() {
    if [[ -z ${RIFT_TEST_BIN:-} ]]; then
        cargo build --release -p rift-terminal --locked || return 1
    fi
    python3 scripts/leak-soak.py "${RIFT_TEST_BIN:-${CARGO_TARGET_DIR:-target}/release/rift-terminal}"
}

gate_shell() {
    have shellcheck || { echo "shellcheck not installed (brew install shellcheck / apt install shellcheck)"; return 1; }
    shellcheck packaging/*.sh scripts/*.sh .githooks/* shell/rift.bash shell/rift-bashrc shell/rift-bashrc-child shell/complete.bash || return 1
    # zsh and fish hooks: syntax only (shellcheck has no zsh/fish mode).
    # A missing shell is a failure: the hook would ship unchecked.
    local rc=0
    # packages are named for their target, not the build host (the Intel
    # Mac app is cross-built on Apple silicon)
    local t want got
    for t in aarch64-apple-darwin=macos-arm64 x86_64-apple-darwin=macos-x86_64 \
        x86_64-unknown-linux-gnu=linux-x86_64 aarch64-unknown-linux-gnu=linux-aarch64; do
        want="${t#*=}"
        got="$(. packaging/lib.sh && echo "$(rift_target_os "${t%%=*}")-$(rift_target_arch "${t%%=*}")")"
        [ "$got" = "$want" ] || { echo "packaging/lib.sh: ${t%%=*} is named $got, not $want"; rc=1; }
    done
    # one file per run: further arguments are the script's, not files
    local f
    for f in shell/rift.zsh shell/complete.zsh; do
        if have zsh; then zsh -n "$f" || rc=1; else echo "zsh not installed: $f unchecked"; rc=1; fi
    done
    for f in shell/rift.fish shell/complete.fish; do
        if have fish; then fish --no-execute "$f" || rc=1; else echo "fish not installed: $f unchecked"; rc=1; fi
    done
    return "$rc"
}

gate_docs() {
    local rc=0 md link target
    if ! cmp -s CLAUDE.md AGENTS.md; then
        echo "CLAUDE.md and AGENTS.md differ (they must be identical copies)"
        rc=1
    fi
    # Relative markdown links: [text](path) and [text](path#anchor).
    while IFS= read -r md; do
        while IFS= read -r link; do
            target="${link%%#*}"
            # a space in a path is written %20 (GitHub resolves it)
            target="${target//%20/ }"
            [[ -z $target ]] && continue
            # GitHub page routes (../../releases from the README) are not files.
            [[ $target =~ ^(\.\./)+(releases|issues|pulls|actions|wiki)(/|$) ]] && continue
            if [[ ! -e "$(dirname "$md")/$target" ]]; then
                echo "$md: broken link ($link)"
                rc=1
            fi
        done < <(grep -o '\]([^)]*)' "$md" | sed 's/^](//; s/)$//' | grep -v -E '^(https?|mailto):|^#' || true)
    # vendored crates keep their upstream docs, whose links point into
    # repositories not copied here
    done < <(git ls-files --cached --others --exclude-standard '*.md' ':!vendor/*/*')
    return "$rc"
}

gate_mobile() {
    have pnpm || { echo "pnpm not installed (corepack enable)"; return 1; }
    (cd mobile && pnpm install --frozen-lockfile && pnpm typecheck && pnpm lint && pnpm test)
}

gate_linux() {
    have docker || { echo "docker not installed"; return 1; }
    # beside the local gates (see the loop): leave two cores to them
    local cpus=()
    [[ -n ${LINUX_CPUS:-} ]] && cpus=(--cpus "$LINUX_CPUS")
    phase "image (linux-verify)" \
        docker build -q -t rift-linux-verify -f packaging/linux-verify.Dockerfile packaging || return 1
    phase "clippy test doc + release builds" \
        docker run --rm --init ${cpus[@]+"${cpus[@]}"} -v "$PWD:/src" -w /src \
        -v rift-cargo-registry:/usr/local/cargo/registry \
        -e CARGO_TARGET_DIR=/src/target/linux-docker \
        rift-linux-verify bash -c '
            scripts/gates.sh clippy test doc &&
            cargo build --release --locked &&
            cargo build --release --locked -p rift-remote --example phone --bin rift-relay' || return 1
    # The shell and UI gates run on the CI runner's packages, with the
    # environment of ci.yml's UI step: a library, font or shellcheck version
    # the runner differs on fails here first.
    phase "image (ci-ubuntu)" \
        docker build -q -t rift-ci-ubuntu -f packaging/ci-ubuntu.Dockerfile packaging || return 1
    phase "shell + ui + leak (xvfb)" \
        docker run --rm --init ${cpus[@]+"${cpus[@]}"} -v "$PWD:/src:ro" -w /src \
        -e RIFT_TEST_BIN=/src/target/linux-docker/release/rift-terminal \
        -e RIFT_PHONE_BIN=/src/target/linux-docker/release/examples/phone \
        -e RIFT_RELAY_BIN=/src/target/linux-docker/release/rift-relay \
        -e LIBGL_ALWAYS_SOFTWARE=1 -e WGPU_BACKEND=vulkan -e RIFT_LEAK_SHOWN=1 \
        -e RIFT_UI_SHARDS="${RIFT_UI_SHARDS:-2}" \
        rift-ci-ubuntu bash -c 'scripts/gates.sh shell && xvfb-run -a scripts/gates.sh ui leak'
}

# One step of a gate, timed: `-- <name> pass 312s (14:02:11–14:07:23)`.
phase() {
    local name=$1 t0=$SECONDS at rc
    at=$(date +%H:%M:%S)
    shift
    echo "-- $name (from $at)"
    "$@"
    rc=$?
    echo "-- $name $( ((rc == 0)) && echo pass || echo FAIL) $((SECONDS - t0))s ($at–$(date +%H:%M:%S))"
    return "$rc"
}

# Wall-clock limit of each gate in seconds, cold builds included, with
# room over a normal run.
limit_for() {
    local s
    case $1 in
        fmt | shell | docs) s=300 ;;
        test) s=900 ;;
        clippy | doc | mobile | leak) s=1200 ;;
        linux) s=5400 ;;
        *) s=1800 ;;
    esac
    echo "${RIFT_GATE_TIMEOUT:-$s}"
}

# A process and everything under it (only this run's own children).
descendants() {
    local c
    for c in $(pgrep -P "$1" 2>/dev/null); do
        echo "$c"
        descendants "$c"
    done
}
stop_tree() { # pid
    local p
    for p in $(descendants "$1") "$1"; do kill -TERM "$p" 2>/dev/null; done
    sleep 3
    for p in $(descendants "$1") "$1"; do kill -KILL "$p" 2>/dev/null; done
}

# Run a command with a wall-clock limit. Past it, the processes still
# running under it are listed (that names the hung test binary) and stopped,
# and the command fails with 124.
limited() { # seconds, command...
    local secs=$1 t0=$SECONDS
    shift
    "$@" &
    cur_pid=$!
    while kill -0 "$cur_pid" 2>/dev/null; do
        if ((SECONDS - t0 >= secs)); then
            {
                echo "TIMEOUT after ${secs}s; still running:"
                ps -o pid=,etime=,command= -p "$( (descendants "$cur_pid"; echo "$cur_pid") | paste -sd, -)" |
                    cut -c1-200 | sed 's/^/  /'
            } >&2 2>/dev/null
            stop_tree "$cur_pid"
            wait "$cur_pid" 2>/dev/null
            cur_pid=""
            return 124
        fi
        sleep 1
    done
    wait "$cur_pid"
    local rc=$?
    cur_pid=""
    return "$rc"
}
cur_pid=""

case "${1:-}" in
    --list | -l) describe; exit 0 ;;
    --help | -h) sed -n '3,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac

# `--only NAME[,NAME]` is the ui gate's: scripts/ui-smoke.sh reads RIFT_UI_ONLY
args=()
while [[ $# -gt 0 ]]; do
    case $1 in
        --only) [[ $# -ge 2 ]] || { echo "--only needs a section name" >&2; exit 2; }
            export RIFT_UI_ONLY=$2; shift 2 ;;
        --only=*) export RIFT_UI_ONLY=${1#--only=}; shift ;;
        *) args+=("$1"); shift ;;
    esac
done
set -- ${args[@]+"${args[@]}"}
if [[ -n ${RIFT_UI_ONLY:-} && " $* " != *" ui "* ]]; then
    echo "--only selects ui-smoke.py sections: name the ui gate (scripts/gates.sh ui --only NAME)" >&2
    exit 2
fi

if [[ $# -eq 0 ]]; then
    gates=("${DEFAULT[@]}")
elif [[ $# -eq 1 && $1 == all ]]; then
    gates=("${ALL[@]}")
else
    gates=("$@")
fi

for g in "${gates[@]}"; do
    declare -F "gate_$g" >/dev/null || { echo "unknown gate: $g (scripts/gates.sh --list)" >&2; exit 2; }
done

# Each gate's output also goes to target/gates/<gate>.log; the summary
# names each failure with the end of its log, so the fix starts there.
LOGS="${CARGO_TARGET_DIR:-target}/gates"
# a read-only checkout (the linux gate's UI container): a temporary folder
if ! mkdir -p "$LOGS" 2>/dev/null || [[ ! -w $LOGS ]]; then
    LOGS=$(mktemp -d "${TMPDIR:-/tmp}/rift-gates.XXXXXX")
fi
run_started=$SECONDS
results=()
failures=()
failed=0
record() { # gate, exit status, seconds, started at
    local status=pass
    if [[ $2 -ne 0 ]]; then
        status=FAIL
        failed=1
        failures+=("$1")
    fi
    results+=("$(printf '%-8s %-4s %5ss  (%s–%s)' "$1" "$status" "$3" "$4" "$(date +%H:%M:%S)")")
}

# The linux gate (docker, its own target dir) is mostly waiting on its own
# VM: with other gates it runs beside them, started once the timing-
# sensitive unit tests are done, its output in target/gates/linux.log. It
# is reported the moment it ends (a failure with the end of its log), not
# only at the end. RIFT_GATES_SERIAL=1 runs it in turn instead.
bg_pid="" bg_start=0 bg_at=""
parallel_linux=0
if [[ -z ${RIFT_GATES_SERIAL:-} && ${#gates[@]} -gt 1 ]]; then
    for g in "${gates[@]}"; do [[ $g == linux ]] && parallel_linux=1; done
fi
start_linux() {
    bg_start=$SECONDS
    bg_at=$(date +%H:%M:%S)
    printf '\n\033[1m== linux\033[0m (from %s, in the background: %s)\n' "$bg_at" "$LOGS/linux.log"
    local n
    n=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)
    LINUX_CPUS=$((n > 4 ? n - 2 : n)) limited "$(limit_for linux)" gate_linux >"$LOGS/linux.log" 2>&1 &
    bg_pid=$!
}
# the background run ended: record it and say so now
reap_linux() { # block: wait for it
    [[ -n $bg_pid ]] || return 0
    if [[ ${1:-} != block ]] && kill -0 "$bg_pid" 2>/dev/null; then
        return 0
    fi
    wait "$bg_pid"
    local rc=$?
    bg_pid=""
    record linux "$rc" "$((SECONDS - bg_start))" "$bg_at"
    printf '\n\033[1m== linux\033[0m (background) %s after %ss; its steps:\n' \
        "$( ((rc == 0)) && echo pass || echo FAIL)" "$((SECONDS - bg_start))"
    grep -E '^-- .* (pass|FAIL) [0-9]+s' "$LOGS/linux.log" | sed 's/^/   /'
    ((rc == 0)) || { echo "   end of $LOGS/linux.log:"; tail -n 25 "$LOGS/linux.log" | sed 's/^/   | /'; }
}
# interrupted: stop the running gate and the background run with everything
# under them (the docker client stops its container: --rm, signals proxied)
stop_all() {
    [[ -n $cur_pid ]] && stop_tree "$cur_pid"
    [[ -n $bg_pid ]] && stop_tree "$bg_pid"
    exit 130
}
trap stop_all INT TERM
# started after `test` when it runs, else first
started_after=""
if [[ $parallel_linux -eq 1 ]]; then
    for g in "${gates[@]}"; do [[ $g == test ]] && started_after="test"; done
    [[ -z $started_after ]] && start_linux
fi

GATE_OUT=/dev/null
[[ -n ${CI:-} || -n ${RIFT_GATES_VERBOSE:-} ]] && GATE_OUT=/dev/stdout
run_gate() {
    "gate_$1" 2>&1 | tee "$LOGS/$1.log" >"$GATE_OUT"
    return "${PIPESTATUS[0]}"
}

for g in "${gates[@]}"; do
    [[ $parallel_linux -eq 1 && $g == linux ]] && continue
    at=$(date +%H:%M:%S)
    [[ $GATE_OUT == /dev/stdout ]] && printf '\n\033[1m== %s\033[0m (from %s)\n' "$g" "$at"
    start=$SECONDS
    limited "$(limit_for "$g")" run_gate "$g" 2>"$LOGS/$g.timeout"
    rc=$?
    # a timeout's report goes to the end of the gate's log
    cat "$LOGS/$g.timeout" >>"$LOGS/$g.log" 2>/dev/null
    rm -f "$LOGS/$g.timeout"
    printf -- '-- %s %s %ss%s\n' "$g" "$( ((rc == 0)) && echo pass || echo FAIL)" "$((SECONDS - start))" \
        "$( ((rc == 124)) && echo " (timeout)")"
    record "$g" "$rc" "$((SECONDS - start))" "$at"
    [[ $parallel_linux -eq 1 && $g == "$started_after" && -z $bg_pid ]] && start_linux
    reap_linux
done
if [[ -n $bg_pid ]]; then
    printf '\n\033[1m== linux\033[0m (waiting for the background run, %ss so far)\n' "$((SECONDS - bg_start))"
    reap_linux block
fi

total=$((SECONDS - run_started))
printf '\n\033[1m== summary (%s) %dm%02ds\033[0m\n' \
    "$(git rev-parse --short HEAD 2>/dev/null)$(git diff --quiet HEAD 2>/dev/null || echo '+dirty')" \
    $((total / 60)) $((total % 60))
printf '%s\n' "${results[@]}"
for g in ${failures[@]+"${failures[@]}"}; do
    printf '\n\033[1mFAIL %s\033[0m — the end of %s:\n' "$g" "$LOGS/$g.log"
    tail -n 25 "$LOGS/$g.log" | sed 's/^/  | /'
done
exit $failed
