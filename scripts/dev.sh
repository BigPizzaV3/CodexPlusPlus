#!/usr/bin/env bash
# Codex++ 本地 dev 入口。
#
# 解决问题：之前 `cargo build` 只会编 manager，缺 launcher 导致 Codex++ 重启失败。
# 这个脚本先编两个 debug binary，再起 Tauri dev。
#
# 用法：
#   bash scripts/dev.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> [1/2] 编译两个 debug binary（manager + launcher）"
cargo build --bin codex-plus-plus --bin codex-plus-plus-manager

echo "==> [2/2] 启动 Tauri dev（前端 + manager 窗口）"
cd apps/codex-plus-manager
npm run dev
