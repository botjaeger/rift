#!/usr/bin/env bash
# Build Rift.app (macOS).
#
# Usage: packaging/make-app.sh [--target <triple>] [--target-dir <dir>] [--no-build]
#   --target      pick target/<triple>/release/rift-terminal (CI cross builds)
#   --target-dir  override CARGO_TARGET_DIR / the default `target`
#   --no-build    package an already-built binary
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
        --release)    shift ;;  # accepted for compatibility; release is the only profile
        -h|--help)    rift_usage; exit 0 ;;
        *)            rift_die "unknown option: $1" ;;
    esac
done

VERSION="$(rift_version)"
[ -n "$VERSION" ] || rift_die "could not read [workspace.package] version from Cargo.toml"

if [ "$BUILD" -eq 1 ]; then
    if [ -n "$TRIPLE" ]; then
        cargo build --release --locked --target-dir "$TARGET_DIR" --target "$TRIPLE"
    else
        cargo build --release --locked --target-dir "$TARGET_DIR"
    fi
fi

if [ -n "$TRIPLE" ]; then
    BIN="$TARGET_DIR/$TRIPLE/release/rift-terminal"
else
    BIN="$TARGET_DIR/release/rift-terminal"
fi
[ -x "$BIN" ] || rift_die "no rift-terminal binary at $BIN (drop --no-build, or check --target/--target-dir)"

ICNS="assets/icon/rift.icns"
[ -f "$ICNS" ] || rift_die "missing $ICNS — run: python3 packaging/gen-icon.py"

APP="$TARGET_DIR/Rift.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/rift-terminal"
cp "$ICNS" "$APP/Contents/Resources/rift.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Rift</string>
    <key>CFBundleDisplayName</key><string>Rift</string>
    <key>CFBundleIdentifier</key><string>dev.rift.terminal</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleExecutable</key><string>rift-terminal</string>
    <key>CFBundleIconFile</key><string>rift</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature. Without any signature at all Gatekeeper refuses to launch
# the bundle even locally; "-" is the ad-hoc identity and needs no certificate.
codesign --force --deep --sign - "$APP"
codesign --verify --deep "$APP"

echo "built $APP (version $VERSION)"
