#!/usr/bin/env bash
# shellcheck disable=SC2317,SC2329  # gate_* functions are called as "gate_$g" (SC2317 on shellcheck <0.10)
# Shift quality gates: one runner for local checks and CI.
#
#   scripts/gates.sh                 # default set: fmt clippy test doc shell docs
#   scripts/gates.sh all             # default set + release + ui + linux
#   scripts/gates.sh fmt clippy      # only the named gates
#   scripts/gates.sh --list          # describe every gate
#
# Every requested gate runs even after a failure, so one pass shows every
# problem; the exit status is non-zero when any gate failed. A gate whose
# tool is missing FAILS (it did not run) rather than passing silently.
#
# The compiler comes from rust-toolchain.toml, so local runs, CI and the
# Linux docker image all use the same clippy and rustfmt.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

DEFAULT=(fmt clippy test doc shell docs)
ALL=(fmt clippy test doc shell docs release ui linux)

describe() {
    cat <<'EOF'
fmt      cargo fmt --all --check, and on each vendored crate
clippy   cargo clippy --workspace --all-targets --locked -- -D warnings, and on each vendored crate (all features)
test     cargo test --workspace --locked, and each vendored crate (all features)
doc      cargo doc --workspace --no-deps --locked, rustdoc warnings denied
shell    shellcheck on packaging/*.sh, scripts/*.sh, .githooks/*, bash hooks; zsh -n and fish --no-execute on their hooks
docs     CLAUDE.md and AGENTS.md identical; every relative link in *.md resolves (vendored crates' upstream docs excepted)
release  cargo build --release --locked
ui       isolated scripted UI regressions (display or xvfb-run required)
linux    clippy + test + release build in packaging/linux-verify.Dockerfile, then shell + scripted UI (xvfb) on the CI runner's packages in packaging/ci-ubuntu.Dockerfile (docker)
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
        vendored clippy --all-targets --all-features --locked -- -D warnings
}
gate_test() { cargo test --workspace --locked && vendored test --all-features --locked; }
gate_doc() { RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked; }
gate_release() { cargo build --release --locked; }
gate_ui() { scripts/ui-smoke.sh; }

gate_shell() {
    have shellcheck || { echo "shellcheck not installed (brew install shellcheck / apt install shellcheck)"; return 1; }
    shellcheck packaging/*.sh scripts/*.sh .githooks/* shell/shift.bash shell/shift-bashrc shell/shift-bashrc-child || return 1
    # zsh and fish hooks: syntax only (shellcheck has no zsh/fish mode).
    # A missing shell is a failure: the hook would ship unchecked.
    local rc=0
    if have zsh; then zsh -n shell/shift.zsh || rc=1; else echo "zsh not installed: shell/shift.zsh unchecked"; rc=1; fi
    if have fish; then fish --no-execute shell/shift.fish || rc=1; else echo "fish not installed: shell/shift.fish unchecked"; rc=1; fi
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

gate_linux() {
    have docker || { echo "docker not installed"; return 1; }
    docker build -q -t shift-linux-verify -f packaging/linux-verify.Dockerfile packaging >/dev/null || return 1
    docker run --rm --init -v "$PWD:/src" -w /src \
        -v shift-cargo-registry:/usr/local/cargo/registry \
        -e CARGO_TARGET_DIR=/src/target/linux-docker \
        shift-linux-verify bash -c '
            scripts/gates.sh clippy test &&
            cargo build --release --locked' || return 1
    # The shell and UI gates run on the CI runner's packages, with the
    # environment of ci.yml's UI step: a library, font or shellcheck version
    # the runner differs on fails here first.
    docker build -q -t shift-ci-ubuntu -f packaging/ci-ubuntu.Dockerfile packaging >/dev/null || return 1
    docker run --rm --init -v "$PWD:/src:ro" -w /src \
        -e SHIFT_TEST_BIN=/src/target/linux-docker/release/shift-terminal \
        -e LIBGL_ALWAYS_SOFTWARE=1 -e WGPU_BACKEND=vulkan \
        shift-ci-ubuntu bash -c 'scripts/gates.sh shell && xvfb-run -a scripts/gates.sh ui'
}

case "${1:-}" in
    --list | -l) describe; exit 0 ;;
    --help | -h) sed -n '3,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac

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

results=()
failed=0
for g in "${gates[@]}"; do
    printf '\n\033[1m== %s\033[0m\n' "$g"
    start=$SECONDS
    if "gate_$g"; then
        status=pass
    else
        status=FAIL
        failed=1
    fi
    results+=("$(printf '%-8s %-4s %4ss' "$g" "$status" "$((SECONDS - start))")")
done

printf '\n\033[1m== summary (%s)\033[0m\n' "$(git rev-parse --short HEAD 2>/dev/null)$(git diff --quiet HEAD 2>/dev/null || echo '+dirty')"
printf '%s\n' "${results[@]}"
exit $failed
