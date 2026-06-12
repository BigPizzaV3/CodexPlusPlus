#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "error: run as root, for example: sudo $0" >&2
  exit 1
fi

SERVICE_NAME="${JIYI_MANAGED_PROXY_SERVICE_NAME:-jiyi-managed-proxy}"
USER_NAME="${JIYI_MANAGED_PROXY_USER:-jiyi-codex}"
GROUP_NAME="${JIYI_MANAGED_PROXY_GROUP:-$USER_NAME}"
INSTALL_BIN="${JIYI_MANAGED_PROXY_INSTALL_BIN:-/usr/local/bin/jiyi-managed-proxy}"
CONFIG_DIR="${JIYI_MANAGED_PROXY_CONFIG_DIR:-/etc/jiyi-codex}"
DATA_DIR="${JIYI_MANAGED_PROXY_DATA_DIR:-/var/lib/jiyi-codex}"
ENV_FILE="${JIYI_MANAGED_PROXY_ENV_FILE:-$CONFIG_DIR/jiyi-managed-proxy.env}"
UNIT_PATH="${JIYI_MANAGED_PROXY_UNIT_PATH:-/etc/systemd/system/$SERVICE_NAME.service}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVICE_TEMPLATE="$SCRIPT_DIR/jiyi-managed-proxy.service"
ENV_EXAMPLE="$SCRIPT_DIR/jiyi-managed-proxy.env.example"
BINARY_SOURCE="${1:-${JIYI_MANAGED_PROXY_BINARY:-}}"

if [ -z "$BINARY_SOURCE" ]; then
  if [ -x "./target/release/jiyi-managed-proxy" ]; then
    BINARY_SOURCE="./target/release/jiyi-managed-proxy"
  elif [ -x "./jiyi-managed-proxy" ]; then
    BINARY_SOURCE="./jiyi-managed-proxy"
  else
    echo "error: pass path to jiyi-managed-proxy binary, or set JIYI_MANAGED_PROXY_BINARY" >&2
    exit 1
  fi
fi

if [ ! -x "$BINARY_SOURCE" ]; then
  echo "error: binary not executable: $BINARY_SOURCE" >&2
  exit 1
fi

if ! getent group "$GROUP_NAME" >/dev/null; then
  groupadd --system "$GROUP_NAME"
fi
if ! id "$USER_NAME" >/dev/null 2>&1; then
  useradd --system --gid "$GROUP_NAME" --home-dir "$DATA_DIR" --shell /usr/sbin/nologin "$USER_NAME"
fi

install -d -m 0750 -o "$USER_NAME" -g "$GROUP_NAME" "$DATA_DIR"
install -d -m 0750 "$CONFIG_DIR"
install -m 0755 "$BINARY_SOURCE" "$INSTALL_BIN"

if [ ! -f "$ENV_FILE" ]; then
  install -m 0600 "$ENV_EXAMPLE" "$ENV_FILE"
  echo "created env file: $ENV_FILE"
fi

install -m 0644 "$SERVICE_TEMPLATE" "$UNIT_PATH"
systemctl daemon-reload
systemctl enable "$SERVICE_NAME.service"
systemctl restart "$SERVICE_NAME.service"

echo "installed systemd service: $SERVICE_NAME"
echo "binary: $INSTALL_BIN"
echo "env: $ENV_FILE"
echo "data: $DATA_DIR"
systemctl --no-pager --full status "$SERVICE_NAME.service" || true

if ! grep -Eq '^JIYI_MANAGED_PROXY_UPSTREAM_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: upstream key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_SYNC_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: identity sync key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_ADMIN_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: admin key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_USER_READ_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: user read key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_BILLING_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: billing key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: payment webhook key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: payment webhook signature secret is not configured in $ENV_FILE"
fi
if ! grep -Eq '^(JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY|JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH)=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: alipay official public key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^(JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY|JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH)=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: wechatpay official public key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_ACCESS_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: access key is not configured in $ENV_FILE"
fi
if ! grep -Eq '^JIYI_MANAGED_PROXY_AUDIT_API_KEY=".+"' "$ENV_FILE" 2>/dev/null; then
  echo "warning: audit key is not configured in $ENV_FILE"
fi
