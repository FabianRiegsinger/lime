#!/bin/sh
# Builds build/Lime.app: menu-bar app + lime CLI + bundled LGPL ffmpeg, ad-hoc signed.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
[ -x build/ffmpeg/bin/ffmpeg ] || scripts/build-ffmpeg.sh
cargo build --release -p lime -p lime-mac

APP=build/Lime.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/lime-mac target/release/lime "$APP/Contents/MacOS/"
cp build/ffmpeg/bin/ffmpeg "$APP/Contents/Resources/ffmpeg"
strip -x "$APP/Contents/Resources/ffmpeg"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Lime</string>
  <key>CFBundleDisplayName</key><string>Lime</string>
  <key>CFBundleIdentifier</key><string>com.fabianriegsinger.lime</string>
  <key>CFBundleExecutable</key><string>lime-mac</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$APP/Contents/Resources/ffmpeg" "$APP/Contents/MacOS/lime"
codesign --force --sign - "$APP"
du -sh "$APP"
