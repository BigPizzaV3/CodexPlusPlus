#!/usr/bin/env bash
# build-paste-fix-dmg.sh — 一键构建带"粘贴修复"功能的 Codex++ DMG
#
# 用法: bash scripts/build-paste-fix-dmg.sh [version] [arch]
#   version 可选，默认 1.2.17-pastefix.1
#   arch 可选：x86_64 (Intel) | arm64 (Apple Silicon) | host，默认 x86_64
#
# 前置: Rust toolchain (rustc + cargo), Node 22+, npm
#       macOS 自带: sips, iconutil, hdiutil, codesign
#
# 输出: dist/macos/CodexPlusPlus-<version>-macos-<arch>.dmg

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

VERSION="${1:-1.2.17-pastefix.1}"

# 解析 arch：第二个参数 > 环境变量 > 默认 x86_64
REQUESTED_ARCH="${2:-${CODEXPP_ARCH:-x86_64}}"
case "$REQUESTED_ARCH" in
  x86_64|intel)
    ARCH="x86_64"
    TARGET="x86_64-apple-darwin"
    ;;
  arm64|apple|apple-silicon)
    ARCH="arm64"
    TARGET="aarch64-apple-darwin"
    ;;
  host)
    HOST_ARCH_RAW="$(uname -m)"
    case "$HOST_ARCH_RAW" in
      x86_64) ARCH="x86_64"; TARGET="x86_64-apple-darwin" ;;
      arm64)  ARCH="arm64";  TARGET="aarch64-apple-darwin" ;;
      *) echo "error: unknown host arch: $HOST_ARCH_RAW" >&2; exit 1 ;;
    esac
    ;;
  *) echo "error: arch must be x86_64, arm64, or host (got: $REQUESTED_ARCH)" >&2; exit 1 ;;
esac

echo "==> 当前分支"
git branch --show-current
git log -1 --format='%H %s'

echo
echo "==> 配置: version=$VERSION arch=$ARCH target=$TARGET"
echo "    host arch = $(uname -m)"

# 如果是跨架构编译，提示用户需要先装 target
if [ "$TARGET" != "$(uname -m | sed 's/x86_64/x86_64-apple-darwin/;s/arm64/aarch64-apple-darwin/')" ]; then
  echo
  echo "==> 跨架构编译检测到。检查 rustup target 是否已安装..."
  if ! rustup target list --installed 2>/dev/null | grep -q "^$TARGET$"; then
    echo "    $TARGET 未安装。运行: rustup target add $TARGET"
    echo "    注意：跨架构编译还需要对应架构的 macOS SDK 和 linker，"
    echo "    通常通过安装 Xcode Command Line Tools + cargo-zigbuild 等价物解决。"
    echo "    推荐：本机是 $(uname -m) 时直接用 host 模式避免跨编译。"
    exit 1
  fi
fi

echo
echo "==> Step 1/6: 运行 Rust 单元/集成测试 (host target)"
cargo test -p codex-plus-core --test paste_fix_settings --release
cargo test -p codex-plus-core --lib settings:: --release

echo
echo "==> Step 2/6: 构建 launcher (codex-plus-plus) — $TARGET"
if [ "$TARGET" = "$(uname -m | sed 's/x86_64/x86_64-apple-darwin/;s/arm64/aarch64-apple-darwin/')" ]; then
  cargo build -p codex-plus-launcher --release
else
  cargo build -p codex-plus-launcher --release --target "$TARGET"
fi

echo
echo "==> Step 3/6: 构建 manager Tauri 应用 (npm install + vite build + tauri build)"
cd apps/codex-plus-manager
if [ ! -d node_modules ]; then
  npm install
fi
npm run check
# Tauri build 默认用 host 架构；跨架构需要 tauri.conf.json 配 target
npm run build 2>&1 | tail -20 || {
  echo "warning: tauri build returned non-zero. Will try alternate path."
}
cd ../..

# 兜底：如果 tauri build 没把 binary 放到 target/release，从 src-tauri/target/release 复制
LAUNCHER_BIN="target/release/codex-plus-plus"
MANAGER_BIN="target/release/codex-plus-plus-manager"
if [ ! -x "$MANAGER_BIN" ]; then
  echo "==> 寻找 manager binary 的备选路径"
  ALT_MANAGER=$(find apps/codex-plus-manager/src-tauri/target -name "codex-plus-plus-manager" -type f 2>/dev/null | head -1)
  if [ -n "$ALT_MANAGER" ]; then
    cp "$ALT_MANAGER" "$MANAGER_BIN"
    echo "==> Copied $ALT_MANAGER -> $MANAGER_BIN"
  fi
fi

# 如果 launcher 也不在 target/release，尝试从 target/$TARGET/release 复制
if [ ! -x "$LAUNCHER_BIN" ] && [ -x "target/$TARGET/release/codex-plus-plus" ]; then
  mkdir -p target/release
  cp "target/$TARGET/release/codex-plus-plus" "$LAUNCHER_BIN"
  echo "==> Copied target/$TARGET/release/codex-plus-plus -> $LAUNCHER_BIN"
fi

if [ ! -x "$LAUNCHER_BIN" ]; then
  echo "error: launcher binary not found at $LAUNCHER_BIN" >&2
  echo "       (also tried target/$TARGET/release/codex-plus-plus)" >&2
  exit 1
fi
if [ ! -x "$MANAGER_BIN" ]; then
  echo "error: manager binary not found at $MANAGER_BIN" >&2
  exit 1
fi

# 验证两个 binary 真的是目标架构
echo
echo "==> Step 4/6: 验证 binary 架构"
for bin in "$LAUNCHER_BIN" "$MANAGER_BIN"; do
  BIN_ARCH=$(lipo -archs "$bin" 2>/dev/null | tr -d ' ' || echo "unknown")
  echo "    $bin -> $BIN_ARCH"
  if [ "$ARCH" = "x86_64" ] && ! echo "$BIN_ARCH" | grep -q "x86_64"; then
    echo "error: $bin 期望 x86_64 但实际是 [$BIN_ARCH]" >&2
    exit 1
  fi
  if [ "$ARCH" = "arm64" ] && ! echo "$BIN_ARCH" | grep -q "arm64"; then
    echo "error: $bin 期望 arm64 但实际是 [$BIN_ARCH]" >&2
    exit 1
  fi
done

echo
echo "==> Step 5/6: 打包 DMG (version=$VERSION arch=$ARCH)"
bash scripts/installer/macos/package-dmg.sh "$VERSION" "$ARCH"

echo
echo "==> Step 6/6: 完成"
DMG="dist/macos/CodexPlusPlus-${VERSION}-macos-${ARCH}.dmg"
ls -lh "$DMG"
echo
echo "DMG 已生成: $DMG"
echo "下一步: open '$DMG' 或拖入 /Applications"
