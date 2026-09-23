#!/usr/bin/env bash
# Shared helpers for packaging/*.sh. Source AFTER cd-ing to the repo root.
# shellcheck shell=bash

# The version lives in exactly one place: [workspace.package] version.
shift_version() {
    awk -F'"' '
        /^\[workspace\.package\]/                     { sec = 1; next }
        /^\[/                                         { sec = 0 }
        sec && /^[[:space:]]*version[[:space:]]*=/    { print $2; exit }
    ' Cargo.toml
}

# Cargo writes into CARGO_TARGET_DIR when it is set (the docker verification
# points it at target/linux-docker so it cannot clobber the macOS target dir),
# so every artifact path has to be derived from the same value.
shift_default_target_dir() {
    printf '%s' "${CARGO_TARGET_DIR:-target}"
}

shift_die() {
    echo "error: $*" >&2
    exit 1
}
