#!/usr/bin/env bash
# Build a portable AppImage. Linux only.
#
# Usage: packaging/make-appimage.sh [--target <triple>] [--target-dir <dir>] [--no-build]
# appimagetool is downloaded into <target-dir>/ on first run.
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
        -h|--help)    sed -n '2,6p' "$0"; exit 0 ;;
        *)            runway_die "unknown option: $1" ;;
    esac
done

VERSION="$(runway_version)"
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

# --- AppDir ---------------------------------------------------------------
APPDIR="$TARGET_DIR/Runway.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications"

install -m 0755 "$BIN" "$APPDIR/usr/bin/runway"
command -v strip >/dev/null && strip "$APPDIR/usr/bin/runway" || true

install -m 0644 packaging/runway.desktop "$APPDIR/usr/share/applications/runway.desktop"

for dir in assets/icon/hicolor/*/; do
    size="$(basename "$dir")"
    install -D -m 0644 "$dir/apps/runway.png" \
        "$APPDIR/usr/share/icons/hicolor/$size/apps/runway.png"
done

# appimagetool reads the .desktop and the icon from the AppDir ROOT, and runs
# AppRun as the entry point.
cp packaging/runway.desktop "$APPDIR/runway.desktop"
cp assets/icon/hicolor/256x256/apps/runway.png "$APPDIR/runway.png"
ln -sf usr/bin/runway "$APPDIR/AppRun"

# --- appimagetool ---------------------------------------------------------
# Cached in its own subdirectory, never next to the artifacts: the release
# workflow globs <target>/*.AppImage and would otherwise publish the 14 MB
# appimagetool as a release asset.
TOOL="$TARGET_DIR/appimagetool/appimagetool-$ARCH.AppImage"
if [ ! -x "$TOOL" ]; then
    mkdir -p "$TARGET_DIR/appimagetool"
    URLS="
https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage
https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-$ARCH.AppImage
"
    ok=0
    for url in $URLS; do
        echo "downloading $url"
        if curl -fsSL --retry 3 -o "$TOOL.part" "$url"; then
            mv "$TOOL.part" "$TOOL"
            ok=1
            break
        fi
    done
    rm -f "$TOOL.part"
    [ "$ok" -eq 1 ] || runway_die "could not download appimagetool for $ARCH (no network?)"
    chmod +x "$TOOL"
fi

OUT="$TARGET_DIR/Runway-$VERSION-$ARCH.AppImage"
rm -f "$OUT"

# --appimage-extract-and-run: docker and GitHub runners have no FUSE, so the
# tool cannot mount its own squashfs and must unpack itself instead.
ARCH="$ARCH" "$TOOL" --appimage-extract-and-run "$APPDIR" "$OUT"
chmod +x "$OUT"

echo "built $OUT ($(du -h "$OUT" | cut -f1))"
