#!/usr/bin/env bash
# Build a portable tarball: binary + .desktop + icon + docs.
#
# Usage: packaging/make-tarball.sh [--target <triple>] [--target-dir <dir>] [--no-build]
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
. packaging/lib.sh

TRIPLE=""
TARGET_DIR="$(shift_default_target_dir)"
BUILD=1

while [ $# -gt 0 ]; do
    case "$1" in
        --target)     TRIPLE="${2:?--target needs a triple}"; shift 2 ;;
        --target-dir) TARGET_DIR="${2:?--target-dir needs a path}"; shift 2 ;;
        --no-build)   BUILD=0; shift ;;
        -h|--help)    sed -n '2,5p' "$0"; exit 0 ;;
        *)            shift_die "unknown option: $1" ;;
    esac
done

VERSION="$(shift_version)"

# named for the build's target, not the host (see shift_target_arch)
OS="$(shift_target_os "$TRIPLE")"
ARCH="$(shift_target_arch "$TRIPLE")"

if [ "$BUILD" -eq 1 ]; then
    if [ -n "$TRIPLE" ]; then cargo build --release --target-dir "$TARGET_DIR" --target "$TRIPLE"; else cargo build --release --target-dir "$TARGET_DIR"; fi
fi

if [ -n "$TRIPLE" ]; then
    BIN="$TARGET_DIR/$TRIPLE/release/shift-terminal"
else
    BIN="$TARGET_DIR/release/shift-terminal"
fi
[ -x "$BIN" ] || shift_die "no shift-terminal binary at $BIN"

NAME="shift-terminal-$VERSION-$OS-$ARCH"
STAGE="$TARGET_DIR/tarball/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE"

install -m 0755 "$BIN" "$STAGE/shift-terminal"
install -m 0644 packaging/shift.desktop "$STAGE/shift.desktop"
install -m 0644 assets/icon/hicolor/256x256/apps/shift.png "$STAGE/shift.png"
cp -R docs "$STAGE/docs"
if [ -f README.md ]; then
    install -m 0644 README.md "$STAGE/README.md"
fi

TARBALL="$TARGET_DIR/$NAME.tar.gz"
rm -f "$TARBALL"
tar -czf "$TARBALL" -C "$TARGET_DIR/tarball" "$NAME"

echo "built $TARBALL ($(du -h "$TARBALL" | cut -f1))"
