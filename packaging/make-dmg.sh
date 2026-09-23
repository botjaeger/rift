#!/usr/bin/env bash
# Build a distributable DMG from an already-built Shift.app (macOS).
#
# Usage: packaging/make-dmg.sh [--target-dir <dir>] [--arch <arch>]
# Run packaging/make-app.sh first.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
. packaging/lib.sh

TARGET_DIR="$(shift_default_target_dir)"
ARCH=""

while [ $# -gt 0 ]; do
    case "$1" in
        --target-dir) TARGET_DIR="${2:?--target-dir needs a path}"; shift 2 ;;
        --arch)       ARCH="${2:?--arch needs a value}"; shift 2 ;;
        -h|--help)    sed -n '2,6p' "$0"; exit 0 ;;
        *)            shift_die "unknown option: $1" ;;
    esac
done

VERSION="$(shift_version)"
APP="$TARGET_DIR/Shift.app"
[ -d "$APP" ] || shift_die "no $APP — run packaging/make-app.sh first"

# Name the DMG after the architecture the BINARY was built for, not the host:
# CI cross-builds x86_64 on an arm64 runner and vice versa.
if [ -z "$ARCH" ]; then
    ARCH="$(lipo -archs "$APP/Contents/MacOS/shift-terminal" 2>/dev/null | tr ' ' '-')"
    [ -n "$ARCH" ] || ARCH="$(uname -m)"
fi

DMG="$TARGET_DIR/Shift-$VERSION-$ARCH.dmg"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp -R "$APP" "$STAGE/Shift.app"
ln -s /Applications "$STAGE/Applications"

rm -f "$DMG"
hdiutil create \
    -volname Shift \
    -srcfolder "$STAGE" \
    -ov -format UDZO \
    "$DMG"

echo "built $DMG ($(du -h "$DMG" | cut -f1))"
