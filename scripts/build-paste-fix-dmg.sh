#!/usr/bin/env bash
# build-paste-fix-dmg.sh — 一键构建带"粘贴修复"功能的 Codex++ DMG
#
# 用法: bash scripts/build-paste-fix-dmg.sh [version]
#   version 可选，默认 1.2.17-pastefix.1
#
# 前置: 已安装 Rust toolchain (rustc + cargo), Node 22+, npm
#       macOS (需要 sips, iconutil, hdiutil, codesign)
#
# 输出: dist/macos/CodexPlusPlus-<version>-macos-<arch>.dmg

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

VERSION="${1:-1.2.17-pastefix.1}"
ARCH="$(uname -m)"

echo "==> 当前分支"
git branch --show-current
git log -1 --format='%H %s'

echo
echo "==> Step 1/5: 运行 Rust 单元/集成测试"
cargo test -p codex-plus-core --test paste_fix_settings --release
cargo test -p codex-plus-core --lib settings:: --release

echo
echo "==> Step 2/5: 构建 launcher (codex-plus-plus) — release"
cargo build -p codex-plus-launcher --release

echo
echo "==> Step 3/5: 构建 manager Tauri 应用 (会跑 npm install + vite build)"
cd apps/codex-plus-manager
if [ ! -d node_modules ]; then
  npm install
fi
npm run check
cd ../..
# 用 tauri build 会把 binary 放到 src-tauri/target/release
cd apps/codex-plus-manager
npm run build 2>&1 | tail -20 || {
  echo "warning: tauri build failed or returned non-zero. Will try alternate path."
}
cd ../..

# 兜底：如果 tauri build 没把 binary 放到 target/release，从 src-tauri/target/release 复制
LAUNCHER_BIN="target/release/codex-plus-plus"
MANAGER_BIN="target/release/codex-plus-plus-manager"
if [ ! -x "$MANAGER_BIN" ]; then
  echo "==> 寻找 manager binary 的备选路径"
  find apps/codex-plus-manager/src-tauri/target -name "codex-plus-plus-manager" -type f 2>/dev/null | head -3
  ALT_MANAGER=$(find apps/codex-plus-manager/src-tauri/target -name "codex-plus-plus-manager" -type f 2>/dev/null | head -1)
  if [ -n "$ALT_MANAGER" ]; then
    cp "$ALT_MANAGER" "$MANAGER_BIN"
    echo "==> Copied $ALT_MANAGER -> $MANAGER_BIN"
  fi
fi

if [ ! -x "$LAUNCHER_BIN" ]; then
  echo "error: launcher binary not found at $LAUNCHER_BIN" >&2
  exit 1
fi
if [ ! -x "$MANAGER_BIN" ]; then
  echo "error: manager binary not found at $MANAGER_BIN" >&2
  exit 1
fi

echo
echo "==> Step 4/5: 打包 DMG (version=$VERSION arch=$ARCH)"
bash scripts/installer/macos/package-dmg.sh "$VERSION" "$ARCH"

echo
echo "==> Step 5/5: 完成"
DMG="dist/macos/CodexPlusPlus-${VERSION}-macos-${ARCH}.dmg"
ls -lh "$DMG"
echo
echo "DMG 已生成: $DMG"
echo "下一步: open '$DMG' 或拖入 /Applications"
