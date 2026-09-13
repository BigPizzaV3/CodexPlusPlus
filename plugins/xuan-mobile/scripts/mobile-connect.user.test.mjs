import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const source = fs.readFileSync(path.join(import.meta.dirname, "mobile-connect.user.js"), "utf8");
const routes = ["status", "pair", "enable", "confirm", "auto-sync", "select", "tasks"];

function adapter(pageBridge, fetch = () => { throw new Error("不应直连本地端口"); }) {
  const timers = new Map();
  const context = vm.createContext({
    window: {
      __xuanPluginBridge: { "xuan-mobile": pageBridge },
      __codexSessionDeleteBridge: () => { throw new Error("不应调用宿主扩展路由"); },
      setTimeout(callback) { timers.set(1, callback); return 1; },
      clearTimeout(id) { timers.delete(id); },
    },
    fetch,
    AbortController,
    bridgeUrl: "http://127.0.0.1:57324",
    bridgeToken: "test-local-token",
  });
  const start = source.indexOf("  async function request(");
  const end = source.indexOf("  function ensureStyle()", start);
  assert.ok(start >= 0 && end > start);
  vm.runInContext(source.slice(start, end), context);
  return { call: context.request, timers };
}

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
  assert.match(source, /__xuanPluginBridge/);
  assert.doesNotMatch(source, /__codexSessionDeleteBridge|127\.0\.0\.1:57324/);
});

test("手机全部界面接口通过页面桥接，直连阻断不影响转发", async () => {
  const calls = [];
  const response = { enabled: true, bound: false };
  const { call, timers } = adapter((path, payload) => {
    calls.push({ path, payload });
    return Promise.resolve(response);
  });
  for (const route of routes) {
    const path = `/v1/mobile/${route}`;
    const payload = { enabled: true, confirmed: false, selected: ["test-task"] };
    assert.equal(await call(path, payload, route === "status" ? "GET" : "POST"), response);
    assert.deepEqual(calls.at(-1), { path, payload });
    assert.equal(timers.size, 0);
  }
});

test("手机插件未连接时不回退宿主或 HTTP", async () => {
  const { call } = adapter(undefined);
  for (const route of routes) {
    await assert.rejects(call(`/v1/mobile/${route}`, {}), /插件尚未连接/);
  }
});

test("手机桥接失败不重复绑定或确认，错误转换为中文", async () => {
  for (const bridge of [
    () => { throw new Error("Failed to fetch (127.0.0.1:57324)"); },
    () => Promise.reject(new Error("networkerror")),
    () => ({ status: "failed", message: "Unknown bridge path" }),
    () => ({ status: "failed", error: { message: "绑定请求已过期" } }),
    () => null,
  ]) {
    const { call, timers } = adapter(bridge);
    await assert.rejects(call("/v1/mobile/confirm", { confirmed: true }), (error) => {
      assert.match(error.message, /\p{Script=Han}/u);
      assert.doesNotMatch(error.message, /Failed to fetch|127\.0\.0\.1|\[object Object\]/);
      return true;
    });
    assert.equal(timers.size, 0);
  }
});

test("手机业务错误和超时不会被当作正常状态", async () => {
  const { call } = adapter(async () => ({ status: "failed", error: { message: "手机暂未连接" } }));
  await assert.rejects(call("/v1/mobile/pair", {}), /手机暂未连接/);
  const stalled = adapter(() => new Promise(() => {}));
  const pending = stalled.call("/v1/mobile/status", null, "GET");
  stalled.timers.get(1)();
  await assert.rejects(pending, /超时/);
  assert.equal(stalled.timers.size, 0);
});
