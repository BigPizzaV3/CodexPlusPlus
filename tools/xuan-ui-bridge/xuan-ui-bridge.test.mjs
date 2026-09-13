import test from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import http from "node:http";
import net from "node:net";
import fs from "node:fs";
import path from "node:path";
import { once } from "node:events";
import { adapterScript, contracts, dispatchUi, isAppPage, startPluginUi, validateSocket } from "./xuan-ui-bridge.mjs";

const root = path.resolve(import.meta.dirname, "../..");

test("全部插件界面路由经独立进程分发，参数保持不变", async () => {
  for (const [name, { routes }] of Object.entries(contracts)) {
    for (const [route, method] of Object.entries(routes)) {
      let calls = 0;
      const value = await dispatchUi(name, JSON.stringify({ id: "request", route, payload: { marker: route } }), async (actual, params) => {
        calls++;
        assert.equal(actual, method);
        assert.deepEqual(params, { marker: route });
        return { status: "ok" };
      });
      assert.equal(value.status, "ok");
      assert.equal(calls, 1);
    }
  }
});

test("插件路由拒绝跨插件操作、远程地址和超限请求", async () => {
  const rejectDispatch = () => { throw new Error("不应调用后端"); };
  for (const route of ["/v1/mobile/pair", "/v1/paths", "/v1/polish?url=http://evil.test", "__proto__", "http://127.0.0.1/v1/polish"]) {
    await assert.rejects(dispatchUi("xuan-polish", JSON.stringify({ id: "request", route, payload: {} }), rejectDispatch), /不受支持/);
  }
  await assert.rejects(dispatchUi("xuan-polish", "x".repeat(1_048_577), rejectDispatch), /过大/);
  await assert.rejects(dispatchUi("xuan-polish", "{}", rejectDispatch), /不受支持/);
  assert.equal(isAppPage({ type: "page", url: "app://-/index.html" }), true);
  for (const url of ["https://example.com", "app://-/browser.html", "app://-evil/index.html"]) {
    assert.equal(isAppPage({ type: "page", url }), false);
  }
  assert.equal(validateSocket("ws://127.0.0.1:9229/devtools/page/test", 9229), "ws://127.0.0.1:9229/devtools/page/test");
  for (const url of ["ws://example.com:9229/devtools/page/test", "ws://127.0.0.1:80/devtools/page/test", "ws://127.0.0.1:9229/devtools/page/test?token=test"]) {
    assert.throws(() => validateSocket(url, 9229), /不受支持/);
  }
});

function page() {
  const calls = [];
  const timers = new Map();
  let next = 0;
  const window = {
    location: { origin: "app://-" },
    localBinding: (value) => calls.push(JSON.parse(value)),
    setTimeout(callback) { const id = ++next; timers.set(id, callback); return id; },
    clearTimeout(id) { timers.delete(id); },
    __codexSessionDeleteBridge() { throw new Error("不应调用宿主路由"); },
  };
  window.top = window;
  const context = vm.createContext({ window });
  return { calls, timers, window, install(owner) { vm.runInContext(adapterScript("xuan-polish", "localBinding", owner), context); } };
}

test("页面通道不修改宿主函数，响应与超时均清理回调", async () => {
  const fixture = page();
  const host = fixture.window.__codexSessionDeleteBridge;
  fixture.install("owner");
  const api = fixture.window.__xuanPluginBridge["xuan-polish"];
  const pending = api("/v1/polish", { text: "草稿" });
  assert.equal(fixture.calls.length, 1);
  const result = { status: "ok" };
  api.resolve(fixture.calls[0].id, result);
  assert.equal(await pending, result);
  assert.equal(fixture.timers.size, 0);
  assert.equal(fixture.window.__codexSessionDeleteBridge, host);
  const timeout = api("/v1/polish/settings", {});
  const check = assert.rejects(timeout, /超时/);
  [...fixture.timers.values()][0]();
  await check;
  api.dispose();
  assert.equal(fixture.window.__xuanPluginBridge["xuan-polish"], undefined);
});

test("重连撤销旧回调但不重放写请求，其他插件保持不变", async () => {
  const fixture = page();
  fixture.install("old");
  fixture.window.__xuanPluginBridge.other = () => "unchanged";
  const pending = fixture.window.__xuanPluginBridge["xuan-polish"]("/v1/polish/settings/set", {});
  const check = assert.rejects(pending, /连接已更新/);
  fixture.install("new");
  await check;
  assert.equal(fixture.calls.length, 1);
  assert.equal(fixture.timers.size, 0);
  assert.equal(fixture.window.__xuanPluginBridge.other(), "unchanged");
});

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(condition) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (condition()) return;
    await delay(25);
  }
  assert.fail("等待插件生命周期状态超时");
}

test("多个任务仅一个插件实例接管界面，退出后自动移交且释放端口", async () => {
  const server = http.createServer((request, response) => { response.writeHead(200); response.end("[]"); });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const reservation = net.createServer();
  reservation.listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const lockPort = reservation.address().port;
  await new Promise((resolve) => reservation.close(resolve));
  const options = { name: "xuan-polish", request: async () => ({}), debugPort: server.address().port, lockPort, intervalMs: 25 };
  const first = startPluginUi(options);
  const second = startPluginUi(options);
  try {
    await until(() => first.leader || second.leader);
    await delay(100);
    assert.notEqual(first.leader, second.leader);
    const leader = first.leader ? first : second;
    const follower = first.leader ? second : first;
    await leader.close();
    await until(() => follower.leader);
  } finally {
    await first.close();
    await second.close();
    await new Promise((resolve) => server.close(resolve));
  }
  const reused = net.createServer();
  reused.listen(lockPort, "127.0.0.1");
  await once(reused, "listening");
  await new Promise((resolve) => reused.close(resolve));
});

test("四个用户脚本和宿主源码不再依赖定制转发或服务启动钩子", () => {
  for (const name of Object.keys(contracts)) {
    const directory = path.join(root, "plugins", name, "scripts");
    const file = fs.readdirSync(directory).find((entry) => entry.endsWith(".user.js"));
    const source = fs.readFileSync(path.join(directory, file), "utf8");
    assert.match(source, /__xuanPluginBridge/);
    assert.doesNotMatch(source, /__codexSessionDeleteBridge|127\.0\.0\.1:57324/);
  }
  for (const file of ["apps/codex-plus-launcher/src/main.rs", "crates/codex-plus-core/src/routes.rs"]) {
    assert.doesNotMatch(fs.readFileSync(path.join(root, file), "utf8"), /xuan_bridge|XuanBridgeServices|57324|17421/);
  }
  const installer = fs.readFileSync(path.join(root, "install-xuan-features.bat"), "utf8");
  assert.doesNotMatch(installer, /taskkill|setx /i);
  assert.match(installer, /install-xuan-runtime\.ps1/);
});
