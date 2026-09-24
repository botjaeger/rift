#!/usr/bin/env bash
# shellcheck disable=SC2329  # gate_* functions are called as "gate_$g"
# Shift quality gates: one runner for local checks and CI.
#
#   scripts/gates.sh                 # default set: fmt clippy test doc shell docs
#   scripts/gates.sh all             # default set + release + linux
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
ALL=(fmt clippy test doc shell docs release linux)

describe() {
    cat <<'EOF'
fmt      cargo fmt --all --check
clippy   cargo clippy --workspace --all-targets --locked -- -D warnings
test     cargo test --workspace --locked
doc      cargo doc --workspace --no-deps --locked, rustdoc warnings denied
shell    shellcheck on packaging/*.sh, scripts/*.sh, .githooks/*, bash hooks; zsh -n and fish --no-execute on their hooks
docs     CLAUDE.md and AGENTS.md identical; every relative link in *.md resolves
release  cargo build --release --locked
linux    clippy + test + release build inside packaging/linux-verify.Dockerfile (docker)
EOF
}

have() { command -v "$1" >/dev/null 2>&1; }

gate_fmt() { cargo fmt --all --check; }
gate_clippy() { cargo clippy --workspace --all-targets --locked -- -D warnings; }
gate_test() { cargo test --workspace --locked; }
gate_doc() { RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked; }
gate_release() { cargo build --release --locked; }

gate_shell() {
    have shellcheck || { echo "shellcheck not installed (brew install shellcheck / apt install shellcheck)"; return 1; }
    shellcheck packaging/*.sh scripts/*.sh .githooks/* shell/shift.bash shell/shift-bashrc || return 1
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
    done < <(git ls-files --cached --others --exclude-standard '*.md')
    return "$rc"
}

gate_linux() {
    have docker || { echo "docker not installed"; return 1; }
    docker build -q -t shift-linux-verify -f packaging/linux-verify.Dockerfile packaging >/dev/null || return 1
    docker run --rm -v "$PWD:/src" -w /src \
        -v shift-cargo-registry:/usr/local/cargo/registry \
        -e CARGO_TARGET_DIR=/src/target/linux-docker \
        shift-linux-verify bash -c '
            cargo clippy --workspace --all-targets --locked -- -D warnings &&
            cargo test --workspace --locked &&
            cargo build --release --locked'
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
