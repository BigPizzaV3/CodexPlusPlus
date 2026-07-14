#!/usr/bin/env bash
# Codex++ 本地打包成 .dmg。
#
# 流程：前端 typecheck + vite build → 编译两个 release binary → 调 package-dmg.sh。
#
# 用法：
#   bash scripts/package-dmg.sh                    # 默认版本号 (从 Cargo.toml workspace 读) + 当前架构
#   bash scripts/package-dmg.sh 1.2.35             # 指定版本号
#   bash scripts/package-dmg.sh 1.2.35 arm64       # 指定版本号 + 架构 (arm64 | x86_64)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-}"
ARCH="${2:-$(uname -m)}"

# 没传版本号就从 workspace Cargo.toml 读
if [ -z "$VERSION" ]; then
  VERSION=$(grep '^version' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
  echo "==> 从 Cargo.toml 读版本号: $VERSION"
fi

echo "==> [1/4] 安装前端依赖"
cd apps/codex-plus-manager
npm install --package-lock=false

echo "==> [2/4] 前端类型检查 + 构建"
npm run check
npm run vite:build

echo "==> [3/4] 编译两个 release binary"
cd "$ROOT"
cargo build --release

echo "==> [4/4] 打 DMG（version=${VERSION} arch=${ARCH}）"
BINARY_DIR="$ROOT/target/release" bash scripts/installer/macos/package-dmg.sh "$VERSION" "$ARCH"

echo ""
echo "==> 完成！DMG 位置："
ls -lh dist/macos/*.dmg 2>/dev/null || echo "(没找到 dmg，请检查上面的输出)"
