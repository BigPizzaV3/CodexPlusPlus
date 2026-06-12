#!/usr/bin/env bash
set -euo pipefail

LABEL="${JIYI_MANAGED_PROXY_LAUNCHD_LABEL:-com.jiyi.codex.managed-proxy}"
APP_PATH="${JIYI_CODEX_APP_PATH:-/Applications/极义codex.app}"
BINARY="$APP_PATH/Contents/MacOS/jiyi-managed-proxy"
STATE_DIR="${JIYI_CODEX_STATE_DIR:-$HOME/.codex-session-delete}"
BIN_DIR="$STATE_DIR/bin"
RUNTIME_BINARY="${JIYI_MANAGED_PROXY_RUNTIME_BINARY:-$BIN_DIR/jiyi-managed-proxy}"
ENV_FILE="${JIYI_MANAGED_PROXY_ENV_FILE:-$STATE_DIR/jiyi-managed-proxy.env}"
LOG_DIR="$STATE_DIR/logs"
RUNNER="$STATE_DIR/run-jiyi-managed-proxy-launchd.sh"
PLIST_DIR="$HOME/Library/LaunchAgents"
PLIST="$PLIST_DIR/$LABEL.plist"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_EXAMPLE="$SCRIPT_DIR/jiyi-managed-proxy.env.example"

shell_quote() {
  printf '%q' "$1"
}

xml_escape() {
  local value="$1"
  value="${value//&/&amp;}"
  value="${value//</&lt;}"
  value="${value//>/&gt;}"
  value="${value//\"/&quot;}"
  printf '%s' "$value"
}

if [ ! -x "$BINARY" ]; then
  echo "error: jiyi-managed-proxy not found or not executable: $BINARY" >&2
  echo "hint: install the full 极义codex DMG first, or set JIYI_CODEX_APP_PATH=/path/to/极义codex.app" >&2
  exit 1
fi

mkdir -p "$STATE_DIR" "$BIN_DIR" "$LOG_DIR" "$PLIST_DIR"
cp "$BINARY" "$RUNTIME_BINARY"
chmod 755 "$RUNTIME_BINARY"

if [ ! -f "$ENV_FILE" ]; then
  if [ -f "$ENV_EXAMPLE" ]; then
    cp "$ENV_EXAMPLE" "$ENV_FILE"
  else
    touch "$ENV_FILE"
  fi
  chmod 600 "$ENV_FILE"
  echo "created env file: $ENV_FILE"
fi

cat > "$RUNNER" <<RUNNER
#!/usr/bin/env bash
set -euo pipefail

ENV_FILE=$(shell_quote "$ENV_FILE")
BINARY=$(shell_quote "$RUNTIME_BINARY")

if [ -f "\$ENV_FILE" ]; then
  set -a
  # shellcheck disable=SC1090
  source "\$ENV_FILE"
  set +a
fi

exec "\$BINARY"
RUNNER
chmod 700 "$RUNNER"

cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$(xml_escape "$LABEL")</string>
  <key>ProgramArguments</key>
  <array>
    <string>$(xml_escape "$RUNNER")</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  <string>$(xml_escape "$LOG_DIR/jiyi-managed-proxy.out.log")</string>
  <key>StandardErrorPath</key>
  <string>$(xml_escape "$LOG_DIR/jiyi-managed-proxy.err.log")</string>
  <key>WorkingDirectory</key>
  <string>$(xml_escape "$STATE_DIR")</string>
</dict>
</plist>
PLIST

plutil -lint "$PLIST" >/dev/null
launchctl bootout "gui/$(id -u)" "$PLIST" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
launchctl kickstart -k "gui/$(id -u)/$LABEL"

echo "installed launchd service: $LABEL"
echo "plist: $PLIST"
echo "binary: $RUNTIME_BINARY"
echo "env: $ENV_FILE"
echo "logs: $LOG_DIR/jiyi-managed-proxy.out.log / $LOG_DIR/jiyi-managed-proxy.err.log"
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
