#!/usr/bin/env bash
# Build a portable tarball: binary + .desktop + icon + docs.
#
# Usage: packaging/make-tarball.sh [--target <triple>] [--target-dir <dir>] [--no-build]
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
. packaging/lib.sh

TRIPLE=""
TARGET_DIR="$(runway_default_target_dir)"
BUILD=1

while [ $# -gt 0 ]; do
    case "$1" in
        --target)     TRIPLE="${2:?--target needs a triple}"; shift 2 ;;
        --target-dir) TARGET_DIR="${2:?--target-dir needs a path}"; shift 2 ;;
        --no-build)   BUILD=0; shift ;;
        -h|--help)    sed -n '2,5p' "$0"; exit 0 ;;
        *)            runway_die "unknown option: $1" ;;
    esac
done

VERSION="$(runway_version)"

case "$(uname -s)" in
    Darwin) OS=macos ;;
    Linux)  OS=linux ;;
    *)      OS="$(uname -s | tr '[:upper:]' '[:lower:]')" ;;
esac
ARCH="$(uname -m)"

if [ "$BUILD" -eq 1 ]; then
    if [ -n "$TRIPLE" ]; then cargo build --release --target "$TRIPLE"; else cargo build --release; fi
fi

if [ -n "$TRIPLE" ]; then
    BIN="$TARGET_DIR/$TRIPLE/release/runway"
else
    BIN="$TARGET_DIR/release/runway"
fi
[ -x "$BIN" ] || runway_die "no runway binary at $BIN"

NAME="runway-$VERSION-$OS-$ARCH"
STAGE="$TARGET_DIR/tarball/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE"

install -m 0755 "$BIN" "$STAGE/runway"
install -m 0644 packaging/runway.desktop "$STAGE/runway.desktop"
install -m 0644 assets/icon/hicolor/256x256/apps/runway.png "$STAGE/runway.png"
cp -R docs "$STAGE/docs"
if [ -f README.md ]; then
    install -m 0644 README.md "$STAGE/README.md"
fi

TARBALL="$TARGET_DIR/$NAME.tar.gz"
rm -f "$TARBALL"
tar -czf "$TARBALL" -C "$TARGET_DIR/tarball" "$NAME"

echo "built $TARBALL ($(du -h "$TARBALL" | cut -f1))"
