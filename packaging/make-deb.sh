#!/usr/bin/env bash
# Build a Debian package with plain dpkg-deb (no cargo-deb). Linux only.
#
# Usage: packaging/make-deb.sh [--target <triple>] [--target-dir <dir>] [--no-build]
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

command -v dpkg-deb >/dev/null || shift_die "dpkg-deb not found (apt-get install dpkg-dev)"

VERSION="$(shift_version)"
ARCH="$(dpkg --print-architecture)"

if [ "$BUILD" -eq 1 ]; then
    if [ -n "$TRIPLE" ]; then cargo build --release --target "$TRIPLE"; else cargo build --release; fi
fi

if [ -n "$TRIPLE" ]; then
    BIN="$TARGET_DIR/$TRIPLE/release/shift-terminal"
else
    BIN="$TARGET_DIR/release/shift-terminal"
fi
[ -x "$BIN" ] || shift_die "no shift-terminal binary at $BIN"

# Runtime dependencies, derived from the release binary itself. Note that
# `ldd` alone is NOT enough here: winit and wgpu dlopen() their windowing and
# GPU libraries by SONAME, so almost nothing shows up as a load-time link.
#
#   readelf -d target/release/shift-terminal | grep NEEDED
#     libz.so.1  libgcc_s.so.1  libm.so.6  libc.so.6
#         -> zlib1g, libgcc-s1, libc6      (libm is part of libc6)
#
#   strings -a target/release/shift-terminal | grep -oE 'lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*'
#     libxkbcommon.so.0      -> libxkbcommon0       (keymap handling)
#     libxkbcommon-x11.so.0  -> libxkbcommon-x11-0  (X11 keymap bridge)
#     libwayland-client.so.0 -> libwayland-client0
#     libwayland-egl.so.1    -> libwayland-egl1
#     libxcb.so.1            -> libxcb1
#     libX11.so.6            -> libx11-6
#     libX11-xcb.so.1        -> libx11-xcb1
#     libXcursor.so.1        -> libxcursor1
#     libXi.so.6             -> libxi6
#     libEGL.so.1            -> libegl1     ) GPU backends, see Recommends
#     libvulkan.so.1         -> libvulkan1  )
#
# Deliberately NOT listed: libwayland-cursor0, libxrandr2, libxcb-render0,
# libxcb-shape0 and libxcb-xfixes0. They are build-time deps of the *-sys
# crates (hence their -dev packages in ci.yml) but the shipped binary never
# references them, and libxcb1/libx11-6 pull what they do need.
# git2 is built with default-features = false, so there is no libssl/libgit2
# dependency either.
DEPENDS="libc6, libgcc-s1, zlib1g, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-egl1, libxcb1, libx11-6, libx11-xcb1, libxcursor1, libxi6"

# wgpu picks a backend at runtime, so requiring both would refuse to install on
# a machine that has only one. Neither is needed to install; one is needed to
# actually render.
RECOMMENDS="libvulkan1 | libegl1"

PKGROOT="$TARGET_DIR/deb/shift-terminal_${VERSION}_${ARCH}"
rm -rf "$PKGROOT"
mkdir -p \
    "$PKGROOT/DEBIAN" \
    "$PKGROOT/usr/bin" \
    "$PKGROOT/usr/share/applications" \
    "$PKGROOT/usr/share/doc/shift-terminal"

install -m 0755 "$BIN" "$PKGROOT/usr/bin/shift-terminal"
if command -v strip >/dev/null; then strip "$PKGROOT/usr/bin/shift-terminal"; fi

install -m 0644 packaging/shift.desktop "$PKGROOT/usr/share/applications/shift.desktop"

for dir in assets/icon/hicolor/*/; do
    size="$(basename "$dir")"
    install -D -m 0644 "$dir/apps/shift.png" \
        "$PKGROOT/usr/share/icons/hicolor/$size/apps/shift.png"
done

cat > "$PKGROOT/usr/share/doc/shift-terminal/copyright" <<'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: shift-terminal
Source: https://github.com/botjaeger/runway

Files: *
Copyright: Shift contributors
License: MIT

License: MIT
 Permission is hereby granted, free of charge, to any person obtaining a copy
 of this software and associated documentation files (the "Software"), to deal
 in the Software without restriction, including without limitation the rights
 to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 copies of the Software, and to permit persons to whom the Software is
 furnished to do so, subject to the following conditions:
 .
 The above copyright notice and this permission notice shall be included in all
 copies or substantial portions of the Software.
 .
 THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 SOFTWARE.
COPYRIGHT

INSTALLED_SIZE="$(du -ks "$PKGROOT" | cut -f1)"

cat > "$PKGROOT/DEBIAN/control" <<CONTROL
Package: shift-terminal
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${ARCH}
Maintainer: Shift contributors <noreply@github.com>
Installed-Size: ${INSTALLED_SIZE}
Depends: ${DEPENDS}
Recommends: ${RECOMMENDS}
Homepage: https://github.com/botjaeger/runway
Description: GPU-accelerated terminal emulator with block-based output
 Shift is a terminal emulator built on wgpu and winit. Every command is a
 bounded block (via OSC 133 shell integration) with an exit-code coloured
 border, duration, collapse and one-click output copy.
 .
 It also provides tabs, split panes, a fuzzy command palette, an owned
 syntax-highlighted input editor with completions, Warp-compatible YAML
 workflows and a hot-reloaded TOML configuration.
CONTROL

DEB="$TARGET_DIR/shift-terminal_${VERSION}_${ARCH}.deb"
rm -f "$DEB"
dpkg-deb --root-owner-group --build "$PKGROOT" "$DEB"

echo "built $DEB ($(du -h "$DEB" | cut -f1))"
