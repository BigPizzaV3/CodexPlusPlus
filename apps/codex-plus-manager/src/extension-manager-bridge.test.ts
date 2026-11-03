import assert from "node:assert/strict";
import { test } from "node:test";
import vm from "node:vm";
import {
  handleManagerRequest,
  isManagerRequest,
  managerPanelSrcdoc,
  MANAGER_PANEL_BOOTSTRAP,
  MANAGER_PANEL_CSP,
  MANAGER_PANEL_MAX_HEIGHT,
  MANAGER_PANEL_MIN_HEIGHT,
  type ManagerBridgeDeps,
} from "./extension-manager-bridge.ts";

function deps(overrides: Partial<ManagerBridgeDeps> = {}) {
  const calls: unknown[][] = [];
  let config: Record<string, unknown> = { greeting: "hi" };
  const value: ManagerBridgeDeps = {
    config: () => config,
    saveConfig: async (values) => { calls.push(["save", values]); config = { ...config, ...values }; return true; },
    backendCall: async (method, params) => { calls.push(["backend", method, params]); return { echoed: params }; },
    asset: async (path) => { calls.push(["asset", path]); return "data:image/png;base64,AAAA"; },
    toast: (message) => calls.push(["toast", message]),
    resize: (height) => calls.push(["resize", height]),
    theme: () => "dark",
    ...overrides,
  };
  return { value, calls };
}

const request = (method: string, params?: unknown) => ({ __codexPlusManager: 1 as const, id: 7, method, params });

test("面板请求只认带 __codexPlusManager 标记、有 id 与方法名的消息", () => {
  assert.equal(isManagerRequest(request("config.get")), true);
  assert.equal(isManagerRequest({ id: 1, method: "config.get" }), false);
  assert.equal(isManagerRequest({ __codexPlusManager: 1, method: "config.get" }), false);
  assert.equal(isManagerRequest(null), false);
});

test("面板只能读写本拓展的配置、调本拓展的后台、取本拓展的资源", async () => {
  const { value, calls } = deps();
  assert.deepEqual(await handleManagerRequest(request("config.get"), value), { __codexPlusManager: 1, id: 7, ok: true, value: { greeting: "hi" } });
  const saved = await handleManagerRequest(request("config.set", { values: { greeting: "你好" } }), value);
  assert.deepEqual(saved.value, { greeting: "你好" }, "保存后回最新配置");
  const backend = await handleManagerRequest(request("backend.call", { method: " stats ", params: { days: 7 } }), value);
  assert.deepEqual(backend.value, { echoed: { days: 7 } });
  assert.equal((await handleManagerRequest(request("asset", { path: "logo.png" }), value)).value, "data:image/png;base64,AAAA");
  assert.equal((await handleManagerRequest(request("theme"), value)).value, "dark");
  await handleManagerRequest(request("toast", { message: "x".repeat(500) }), value);
  assert.deepEqual(calls.map((call) => call[0]), ["save", "backend", "asset", "toast"]);
  assert.equal((calls[1] as string[])[1], "stats", "方法名去掉首尾空白");
  assert.equal(((calls[3] as string[])[1]).length, 300, "提示条文字截到 300 字");
});

test("高度夹在上下限之间；参数不对、未知方法、依赖抛错都回 ok: false", async () => {
  const { value, calls } = deps({ saveConfig: async () => false, backendCall: async () => { throw new Error("后台挂了"); } });
  await handleManagerRequest(request("resize", { height: 1 }), value);
  await handleManagerRequest(request("resize", { height: 99999 }), value);
  assert.deepEqual(calls, [["resize", MANAGER_PANEL_MIN_HEIGHT], ["resize", MANAGER_PANEL_MAX_HEIGHT]]);
  for (const [method, params, error] of [
    ["resize", { height: "高" }, /高度/],
    ["backend.call", { method: "" }, /方法名/],
    ["asset", {}, /包内路径/],
    ["config.set", { values: { greeting: 1 } }, /没有保存/],
    ["backend.call", { method: "x" }, /后台挂了/],
    ["settings.set", {}, /不支持的方法/],
  ] as const) {
    const reply = await handleManagerRequest(request(method, params), value);
    assert.equal(reply.ok, false, method);
    assert.match(String(reply.error), error);
  }
});

