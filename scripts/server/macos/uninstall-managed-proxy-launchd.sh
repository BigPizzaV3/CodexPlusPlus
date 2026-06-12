#!/usr/bin/env bash
set -euo pipefail

LABEL="${JIYI_MANAGED_PROXY_LAUNCHD_LABEL:-com.jiyi.codex.managed-proxy}"
STATE_DIR="${JIYI_CODEX_STATE_DIR:-$HOME/.codex-session-delete}"
ENV_FILE="${JIYI_MANAGED_PROXY_ENV_FILE:-$STATE_DIR/jiyi-managed-proxy.env}"
RUNTIME_BINARY="${JIYI_MANAGED_PROXY_RUNTIME_BINARY:-$STATE_DIR/bin/jiyi-managed-proxy}"
RUNNER="$STATE_DIR/run-jiyi-managed-proxy-launchd.sh"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

launchctl bootout "gui/$(id -u)" "$PLIST" >/dev/null 2>&1 || true
launchctl remove "$LABEL" >/dev/null 2>&1 || true
rm -f "$PLIST" "$RUNNER" "$RUNTIME_BINARY"

if [ "${1:-}" = "--purge-env" ]; then
  rm -f "$ENV_FILE"
  echo "removed env file: $ENV_FILE"
fi

echo "uninstalled launchd service: $LABEL"
