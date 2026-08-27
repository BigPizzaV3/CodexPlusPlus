#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.0.0}"
ARCH="${2:-$(uname -m)}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DIST="$ROOT/dist/macos"
STAGE="$DIST/stage"
BINARY_DIR="${BINARY_DIR:-$ROOT/target/release}"
DMG="$DIST/CodexPlusPlus-${VERSION}-macos-${ARCH}.dmg"
ICON_SOURCE="$ROOT/apps/codex-plus-manager/src-tauri/icons/icon.png"
ICON_NAME="codex-plus-plus.icns"
ICON_ICNS="$DIST/$ICON_NAME"
BACKGROUND_SOURCE="$ROOT/assets/installer/macos/dmg-background.svg"
BACKGROUND_PATH="$STAGE/.background/background.png"

rm -rf "$DIST"
mkdir -p "$STAGE"

prepare_background() {
  mkdir -p "$(dirname "$BACKGROUND_PATH")"

  if sips -s format png "$BACKGROUND_SOURCE" --out "$BACKGROUND_PATH" >/dev/null 2>&1; then
    return 0
  fi

  # Older macOS versions do not let sips decode SVG, so use Quick Look as a fallback.
  local preview_dir="$DIST/background-preview"
  local preview_path="$preview_dir/$(basename "$BACKGROUND_SOURCE").png"
  mkdir -p "$preview_dir"
  qlmanage -t -s 1200 -o "$preview_dir" "$BACKGROUND_SOURCE" >/dev/null 2>&1
  if [ ! -f "$preview_path" ]; then
    echo "error: failed to render DMG background: $BACKGROUND_SOURCE" >&2
    return 1
  fi
  cp "$preview_path" "$BACKGROUND_PATH"
}

prepare_icon() {
  local iconset="$DIST/codex-plus-plus.iconset"
  rm -rf "$iconset"
  mkdir -p "$iconset"

  sips -z 16 16 "$ICON_SOURCE" --out "$iconset/icon_16x16.png" >/dev/null
  sips -z 32 32 "$ICON_SOURCE" --out "$iconset/icon_16x16@2x.png" >/dev/null
  sips -z 32 32 "$ICON_SOURCE" --out "$iconset/icon_32x32.png" >/dev/null
  sips -z 64 64 "$ICON_SOURCE" --out "$iconset/icon_32x32@2x.png" >/dev/null
  sips -z 128 128 "$ICON_SOURCE" --out "$iconset/icon_128x128.png" >/dev/null
  sips -z 256 256 "$ICON_SOURCE" --out "$iconset/icon_128x128@2x.png" >/dev/null
  sips -z 256 256 "$ICON_SOURCE" --out "$iconset/icon_256x256.png" >/dev/null
  sips -z 512 512 "$ICON_SOURCE" --out "$iconset/icon_256x256@2x.png" >/dev/null
  sips -z 512 512 "$ICON_SOURCE" --out "$iconset/icon_512x512.png" >/dev/null
  sips -z 1024 1024 "$ICON_SOURCE" --out "$iconset/icon_512x512@2x.png" >/dev/null

  iconutil -c icns "$iconset" -o "$ICON_ICNS"
}

create_app() {
  local app_name="$1"
  local executable_name="$2"
  local binary_path="$3"
  local bundle_id="$4"
  local lsui_element="${5:-false}"
  local app_dir="$STAGE/$app_name.app"
  local url_types=""

  if [ ! -x "$binary_path" ]; then
    echo "error: binary not found or not executable: $binary_path" >&2
    return 1
  fi

  rm -rf "$app_dir"
  mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
  cp "$binary_path" "$app_dir/Contents/MacOS/$executable_name"
  cp "$ICON_ICNS" "$app_dir/Contents/Resources/$ICON_NAME"
  chmod +x "$app_dir/Contents/MacOS/$executable_name"
  printf 'APPL????' > "$app_dir/Contents/PkgInfo"
  if [ "$executable_name" = "CodexPlusPlusManager" ]; then
    url_types='  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>
      <string>Codex++ Links</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>codexplusplus</string>
        <string>dreamskin</string>
      </array>
    </dict>
  </array>'
  fi
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
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleSignature</key>
  <string>????</string>
  <key>CFBundleExecutable</key>
  <string>$executable_name</string>
  <key>CFBundleIconFile</key>
  <string>$ICON_NAME</string>
$url_types
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>LSUIElement</key>
  <$lsui_element/>
</dict>
</plist>
PLIST
}

sign_app() {
  local app_dir="$1"
  local executable
  executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app_dir/Contents/Info.plist")"
  codesign --force --sign - "$app_dir/Contents/MacOS/$executable"
  codesign --force --sign - "$app_dir"
}

