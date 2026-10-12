#!/usr/bin/env bash
# Run only isolated scripted instances; never send OS-wide input.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -z "${RIFT_TEST_BIN:-}" ]; then
    cargo build -p rift-terminal --locked
    export RIFT_TEST_BIN="${CARGO_TARGET_DIR:-target}/debug/rift-terminal"
fi
python3 scripts/ui-smoke.py "$RIFT_TEST_BIN"
