#!/usr/bin/env bash
# Shared helpers for packaging/*.sh. Source AFTER cd-ing to the repo root.
# shellcheck shell=bash

# The version lives in exactly one place: [workspace.package] version.
rift_version() {
    awk -F'"' '
        /^\[workspace\.package\]/                     { sec = 1; next }
        /^\[/                                         { sec = 0 }
        sec && /^[[:space:]]*version[[:space:]]*=/    { print $2; exit }
    ' Cargo.toml
}

# Cargo writes into CARGO_TARGET_DIR when it is set (the docker verification
# points it at target/linux-docker so it cannot clobber the macOS target dir),
# so every artifact path has to be derived from the same value.
rift_default_target_dir() {
    printf '%s' "${CARGO_TARGET_DIR:-target}"
}

# The OS and architecture a build is FOR, from its --target triple (the host
# when empty): CI cross-builds the Intel Mac app on an Apple-silicon runner, so
# `uname -m` would name both macOS packages arm64. Architectures follow the
# DMG names: arm64/x86_64 on macOS, aarch64/x86_64 on Linux.
rift_target_os() {
    case "${1:-}" in
        *-apple-darwin) echo macos ;;
        *-linux-*)      echo linux ;;
        "") case "$(uname -s)" in
                Darwin) echo macos ;;
                Linux)  echo linux ;;
                *)      uname -s | tr '[:upper:]' '[:lower:]' ;;
            esac ;;
        *) rift_die "unsupported target triple: $1" ;;
    esac
}

rift_target_arch() {
    local triple="${1:-}" os arch
    os="$(rift_target_os "$triple")"
    if [ -n "$triple" ]; then arch="${triple%%-*}"; else arch="$(uname -m)"; fi
    case "$os:$arch" in
        macos:aarch64 | macos:arm64) echo arm64 ;;
        linux:arm64)                 echo aarch64 ;;
        *)                           echo "$arch" ;;
    esac
}

# The calling script's header comment (everything after the shebang up to
# the first code line), without the comment markers: its --help text.
rift_usage() {
    awk 'NR == 1 { next } !/^#/ { exit } { sub(/^# ?/, ""); print }' "$0"
}

rift_die() {
    echo "error: $*" >&2
    exit 1
}
