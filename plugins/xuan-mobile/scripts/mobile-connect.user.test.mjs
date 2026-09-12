import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const source = fs.readFileSync(path.join(import.meta.dirname, "mobile-connect.user.js"), "utf8");

test("mobile script exposes the desktop pairing entry and QR workflow", () => {
  assert.match(source, /手机连接/);
  assert.match(source, /生成绑定二维码/);
  assert.match(source, /手机绑定二维码/);
  assert.match(source, /等待手机扫码/);
  assert.match(source, /\/v1\/mobile\/pair/);
});

test("mobile script supports local confirmation and task synchronization", () => {
  assert.match(source, /确认绑定/);
  assert.match(source, /拒绝/);
  assert.match(source, /\/v1\/mobile\/confirm/);
  assert.match(source, /\/v1\/mobile\/tasks/);
  assert.match(source, /\/v1\/mobile\/select/);
  assert.match(source, /__XUAN_BRIDGE_URL__/);
});
