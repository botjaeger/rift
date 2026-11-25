#!/usr/bin/env bash
# Build the Windows beta download: a zip holding Rift.exe (one file: fonts,
# themes and shell hooks are embedded, the icon and version are its
# resources) and the README. Cross-built with mingw-w64 from macOS or Linux;
# not signed (Windows SmartScreen asks once: More info, Run anyway).
#
# Usage: packaging/make-windows-zip.sh [--target <triple>] [--target-dir <dir>] [--no-build]
#   --target defaults to x86_64-pc-windows-gnu (Windows on ARM runs it too)
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
. packaging/lib.sh

TRIPLE="x86_64-pc-windows-gnu"
TARGET_DIR="$(rift_default_target_dir)"
BUILD=1

while [ $# -gt 0 ]; do
    case "$1" in
        --target)     TRIPLE="${2:?--target needs a triple}"; shift 2 ;;
        --target-dir) TARGET_DIR="${2:?--target-dir needs a path}"; shift 2 ;;
        --no-build)   BUILD=0; shift ;;
        -h|--help)    rift_usage; exit 0 ;;
        *)            rift_die "unknown option: $1" ;;
    esac
done
case "$TRIPLE" in
    *-windows-*) ;;
    *) rift_die "not a Windows target: $TRIPLE" ;;
esac
command -v zip >/dev/null || rift_die "zip is not installed"

VERSION="$(rift_version)"
ARCH="${TRIPLE%%-*}"

if [ "$BUILD" -eq 1 ]; then
    command -v "$ARCH-w64-mingw32-gcc" >/dev/null ||
        rift_die "mingw-w64 is not installed (brew install mingw-w64)"
    rustup target add "$TRIPLE" >/dev/null
    cargo build --release --locked --target-dir "$TARGET_DIR" --target "$TRIPLE" -p rift-terminal
fi

BIN="$TARGET_DIR/$TRIPLE/release/rift-terminal.exe"
[ -f "$BIN" ] || rift_die "no rift-terminal.exe at $BIN"

NAME="Rift-$VERSION-windows-$ARCH"
STAGE="$TARGET_DIR/windows-zip/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "$BIN" "$STAGE/Rift.exe"
cp README.md "$STAGE/README.md"

ZIP="$(cd "$TARGET_DIR" && pwd)/$NAME.zip"
rm -f "$ZIP"
(cd "$TARGET_DIR/windows-zip" && zip -q -r -X "$ZIP" "$NAME")

echo "built $ZIP ($(du -h "$ZIP" | cut -f1))"
