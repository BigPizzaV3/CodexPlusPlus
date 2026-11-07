#!/bin/sh
# 端到端自检：起代理 → 用 curl 走代理 + 本地 CA → 断言假后端生效。
set -eu
cd "$(dirname "$0")/.."
PORT="${PORT:-8899}"

[ -f ca/ca.pem ] || sh scripts/gen-ca.sh

node proxy.mjs --port "$PORT" --mode demo > /tmp/claude-local-test.log 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
sleep 0.8

fail=0
echo "== 1. /api/bootstrap"
OUT=$(curl -s --max-time 10 -x "http://127.0.0.1:$PORT" --cacert ca/ca.pem https://claude.ai/api/bootstrap)
echo "$OUT" | head -c 200; echo
case "$OUT" in
  *'"plan":"max"'*|*'"plan": "max"'*) echo "  OK plan=max" ;;
  *) echo "  FAIL 没有 plan=max"; fail=1 ;;
esac

echo "== 2. / (demo 页面)"
CODE=$(curl -s -o /tmp/claude-local-demo.html -w '%{http_code}' --max-time 10 \
  -x "http://127.0.0.1:$PORT" --cacert ca/ca.pem https://claude.ai/)
echo "  HTTP $CODE, $(wc -c < /tmp/claude-local-demo.html) bytes"
grep -q "Claude（本地后端）" /tmp/claude-local-demo.html && echo "  OK 演示页" || { echo "  FAIL 演示页"; fail=1; }

echo "== 3. /api/organizations"
curl -s --max-time 10 -x "http://127.0.0.1:$PORT" --cacert ca/ca.pem https://claude.ai/api/organizations | head -c 200; echo

if [ "$fail" -eq 0 ]; then echo "\\n全部通过"; else echo "\\n有失败项"; exit 1; fi
