#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.0.0}"
ARCH="${2:-$(uname -m)}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DIST="$ROOT/dist/macos"
STAGE="${TMPDIR:-/tmp}/jiyi-codex-dmg-stage-${VERSION}-${ARCH}"
BINARY_DIR="${BINARY_DIR:-$ROOT/target/release}"
DMG="$DIST/JiyiCodex-${VERSION}-macos-${ARCH}.dmg"
ICON_SOURCE="$ROOT/apps/codex-plus-manager/src-tauri/icons/icon.png"
ICON_NAME="codex-plus-plus.icns"
ICON_ICNS="$DIST/$ICON_NAME"
CODEX_APP_SOURCE="${CODEX_APP_SOURCE:-/Applications/Codex.app}"
CODESIGN_IDENTITY="${JIYI_CODESIGN_IDENTITY:-${CODESIGN_IDENTITY:--}}"
NOTARIZE="${JIYI_NOTARIZE:-0}"

rm -rf "$DIST" "$STAGE"
mkdir -p "$DIST" "$STAGE"

is_truthy() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

using_developer_id() {
  [ "$CODESIGN_IDENTITY" != "-" ]
}

codesign_binary() {
  local target="$1"
  if using_developer_id; then
    codesign --force --timestamp --options runtime --sign "$CODESIGN_IDENTITY" "$target" >/dev/null
  else
    codesign --force --sign - "$target" >/dev/null
  fi
}

codesign_bundle() {
  local target="$1"
  if using_developer_id; then
    codesign --force --deep --timestamp --options runtime --sign "$CODESIGN_IDENTITY" "$target" >/dev/null
  else
    codesign --force --deep --sign - "$target" >/dev/null
  fi
}

sign_dmg_if_needed() {
  if using_developer_id; then
    codesign --force --timestamp --sign "$CODESIGN_IDENTITY" "$DMG" >/dev/null
  fi
}

