#!/usr/bin/env bash
# Builds dj-gui and wraps it in an app bundle, gui/target/release/decomp-dj.app, so Finder,
# the Dock and Launchpad show its icon (assets/icon_gui.icns). Run on macOS.
# Usage: gui/macos/bundle.sh [--no-build]
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
gui="$here/.."
root="$gui/.."
[ "${1:-}" = "--no-build" ] || (cd "$gui" && cargo build --release)

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$gui/Cargo.toml" | head -1)"
app="$gui/target/release/decomp-dj.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$gui/target/release/dj-gui" "$app/Contents/MacOS/dj-gui"
cp "$root/assets/icon_gui.icns" "$app/Contents/Resources/icon_gui.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key><string>decomp-dj</string>
	<key>CFBundleDisplayName</key><string>decomp-dj</string>
	<key>CFBundleIdentifier</key><string>io.github.and0p.decomp-dj</string>
	<key>CFBundleExecutable</key><string>dj-gui</string>
	<key>CFBundleIconFile</key><string>icon_gui</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>$version</string>
	<key>CFBundleVersion</key><string>$version</string>
	<key>LSMinimumSystemVersion</key><string>11.0</string>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Signed ad hoc, as the linker signs the bare binary: an unsigned bundle won't open on Apple
# silicon. (Not a Developer ID signature: a downloaded copy still needs the quarantine flag
# cleared, docs/install.md.)
codesign --force --sign - "$app" >/dev/null 2>&1 || echo "note: couldn't sign $app ad hoc"
echo "built $app"
