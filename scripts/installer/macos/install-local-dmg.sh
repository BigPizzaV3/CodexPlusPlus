#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
VERSION="${VERSION:-1.2.4}"
ARCH="${ARCH:-$(uname -m)}"
DMG="${1:-$ROOT/dist/macos/JiyiCodex-${VERSION}-macos-${ARCH}.dmg}"
MOUNT="${TMPDIR:-/tmp}/jiyi-codex-local-install-$$"
BACKUP_ROOT="$HOME/.codex-session-delete/app-backups.noindex"
BACKUP_DIR="$BACKUP_ROOT/$(date +%Y%m%d-%H%M%S-local-install)"
MAIN_APP="/Applications/极义codex.app"
MANAGER_APP="/Applications/极义codex 管理工具.app"
OFFICIAL_CODEX_APP="/Applications/Codex.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

if [ ! -f "$DMG" ]; then
  echo "error: DMG not found: $DMG" >&2
  exit 1
fi

kill_exact_process() {
  local pattern="$1"
  local pids
  pids="$(pgrep -f "$pattern" || true)"
  if [ -n "$pids" ]; then
    echo "$pids" | while IFS= read -r pid; do
      [ -n "$pid" ] || continue
      kill "$pid" 2>/dev/null || true
    done
    sleep 1
  fi
}

plist_value() {
  local plist="$1"
  local key="$2"
  /usr/libexec/PlistBuddy -c "Print :$key" "$plist"
}

verify_bundle_id() {
  local app="$1"
  local expected="$2"
  local actual
  actual="$(plist_value "$app/Contents/Info.plist" "CFBundleIdentifier")"
  if [ "$actual" != "$expected" ]; then
    echo "error: $app bundle id is $actual, expected $expected" >&2
    exit 1
  fi
}

verify_installed_apps() {
  verify_bundle_id "$OFFICIAL_CODEX_APP" "com.openai.codex"
  verify_bundle_id "$MAIN_APP" "com.jiyi.codex"
  verify_bundle_id "$MANAGER_APP" "com.jiyi.codex.manager"
  verify_bundle_id "$MAIN_APP/Contents/Resources/JiyiCodexClient.app" "com.jiyi.codex.client"

  if [ ! -x "$MAIN_APP/Contents/MacOS/jiyi-managed-proxy" ]; then
    echo "error: missing managed proxy in $MAIN_APP" >&2
    exit 1
  fi
  if [ ! -x "$MANAGER_APP/Contents/MacOS/jiyi-managed-proxy" ]; then
    echo "error: missing managed proxy in $MANAGER_APP" >&2
    exit 1
  fi
  if [ ! -x "$MAIN_APP/Contents/Resources/server/macos/install-managed-proxy-launchd.sh" ]; then
    echo "error: missing managed proxy launchd installer in $MAIN_APP" >&2
    exit 1
  fi
  if [ ! -f "$MAIN_APP/Contents/Resources/server/macos/jiyi-managed-proxy.env.example" ]; then
    echo "error: missing managed proxy env example in $MAIN_APP" >&2
    exit 1
  fi
  if [ ! -x "$MAIN_APP/Contents/Resources/server/linux/install-managed-proxy-systemd.sh" ]; then
    echo "error: missing managed proxy systemd installer in $MAIN_APP" >&2
    exit 1
  fi
  if [ ! -f "$MAIN_APP/Contents/Resources/server/docker/Dockerfile" ]; then
    echo "error: missing managed proxy Dockerfile in $MAIN_APP" >&2
    exit 1
  fi
  if [ -d "$MAIN_APP/Contents/Resources/Codex.app" ]; then
    echo "error: legacy embedded Codex.app exists in $MAIN_APP" >&2
    exit 1
  fi
  if /usr/libexec/PlistBuddy -c 'Print :CFBundleURLTypes' \
    "$MAIN_APP/Contents/Resources/JiyiCodexClient.app/Contents/Info.plist" >/dev/null 2>&1; then
    echo "error: embedded client still declares URL schemes" >&2
    exit 1
  fi

  codesign --verify --deep --strict "$MAIN_APP"
  codesign --verify --deep --strict "$MANAGER_APP"
  codesign --verify --deep --strict "$MAIN_APP/Contents/Resources/JiyiCodexClient.app"
}

disable_backup_app_bundle() {
  local app="$1"
  [ -d "$app" ] || return 0
  "$LSREGISTER" -u "$app" >/dev/null 2>&1 || true
  local disabled="${app}.disabled"
  if [ -e "$disabled" ]; then
    disabled="${app}.disabled.$(date +%s)"
  fi
  mv "$app" "$disabled"
}

disable_existing_backup_apps() {
  [ -d "$BACKUP_ROOT" ] || return 0
  while IFS= read -r app; do
    [ -n "$app" ] || continue
    disable_backup_app_bundle "$app"
  done < <(
    find "$BACKUP_ROOT" -type d -name "*.app" -print |
      awk '{ print length($0) "\t" $0 }' |
      sort -rn |
      cut -f2-
  )
}

mkdir -p "$MOUNT" "$BACKUP_DIR"
touch "$BACKUP_ROOT/.metadata_never_index"
trap 'hdiutil detach "$MOUNT" >/dev/null 2>&1 || true; rmdir "$MOUNT" >/dev/null 2>&1 || true' EXIT

kill_exact_process "$MAIN_APP/Contents/MacOS/JiyiCodex"
kill_exact_process "$MANAGER_APP/Contents/MacOS/JiyiCodexManager"
kill_exact_process "$MAIN_APP/Contents/MacOS/jiyi-managed-proxy"
kill_exact_process "$MANAGER_APP/Contents/MacOS/jiyi-managed-proxy"
disable_existing_backup_apps

hdiutil attach -nobrowse -readonly -mountpoint "$MOUNT" "$DMG" >/dev/null

if [ -d "$MAIN_APP" ]; then
  ditto "$MAIN_APP" "$BACKUP_DIR/极义codex.app"
  disable_backup_app_bundle "$BACKUP_DIR/极义codex.app"
  rm -rf "$MAIN_APP"
fi
if [ -d "$MANAGER_APP" ]; then
  ditto "$MANAGER_APP" "$BACKUP_DIR/极义codex 管理工具.app"
  disable_backup_app_bundle "$BACKUP_DIR/极义codex 管理工具.app"
  rm -rf "$MANAGER_APP"
fi
disable_existing_backup_apps

ditto "$MOUNT/极义codex.app" "$MAIN_APP"
ditto "$MOUNT/极义codex 管理工具.app" "$MANAGER_APP"

xattr -cr "$MAIN_APP" "$MANAGER_APP" 2>/dev/null || true
codesign --force --deep --sign - "$MAIN_APP" >/dev/null
codesign --force --deep --sign - "$MANAGER_APP" >/dev/null

verify_installed_apps

"$LSREGISTER" -r -domain local -domain user >/dev/null 2>&1 || true

echo "installed: $MAIN_APP"
echo "installed: $MANAGER_APP"
echo "backup: $BACKUP_DIR"
