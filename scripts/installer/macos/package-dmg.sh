#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.0.0}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DIST="$ROOT/dist/macos"
STAGE="$DIST/stage"
DMG="$DIST/CodexPlusPlus-${VERSION}-macos-universal.dmg"

rm -rf "$DIST"
mkdir -p "$STAGE"

create_app() {
  local app_name="$1"
  local executable_name="$2"
  local binary_path="$3"
  local bundle_id="$4"
  local app_dir="$STAGE/$app_name.app"

  mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
  cp "$binary_path" "$app_dir/Contents/MacOS/$executable_name"
  chmod +x "$app_dir/Contents/MacOS/$executable_name"
  cat > "$app_dir/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>$app_name</string>
  <key>CFBundleDisplayName</key>
  <string>$app_name</string>
  <key>CFBundleIdentifier</key>
  <string>$bundle_id</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleExecutable</key>
  <string>$executable_name</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
</dict>
</plist>
PLIST
}

create_app "Codex++" "CodexPlusPlus" "$ROOT/target/release/codex-plus-plus" "com.bigpizzav3.codexplusplus"
create_app "Codex++ 管理工具" "CodexPlusPlusManager" "$ROOT/target/release/codex-plus-plus-manager" "com.bigpizzav3.codexplusplus.manager"

hdiutil create -volname "Codex++" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
echo "$DMG"
