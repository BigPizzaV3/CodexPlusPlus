#!/bin/sh
# 从登录钥匙串移除本地 CA。
set -eu
security remove-trusted-cert -d "$(cd "$(dirname "$0")/.." && pwd)/ca/ca.pem" 2>/dev/null || true
security delete-certificate -c "Claude Local CA" "$HOME/Library/Keychains/login.keychain-db" 2>/dev/null || true
echo "已移除本地 CA（若提示未找到，说明本来就没装）。"
