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

# The Dock tile plugin: the chosen `app_icon` while Rift is not running
# (packaging/docktile/RiftDockTile.m). Both architectures, so a cross-built
# bundle carries a loadable plugin too.
PLUGIN="$APP/Contents/PlugIns/RiftDockTile.docktileplugin"
mkdir -p "$PLUGIN/Contents/MacOS" "$PLUGIN/Contents/Resources"
clang -bundle -fobjc-arc -mmacosx-version-min=11.0 -arch arm64 -arch x86_64 \
    -framework AppKit -o "$PLUGIN/Contents/MacOS/RiftDockTile" packaging/docktile/RiftDockTile.m
cp assets/icon/variants/*.png "$PLUGIN/Contents/Resources/"
cat > "$PLUGIN/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.rift.terminal.docktile</string>
    <key>CFBundleName</key><string>RiftDockTile</string>
    <key>CFBundleExecutable</key><string>RiftDockTile</string>
    <key>CFBundlePackageType</key><string>BNDL</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>NSPrincipalClass</key><string>RiftDockTilePlugIn</string>
</dict>
</plist>
PLIST

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
    <key>NSDockTilePlugIn</key><string>RiftDockTile.docktileplugin</string>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key><string>dev.rift.terminal.link</string>
            <key>CFBundleURLSchemes</key><array><string>rift</string></array>
        </dict>
    </array>
    <!-- folders dropped on the Dock icon, Finder's Open With, `open -a Rift <dir>`
         (crates/rift-terminal/src/links.rs); Alternate: never the folder default -->
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Folder</string>
            <key>CFBundleTypeRole</key><string>Shell</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key><array><string>public.folder</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST

# The background helpers as SMAppService agents (crates/rift-pty/src/launchd.rs):
# registered and started by the app on demand, so the Dock does not count them
# as Rift running. Never started at load: the app kickstarts them.
mkdir -p "$APP/Contents/Library/LaunchAgents"
agent() { # <name> <env key or ""> <args…>
    local name="$1" env="$2"
    shift 2
    {
        cat <<HEAD
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>dev.rift.${name}</string>
    <key>BundleProgram</key><string>Contents/MacOS/rift-terminal</string>
    <key>ProgramArguments</key>
    <array>
        <string>Contents/MacOS/rift-terminal</string>
HEAD
        for a in "$@"; do echo "        <string>${a}</string>"; done
        echo "    </array>"
        if [ -n "$env" ]; then
            echo "    <key>EnvironmentVariables</key><dict><key>${env}</key><string>1</string></dict>"
        fi
        cat <<TAIL
    <key>RunAtLoad</key><false/>
    <key>KeepAlive</key><false/>
    <key>AbandonProcessGroup</key><true/>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
TAIL
    } > "$APP/Contents/Library/LaunchAgents/dev.rift.${name}.plist"
}
agent session "" --session-daemon
# the gateway writes <data dir>/remote/gateway.log itself (no path fits here)
agent gateway RIFT_GATEWAY_LOG remote --managed
plutil -lint "$APP"/Contents/Library/LaunchAgents/*.plist >/dev/null

# Ad-hoc signature. Without any signature at all Gatekeeper refuses to launch
# the bundle even locally; "-" is the ad-hoc identity and needs no certificate.
codesign --force --deep --sign - "$APP"
codesign --verify --deep "$APP"

echo "built $APP (version $VERSION)"
