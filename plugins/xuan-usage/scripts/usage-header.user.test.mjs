import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const source = fs.readFileSync(path.join(import.meta.dirname, "usage-header.user.js"), "utf8");

test("usage script renders the original compact panel and OwlAI daily usage", () => {
  assert.match(source, /\/v1\/usage/);
  assert.match(source, /provider === "owlai"/);
  assert.match(source, /todayUsed/);
  assert.match(source, /OwlAI 今日用量/);
  assert.match(source, /Intl\.DateTimeFormat|Intl\.NumberFormat/);
  assert.match(source, /MutationObserver/);
});

test("usage script uses the independent bridge and keeps credentials out of the renderer", () => {
  assert.match(source, /__XUAN_BRIDGE_URL__/);
  assert.match(source, /x-xuan-bridge-token/);
  assert.match(source, /用量服务暂时不可用|用量查询失败/);
  assert.match(source, /无法连接本地 Xuan Bridge/);
  assert.doesNotMatch(source, /Authorization\s*:/i);
  assert.doesNotMatch(source, /bearer\s+\$?\{/i);
});
