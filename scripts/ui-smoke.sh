#!/usr/bin/env bash
# Run only isolated scripted instances; never send OS-wide input.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -z "${SHIFT_TEST_BIN:-}" ]; then
    cargo build -p shift-app --locked
    export SHIFT_TEST_BIN="${CARGO_TARGET_DIR:-target}/debug/shift-terminal"
fi
python3 scripts/ui-smoke.py "$SHIFT_TEST_BIN"
