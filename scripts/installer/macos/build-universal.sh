#!/usr/bin/env bash
# 构建 macOS 通用（universal）安装包：一个 DMG 同时支持 Intel 与 Apple Silicon。
#
# 两个 target 各编一遍，用 lipo 把裸二进制合并成通用二进制，再交给
# package-dmg.sh 打包。后者只认 BINARY_DIR 里的文件，不关心二进制从哪来，
# 所以不需要任何改动。
set -euo pipefail

VERSION="${1:-0.0.0}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UNIVERSAL_DIR="$ROOT/target/universal/release"
BINARIES=(codex-plus-plus codex-plus-plus-manager)

# 前端产物要由调用方先构建好（vite:build），因为 manager 会把 dist 嵌进二进制。
for target in x86_64-apple-darwin aarch64-apple-darwin; do
  if ! rustup target list --installed | grep -qx "$target"; then
    echo "error: 缺少 Rust target $target，请先运行：" >&2
    echo "       rustup target add $target" >&2
    exit 1
  fi
done

cd "$ROOT"

for target in x86_64-apple-darwin aarch64-apple-darwin; do
  echo "==> 构建 $target" >&2
  cargo build --release --target "$target"
done

rm -rf "$UNIVERSAL_DIR"
mkdir -p "$UNIVERSAL_DIR"

for bin in "${BINARIES[@]}"; do
  x64="$ROOT/target/x86_64-apple-darwin/release/$bin"
  arm64="$ROOT/target/aarch64-apple-darwin/release/$bin"
  for path in "$x64" "$arm64"; do
    if [ ! -x "$path" ]; then
      echo "error: 缺少构建产物 $path" >&2
      exit 1
    fi
  done

  echo "==> 合并 $bin" >&2
  lipo -create "$x64" "$arm64" -output "$UNIVERSAL_DIR/$bin"
  chmod +x "$UNIVERSAL_DIR/$bin"

  # 断言确实合并成双架构，避免 lipo 静默产出单架构。
  archs="$(lipo -archs "$UNIVERSAL_DIR/$bin")"
  case "$archs" in
    *x86_64*arm64*|*arm64*x86_64*) ;;
    *)
      echo "error: $bin 不是通用二进制（实际架构：$archs）" >&2
      exit 1
      ;;
  esac
  echo "    $bin -> $archs" >&2
done

BINARY_DIR="$UNIVERSAL_DIR" bash "$ROOT/scripts/installer/macos/package-dmg.sh" "$VERSION" universal
