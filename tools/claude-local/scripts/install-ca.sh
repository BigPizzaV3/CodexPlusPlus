#!/bin/sh
# 把本地 CA 装进登录钥匙串（信任 SSL）。需要系统弹窗点一次「允许」。
set -eu
cd "$(dirname "$0")/.."
CERT="$PWD/ca/ca.pem"
[ -f "$CERT" ] || { echo "先运行 scripts/gen-ca.sh"; exit 1; }

echo "即将把本地 CA 加入登录钥匙串并信任："
echo "  $CERT"
echo "如果弹出授权框，输入登录密码即可（只影响当前用户）。"
security add-trusted-cert -r trustRoot -p ssl \
  -k "$HOME/Library/Keychains/login.keychain-db" "$CERT"

echo "已安装。验证："
security find-certificate -c "Claude Local CA" -Z "$HOME/Library/Keychains/login.keychain-db" | head -3
