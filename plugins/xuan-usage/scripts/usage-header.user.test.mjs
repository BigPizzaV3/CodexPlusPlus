import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const source = fs.readFileSync(path.join(import.meta.dirname, "usage-header.user.js"), "utf8");

test("usage script mounts before Share and renders a compact summary", () => {
  assert.match(source, /button\[aria-label='分享当前会话'\]/);
  assert.match(source, /显示\\\/隐藏侧边面板/);
  assert.match(source, /insertAdjacentElement\("beforebegin", button\)/);
  assert.match(source, /\/v1\/usage/);
  assert.match(source, /__codexSessionDeleteBridge/);
  assert.match(source, /todayUsed/);
  assert.match(source, /今日/);
  assert.match(source, /Intl\.NumberFormat/);
});

test("usage script localizes failures without rendering provider payloads", () => {
  assert.match(source, /尚未配置用量查询/);
  assert.match(source, /用量服务暂时不可用/);
  assert.doesNotMatch(source, /innerHTML|JSON\.stringify\(body/);
  assert.doesNotMatch(source, /authorization|bearer|api[_-]?key/i);
});
