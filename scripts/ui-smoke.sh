#!/usr/bin/env bash
# Run only isolated scripted instances; never send OS-wide input.
set -euo pipefail
cd "$(dirname "$0")/.."
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
python3 scripts/ui-smoke.py "$RIFT_TEST_BIN"
