#!/usr/bin/env bash
# Cargo runner on macOS (.cargo/config.toml). `cargo run` launches
# shift-terminal from a dev bundle next to the binary
# (target/<profile>/Shift.app), so the Dock, ⌘Tab and Activity Monitor call it
# "Shift" instead of the executable name. Every other binary (tests,
# doctests) runs unchanged.
set -euo pipefail

bin="$1"
shift
if [ "$(basename "$bin")" != "shift-terminal" ]; then
    exec "$bin" "$@"
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
app="$(dirname "$bin")/Shift.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# a hard link: instant, and always the binary cargo just built
ln -f "$bin" "$app/Contents/MacOS/shift-terminal"
cp -f "$root/assets/icon/shift.icns" "$app/Contents/Resources/shift.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Shift</string>
    <key>CFBundleDisplayName</key><string>Shift</string>
    <key>CFBundleIdentifier</key><string>dev.shift.terminal.dev</string>
    <key>CFBundleVersion</key><string>${version}</string>
    <key>CFBundleShortVersionString</key><string>${version}</string>
    <key>CFBundleExecutable</key><string>shift-terminal</string>
    <key>CFBundleIconFile</key><string>shift</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
exec "$app/Contents/MacOS/shift-terminal" "$@"
