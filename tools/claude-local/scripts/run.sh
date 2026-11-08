#!/bin/sh
# 启动本地后端，并把 Claude.app 指到它。
#
# 用法：
#   sh scripts/run.sh demo      # 演示页（不需要外网，用来验证链路）
#   sh scripts/run.sh mock      # 真 UI + 假 /api（需要能访问 claude.ai）
#   sh scripts/run.sh passthrough   # 只观察，不改（抓请求用）
set -eu
cd "$(dirname "$0")/.."
MODE="${1:-demo}"
PORT="${PORT:-8899}"
REPO_ROOT="$(CDPATH= cd -- ../.. && pwd)"

if [ -z "${CODEX_PLUS_STATE_DIR:-}" ]; then
  if [ -d "$REPO_ROOT/dist/local-build/Codex++.app" ]; then
    CODEX_PLUS_STATE_DIR="$REPO_ROOT/dist/local-build/state"
  else
    CODEX_PLUS_STATE_DIR="$HOME/.codex-session-delete"
  fi
fi
export CODEX_PLUS_STATE_DIR

[ -f ca/ca.pem ] || sh scripts/gen-ca.sh
if ! security find-certificate -c "Claude Local CA" "$HOME/Library/Keychains/login.keychain-db" >/dev/null 2>&1; then
  echo "⚠️  本地 CA 还没被信任：先运行 sh scripts/install-ca.sh 并点「允许」"
fi

CLAUDE_RUNNING=0
if pgrep -f "/Applications/Claude.app/Contents/MacOS/Claude" >/dev/null 2>&1; then
  CLAUDE_RUNNING=1
fi

if [ "$MODE" = "mock" ] && [ "$CLAUDE_RUNNING" -eq 0 ]; then
  echo "→ 为 mock 模式准备本地听写缓存"
  CLAUDE_LOCAL_MODE=mock node scripts/patch-dictation-drain.mjs --apply
  CLAUDE_LOCAL_MODE=mock node scripts/patch-dictation-start.mjs --apply
fi

echo "→ 启动代理 127.0.0.1:$PORT (mode=$MODE)"
node proxy.mjs --port "$PORT" --mode "$MODE" &
PROXY_PID=$!
sleep 0.6

if [ "$CLAUDE_RUNNING" -eq 1 ]; then
  echo "⚠️  Claude 正在运行。请完全退出 Claude，再重新运行："
  echo "    sh scripts/run.sh $MODE"
elif [ "$MODE" = "mock" ]; then
  echo "→ 启动 Claude（mock 模式使用本地听写缓存）"
  open -a Claude --args --disable-http-cache --proxy-server="http://127.0.0.1:$PORT"
else
  echo "→ 启动 Claude（代理指向本地）"
  open -a Claude --args --proxy-server="http://127.0.0.1:$PORT"
fi

echo "代理 PID: $PROXY_PID（Ctrl-C 结束）"
wait $PROXY_PID