notarize_dmg_if_requested() {
  if ! is_truthy "$NOTARIZE"; then
    return 0
  fi
  if ! using_developer_id; then
    echo "error: JIYI_NOTARIZE=1 requires JIYI_CODESIGN_IDENTITY" >&2
    return 1
  fi

  local args=(xcrun notarytool submit "$DMG" --wait)
  if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    args+=(--apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID")
  elif [ -n "${ASC_KEY_ID:-}" ] && [ -n "${ASC_ISSUER_ID:-}" ] && [ -n "${ASC_KEY_PATH:-}" ]; then
    args+=(--key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID")
  else
    echo "error: notarization requires APPLE_ID/APPLE_APP_SPECIFIC_PASSWORD/APPLE_TEAM_ID or ASC_KEY_ID/ASC_ISSUER_ID/ASC_KEY_PATH" >&2
    return 1
  fi

  "${args[@]}"
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl -a -vv -t install "$DMG"
}

plist_delete_if_present() {
  local plist="$1"
  local key="$2"
  /usr/libexec/PlistBuddy -c "Delete :$key" "$plist" >/dev/null 2>&1 || true
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

  if [ ! -x "$binary_path" ]; then
    echo "error: binary not found or not executable: $binary_path" >&2
    return 1
  fi

  rm -rf "$app_dir"
  mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
  cp "$binary_path" "$app_dir/Contents/MacOS/$executable_name"
  cp "$ICON_ICNS" "$app_dir/Contents/Resources/$ICON_NAME"
  chmod +x "$app_dir/Contents/MacOS/$executable_name"
  printf 'APPLJIYI' > "$app_dir/Contents/PkgInfo"
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
  <string>JIYI</string>
  <key>CFBundleExecutable</key>
  <string>$executable_name</string>
  <key>CFBundleIconFile</key>
  <string>$ICON_NAME</string>
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

embed_codex_client() {
  local app_dir="$1"
  local source="$CODEX_APP_SOURCE"
  local dest="$app_dir/Contents/Resources/JiyiCodexClient.app"

  if [ ! -d "$source/Contents" ]; then
    echo "error: full Codex client not found: $source" >&2
    echo "set CODEX_APP_SOURCE=/path/to/Codex.app to override" >&2
    return 1
  fi
  if [ ! -x "$source/Contents/MacOS/Codex" ]; then
    echo "error: Codex executable missing or not executable: $source/Contents/MacOS/Codex" >&2
    return 1
  fi

  rm -rf "$dest"
  echo "Embedding full Codex client: $source -> $dest"
  ditto "$source" "$dest"
  local plist="$dest/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.jiyi.codex.client' "$plist"
  /usr/libexec/PlistBuddy -c 'Set :CFBundleName JiyiCodexClient' "$plist"
  /usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName 极义codex' "$plist"
  /usr/libexec/PlistBuddy -c 'Set :CFBundleSignature JIYI' "$plist" >/dev/null 2>&1 \
    || /usr/libexec/PlistBuddy -c 'Add :CFBundleSignature string JIYI' "$plist"
  plist_delete_if_present "$plist" "CFBundleURLTypes"
  plist_delete_if_present "$plist" "SUPublicEDKey"
  plist_delete_if_present "$plist" "SUFeedURL"
  xattr -cr "$dest" 2>/dev/null || true
  codesign_bundle "$dest"
}

install_silent_launcher() {
  local app_dir="$1"
  local launcher="$BINARY_DIR/codex-plus-plus"
  local dest="$app_dir/Contents/MacOS/codex-plus-plus"

  if [ ! -x "$launcher" ]; then
    echo "error: silent launcher not found or not executable: $launcher" >&2
    return 1
  fi

  cp "$launcher" "$dest"
  chmod +x "$dest"
  codesign_binary "$dest"
}

install_managed_proxy() {
  local app_dir="$1"
  local proxy="$BINARY_DIR/jiyi-managed-proxy"
  local dest="$app_dir/Contents/MacOS/jiyi-managed-proxy"

  if [ ! -x "$proxy" ]; then
    echo "error: managed proxy binary not found or not executable: $proxy" >&2
    return 1
  fi

  cp "$proxy" "$dest"
  chmod +x "$dest"
  codesign_binary "$dest"
}

install_server_scripts() {
  local app_dir="$1"
  local source="$ROOT/scripts/server"
  local dest="$app_dir/Contents/Resources/server"
  local docker_dest="$dest/docker"

  if [ ! -d "$source" ]; then
    echo "error: server deployment scripts not found: $source" >&2
    return 1
  fi

  rm -rf "$dest"
  mkdir -p "$app_dir/Contents/Resources"
  ditto "$source" "$dest"
  mkdir -p "$docker_dest"
  cp "$ROOT/apps/jiyi-managed-proxy/Dockerfile" "$docker_dest/Dockerfile"
  chmod +x \
    "$dest/macos/install-managed-proxy-launchd.sh" \
    "$dest/macos/uninstall-managed-proxy-launchd.sh" \
    "$dest/linux/install-managed-proxy-systemd.sh" \
    "$dest/linux/uninstall-managed-proxy-systemd.sh"
}

sign_app() {
  local app_dir="$1"
  local executable
  executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app_dir/Contents/Info.plist")"
  xattr -cr "$app_dir" 2>/dev/null || true
  codesign_binary "$app_dir/Contents/MacOS/$executable"
  codesign_bundle "$app_dir"
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
  if [ ! -x "$app_dir/Contents/MacOS/codex-plus-plus" ]; then
    echo "error: missing silent launcher sidecar in $app_dir" >&2
    return 1
  fi
  if [ ! -x "$app_dir/Contents/MacOS/jiyi-managed-proxy" ]; then
    echo "error: missing managed proxy sidecar in $app_dir" >&2
    return 1
  fi
  if [ ! -x "$app_dir/Contents/Resources/server/macos/install-managed-proxy-launchd.sh" ]; then
    echo "error: missing managed proxy launchd installer in $app_dir" >&2
    return 1
  fi
  if [ ! -f "$app_dir/Contents/Resources/server/macos/jiyi-managed-proxy.env.example" ]; then
    echo "error: missing managed proxy env example in $app_dir" >&2
    return 1
  fi
  if [ ! -x "$app_dir/Contents/Resources/server/linux/install-managed-proxy-systemd.sh" ]; then
    echo "error: missing managed proxy systemd installer in $app_dir" >&2
    return 1
  fi
  if [ ! -f "$app_dir/Contents/Resources/server/docker/Dockerfile" ]; then
    echo "error: missing managed proxy Dockerfile in $app_dir" >&2
    return 1
  fi
  codesign -dv "$app_dir" >/dev/null 2>&1 || {
    echo "error: codesign verification failed for $app_dir" >&2
    return 1
  }
}

verify_embedded_codex_client() {
  local app_dir="$1"
  local client_dir="$app_dir/Contents/Resources/JiyiCodexClient.app"
  local plist="$client_dir/Contents/Info.plist"

  if [ ! -x "$client_dir/Contents/MacOS/Codex" ]; then
    echo "error: embedded Codex executable missing: $client_dir/Contents/MacOS/Codex" >&2
    return 1
  fi
  local bundle_id
  bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist")"
  if [ "$bundle_id" != "com.jiyi.codex.client" ]; then
    echo "error: embedded Codex bundle id is not isolated: $bundle_id" >&2
    return 1
  fi
  if /usr/libexec/PlistBuddy -c 'Print :CFBundleURLTypes' "$plist" >/dev/null 2>&1; then
    echo "error: embedded Codex client still declares URL schemes" >&2
    return 1
  fi
  if /usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$plist" >/dev/null 2>&1; then
    echo "error: embedded Codex client still carries Sparkle update identity" >&2
    return 1
  fi
  codesign --verify --deep --strict "$client_dir" >/dev/null 2>&1 || {
    echo "error: embedded Codex codesign verification failed for $client_dir" >&2
    return 1
  }
}

prepare_icon
create_app "极义codex" "JiyiCodex" "$BINARY_DIR/codex-plus-plus-manager" "com.jiyi.codex" "false"
create_app "极义codex 管理工具" "JiyiCodexManager" "$BINARY_DIR/codex-plus-plus-manager" "com.jiyi.codex.manager" "false"
install_silent_launcher "$STAGE/极义codex.app"
install_silent_launcher "$STAGE/极义codex 管理工具.app"
install_managed_proxy "$STAGE/极义codex.app"
install_managed_proxy "$STAGE/极义codex 管理工具.app"
install_server_scripts "$STAGE/极义codex.app"
install_server_scripts "$STAGE/极义codex 管理工具.app"
embed_codex_client "$STAGE/极义codex.app"
ln -s /Applications "$STAGE/Applications"

sign_app "$STAGE/极义codex.app"
sign_app "$STAGE/极义codex 管理工具.app"

verify_app "$STAGE/极义codex.app"
verify_embedded_codex_client "$STAGE/极义codex.app"
verify_app "$STAGE/极义codex 管理工具.app"

hdiutil create -fs HFS+ -volname "极义codex" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
sign_dmg_if_needed
notarize_dmg_if_requested
echo "$DMG"
