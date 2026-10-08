#!/bin/bash
# Builds "Junction Studio.app" on this Mac, for this Mac's processor.
# A locally built app is not quarantined, so Gatekeeper does not block it.
# Usage: packaging/build-macos-app.sh [--install]   (--install copies it to /Applications)
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release
APP="target/Junction Studio.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/junction "$APP/Contents/MacOS/junction"
cp packaging/Info.plist "$APP/Contents/Info.plist"

ICONSET=$(mktemp -d)/icon.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/junction.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/junction.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/junction.icns"
codesign --force --deep --sign - "$APP"

if [[ "${1:-}" == "--install" ]]; then
  rm -rf "/Applications/Junction Studio.app"
  cp -R "$APP" /Applications/
  echo "Installed /Applications/Junction Studio.app"
else
  echo "Built $APP"
fi
