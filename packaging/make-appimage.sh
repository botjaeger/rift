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

VERSION="$(rift_version)"
ARCH="$(uname -m)"

if [ "$BUILD" -eq 1 ]; then
    if [ -n "$TRIPLE" ]; then cargo build --release --locked --target-dir "$TARGET_DIR" --target "$TRIPLE"; else cargo build --release --locked --target-dir "$TARGET_DIR"; fi
fi

if [ -n "$TRIPLE" ]; then
    BIN="$TARGET_DIR/$TRIPLE/release/rift-terminal"
else
    BIN="$TARGET_DIR/release/rift-terminal"
fi
[ -x "$BIN" ] || rift_die "no rift-terminal binary at $BIN"

# --- AppDir ---------------------------------------------------------------
APPDIR="$TARGET_DIR/Rift.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications"

install -m 0755 "$BIN" "$APPDIR/usr/bin/rift-terminal"
if command -v strip >/dev/null; then strip "$APPDIR/usr/bin/rift-terminal"; fi

install -m 0644 packaging/rift.desktop "$APPDIR/usr/share/applications/rift.desktop"

for dir in assets/icon/hicolor/*/; do
    size="$(basename "$dir")"
    install -D -m 0644 "$dir/apps/rift.png" \
        "$APPDIR/usr/share/icons/hicolor/$size/apps/rift.png"
done

# appimagetool reads the .desktop and the icon from the AppDir ROOT, and runs
# AppRun as the entry point.
cp packaging/rift.desktop "$APPDIR/rift.desktop"
cp assets/icon/hicolor/256x256/apps/rift.png "$APPDIR/rift.png"
ln -sf usr/bin/rift-terminal "$APPDIR/AppRun"

# --- appimagetool ---------------------------------------------------------
# Cached in its own subdirectory, never next to the artifacts: the release
# workflow globs <target>/*.AppImage and would otherwise publish the 14 MB
# appimagetool as a release asset.
# Digests from https://api.github.com/repos/AppImage/appimagetool/releases/tags/1.9.1
# Update version and digests together; never execute an unchecked cached tool.
TOOL_VERSION=1.9.1
case "$ARCH" in
    x86_64) TOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 ;;
    aarch64) TOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158 ;;
    armhf) TOOL_SHA256=42b61cba5495d8aaf418a5c9a015a49b85ad92efabcbd3c341f1540440e4e23d ;;
    i686) TOOL_SHA256=7ad9ff47c203aae0149b18f6df9e3018b2e2f470ea644a0413e3ded39e9e3bdb ;;
    *) rift_die "unsupported appimagetool architecture: $ARCH" ;;
esac
TOOL="$TARGET_DIR/appimagetool/$TOOL_VERSION/appimagetool-$ARCH.AppImage"
if [ ! -f "$TOOL" ]; then
    mkdir -p "$(dirname "$TOOL")"
    url="https://github.com/AppImage/appimagetool/releases/download/$TOOL_VERSION/appimagetool-$ARCH.AppImage"
    trap 'rm -f "$TOOL.part"' EXIT
    curl -fsSL --retry 3 -o "$TOOL.part" "$url"
    printf '%s  %s\n' "$TOOL_SHA256" "$TOOL.part" | sha256sum -c - || rift_die "appimagetool checksum mismatch"
    mv "$TOOL.part" "$TOOL"
fi
printf '%s  %s\n' "$TOOL_SHA256" "$TOOL" | sha256sum -c - || rift_die "cached appimagetool checksum mismatch"
chmod +x "$TOOL"

# named for the build's target (the manifest's asset names); ARCH above is
# the host's, for appimagetool
OUT="$TARGET_DIR/Rift-$VERSION-$(rift_target_arch "$TRIPLE").AppImage"
rm -f "$OUT"

# --appimage-extract-and-run: docker and GitHub runners have no FUSE, so the
# tool cannot mount its own squashfs and must unpack itself instead.
ARCH="$ARCH" "$TOOL" --appimage-extract-and-run "$APPDIR" "$OUT"
chmod +x "$OUT"

echo "built $OUT ($(du -h "$OUT" | cut -f1))"
