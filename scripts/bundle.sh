#!/bin/sh
# Builds build/Lime.app: menu-bar app + lime CLI + bundled LGPL ffmpeg + license notices.
# Signs ad hoc by default. For distribution set SIGN_IDENTITY="Developer ID Application: …"
# (hardened runtime), and NOTARY_PROFILE=<notarytool keychain profile> to notarize and staple.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
[ -x build/ffmpeg/bin/ffmpeg ] || scripts/build-ffmpeg.sh
# Keep the build machine's home folder out of the paths Rust embeds in panic messages.
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=~" cargo build --release -p lime -p lime-mac

APP=build/Lime.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/lime-mac target/release/lime "$APP/Contents/MacOS/"
cp build/ffmpeg/bin/ffmpeg "$APP/Contents/Resources/ffmpeg"
strip -x "$APP/Contents/Resources/ffmpeg"
python3 -I scripts/notices.py
cp build/Acknowledgements.txt "$APP/Contents/Resources/"
ICONSET=build/Lime.iconset
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Lime.icns"
rm -rf "$ICONSET"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Lime</string>
  <key>CFBundleDisplayName</key><string>Lime</string>
  <key>CFBundleIdentifier</key><string>com.fabianriegsinger.lime</string>
  <key>CFBundleIconFile</key><string>Lime</string>
  <key>CFBundleExecutable</key><string>lime-mac</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSAppleEventsUsageDescription</key><string>Lime reads the files selected in Finder when you press Option-Return.</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
sign() {
  if [ -n "${SIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$@"
  else
    codesign --force --sign - "$@"
  fi
}
sign "$APP/Contents/Resources/ffmpeg" "$APP/Contents/MacOS/lime"
sign --entitlements scripts/Lime.entitlements "$APP"
if [ -n "${NOTARY_PROFILE:-}" ]; then
  ditto -c -k --keepParent "$APP" build/Lime.zip
  xcrun notarytool submit build/Lime.zip --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  ditto -c -k --keepParent "$APP" build/Lime.zip # again, now with the ticket stapled
fi
du -sh "$APP"