test("srcdoc 把 CSP 与引导脚本放在 head 最前面，资源请求受限制", () => {
  const page = managerPanelSrcdoc('<!doctype html><html lang="zh"><head><title>x</title></head><body>hi</body></html>', "dark");
  const head = page.slice(page.indexOf("<head>") + 6);
  assert.ok(head.startsWith('<meta charset="utf-8"><meta http-equiv="Content-Security-Policy"'), "CSP 在页面自己的内容之前");
  assert.ok(page.indexOf("window.codexPlusManager") < page.indexOf("<title>x</title>"));
  assert.match(page, /dataset\.theme="dark"/);
  assert.match(MANAGER_PANEL_CSP, /default-src 'none'/);
  assert.doesNotMatch(MANAGER_PANEL_CSP, /connect-src/, "没有放开 connect-src：fetch / WebSocket 都被 default-src 'none' 挡住");
  assert.match(managerPanelSrcdoc("<p>片段</p>", "light"), /^<!doctype html><html><head>.*<p>片段<\/p><\/head><\/html>$/s);
  assert.match(managerPanelSrcdoc("<html><body>x</body></html>", "light"), /^<!doctype html><html><head><meta charset/);
});

test("srcdoc 的可信策略必须先于原始 HTML，不受注释和伪 head 影响", () => {
  for (const html of [
    "<!-- <head> --><html><head><title>注释</title></head><body>内容</body></html>",
    '<script>const bait = "<head>";</script><p>脚本</p>',
    "<header>页眉</header><p>片段</p>",
  ]) {
    const page = managerPanelSrcdoc(html, "light");
    assert.match(page, /^<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy"/);
    assert.ok(page.indexOf("window.codexPlusManager") < page.indexOf(html), "引导和策略出现在任何拓展内容之前");
    assert.ok(page.includes(html), "保留原始页面内容");
  }
});

test("面板策略禁止表单外发和改写基础 URL", () => {
  assert.match(MANAGER_PANEL_CSP, /(?:^|; )form-action 'none'(?:;|$)/);
  assert.match(MANAGER_PANEL_CSP, /(?:^|; )base-uri 'none'(?:;|$)/);
});

test("引导脚本在页面里提供 codexPlusManager：请求经 postMessage 发给宿主，只收宿主回的消息", async () => {
  const posted: any[] = [];
  // vm 里的对象原型与这里不同，比较前过一遍 JSON。
  const plain = (value: unknown) => JSON.parse(JSON.stringify(value));
  const listeners: Record<string, (event: any) => void> = {};
  const parent = { postMessage: (message: unknown) => posted.push(message) };
  const context: Record<string, any> = {
    parent,
    document: { documentElement: { dataset: {}, scrollHeight: 321 }, body: null },
    addEventListener: (name: string, handler: (event: any) => void) => { listeners[name] = handler; },
    setTimeout: () => 0,
    console,
    Promise, Map, Set, Object, Error,
  };
  context.window = context;
  vm.runInNewContext(MANAGER_PANEL_BOOTSTRAP, context);
  const api = context.codexPlusManager;
  assert.ok(Object.isFrozen(api), "接口对象冻结，页面脚本改不了");
  const pending = api.backend.call("stats", { days: 7 });
  assert.deepEqual(plain(posted[0]), { __codexPlusManager: 1, id: 1, method: "backend.call", params: { method: "stats", params: { days: 7 } } });
  listeners.message({ source: {}, data: { __codexPlusManager: 1, id: 1, ok: true, value: "伪造" } });
  listeners.message({ source: parent, data: { __codexPlusManager: 1, id: 1, ok: true, value: { total: 3 } } });
  assert.deepEqual(plain(await pending), { total: 3 }, "只认宿主（parent）回的消息");
  const failing = api.config.set({ a: 1 });
  listeners.message({ source: parent, data: { __codexPlusManager: 1, id: 2, ok: false, error: "不行" } });
  await assert.rejects(failing, /不行/);
  const themes: string[] = [];
  api.onTheme((theme: string) => themes.push(theme));
  listeners.message({ source: parent, data: { __codexPlusManager: 1, event: "theme", value: "light" } });
  assert.deepEqual(themes, ["light"]);
  assert.equal(context.document.documentElement.dataset.theme, "light");
  listeners.load({});
  assert.deepEqual(plain(posted.at(-1)), { __codexPlusManager: 1, id: 3, method: "resize", params: { height: 321 } }, "加载完报一次高度");
});
