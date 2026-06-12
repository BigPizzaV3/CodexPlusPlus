#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "error: run as root, for example: sudo $0" >&2
  exit 1
fi

SERVICE_NAME="${JIYI_MANAGED_PROXY_SERVICE_NAME:-jiyi-managed-proxy}"
CONFIG_DIR="${JIYI_MANAGED_PROXY_CONFIG_DIR:-/etc/jiyi-codex}"
DATA_DIR="${JIYI_MANAGED_PROXY_DATA_DIR:-/var/lib/jiyi-codex}"
ENV_FILE="${JIYI_MANAGED_PROXY_ENV_FILE:-$CONFIG_DIR/jiyi-managed-proxy.env}"
UNIT_PATH="${JIYI_MANAGED_PROXY_UNIT_PATH:-/etc/systemd/system/$SERVICE_NAME.service}"
INSTALL_BIN="${JIYI_MANAGED_PROXY_INSTALL_BIN:-/usr/local/bin/jiyi-managed-proxy}"

systemctl disable --now "$SERVICE_NAME.service" >/dev/null 2>&1 || true
rm -f "$UNIT_PATH" "$INSTALL_BIN"
systemctl daemon-reload

if [ "${1:-}" = "--purge-env" ]; then
  rm -f "$ENV_FILE"
  rmdir "$CONFIG_DIR" >/dev/null 2>&1 || true
  echo "removed env file: $ENV_FILE"
fi
if [ "${1:-}" = "--purge-data" ] || [ "${2:-}" = "--purge-data" ]; then
  rm -rf "$DATA_DIR"
  echo "removed data dir: $DATA_DIR"
fi

echo "uninstalled systemd service: $SERVICE_NAME"