verify_app() {
  local app_dir="$1"
  local plist="$app_dir/Contents/Info.plist"
  local plutil_bin
  plutil_bin="$(command -v plutil || true)"
  if [ -n "$plutil_bin" ]; then
    "$plutil_bin" -lint "$plist" >/dev/null
  else
    /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist" >/dev/null
  fi
  if [ ! -f "$app_dir/Contents/PkgInfo" ]; then
    echo "error: missing PkgInfo in $app_dir" >&2
    return 1
  fi
  codesign -dv "$app_dir" >/dev/null 2>&1 || {
    echo "error: codesign verification failed for $app_dir" >&2
    return 1
  }
}

prepare_icon
prepare_background
create_app "Codex++" "CodexPlusPlus" "$BINARY_DIR/codex-plus-plus" "com.bigpizzav3.codexplusplus" "true"
create_app "Codex++ 管理工具" "CodexPlusPlusManager" "$BINARY_DIR/codex-plus-plus-manager" "com.bigpizzav3.codexplusplus.manager" "false"

sign_app "$STAGE/Codex++.app"
sign_app "$STAGE/Codex++ 管理工具.app"

verify_app "$STAGE/Codex++.app"
verify_app "$STAGE/Codex++ 管理工具.app"

ln -s /Applications "$STAGE/Applications"

DMG_WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/codex-plus-plus-dmg.XXXXXX")"
DMG_WORK_PATH="$DMG_WORK_DIR/$(basename "$DMG")"
DMG_CREATED=false
MOUNT_POINT=""

cleanup_dmg_work_dir() {
  if [ -n "$MOUNT_POINT" ]; then
    hdiutil detach "$MOUNT_POINT" >/dev/null 2>&1 || true
  fi
  rm -f "$DMG_WORK_PATH"
  rmdir "$DMG_WORK_DIR" 2>/dev/null || true
}

trap cleanup_dmg_work_dir EXIT

hdiutil create -volname "Codex++" -srcfolder "$STAGE" -ov -format UDRW "$DMG_WORK_PATH"

MOUNT_OUTPUT="$(hdiutil attach "$DMG_WORK_PATH" -readwrite -noverify -noautoopen -nobrowse)"
MOUNT_POINT="$(printf '%s\n' "$MOUNT_OUTPUT" | awk 'match($0, /\/Volumes\//) {print substr($0, RSTART)}' | tail -1)"
if [ -z "$MOUNT_POINT" ]; then
  echo "error: failed to find mounted DMG volume" >&2
  exit 1
fi
VOLUME_NAME="$(basename "$MOUNT_POINT")"

if ! MOUNT_POINT="$MOUNT_POINT" VOLUME_NAME="$VOLUME_NAME" osascript <<'APPLESCRIPT'
with timeout of 30 seconds
  tell application "Finder"
    set dmgDisk to disk (system attribute "VOLUME_NAME")
    open dmgDisk
    delay 1
    set dmgWindow to container window of dmgDisk
    set current view of dmgWindow to icon view
    set toolbar visible of dmgWindow to false
    set statusbar visible of dmgWindow to false
    set bounds of dmgWindow to {100, 100, 1300, 850}

    set viewOptions to icon view options of dmgWindow
    set arrangement of viewOptions to not arranged
    set icon size of viewOptions to 96
    set text size of viewOptions to 14
    set color of viewOptions to {65535, 65535, 65535}
    set backgroundFile to (POSIX file ((system attribute "MOUNT_POINT") & "/.background/background.png")) as alias
    set background picture of viewOptions to backgroundFile

    tell dmgDisk
      set position of item "Applications" to {1000, 390}
      set position of item "Codex++.app" to {220, 390}
      set position of item "Codex++ 管理工具.app" to {460, 390}
    end tell

    close dmgWindow
    delay 1
  end tell
end timeout
APPLESCRIPT
then
  echo "warning: unable to persist Finder DMG window layout; the background is still included" >&2
fi

if ! hdiutil detach "$MOUNT_POINT" >/dev/null; then
  sleep 1
  hdiutil detach "$MOUNT_POINT" -force >/dev/null
fi
MOUNT_POINT=""

for attempt in 1 2 3; do
  if hdiutil convert "$DMG_WORK_PATH" -format UDZO -ov -o "$DMG"; then
    DMG_CREATED=true
    break
  fi

  if [ "$attempt" -lt 3 ]; then
    sleep "$((attempt * 2))"
  fi
done

if [ "$DMG_CREATED" != true ]; then
  echo "error: failed to create DMG after 3 attempts" >&2
  exit 1
fi

echo "$DMG"
