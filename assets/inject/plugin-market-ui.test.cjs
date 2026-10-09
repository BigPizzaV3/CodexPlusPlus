const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

const fragmentRoot = `${__dirname}/renderer-inject`;
const source = readFileSync(`${fragmentRoot}/61-plugin-market-ui.js`, "utf8");
const flush = () => new Promise((resolve) => setImmediate(resolve));
const plugin = (id, overrides = {}) => ({
  id, name: `plugin-${id}`, displayName: `Plugin ${id}`, description: "A useful plugin", version: "1.0.0", author: "Example",
  tags: ["coding"], bytes: 1024, skills: 2, installed: false, updateAvailable: false, ...overrides,
});

function fixture(bridge) {
  const calls = [];
  const timers = new Map();
  let nextTimer = 1;
  let visible = true;
  const nodes = new Map([
    ["[data-codex-plugin-refresh]", {}], ["[data-codex-plugin-market-status]", {}],
    ["[data-codex-plugin-source-note]", {}],
    ["[data-codex-plugin-results]", {}], ["[data-codex-plugin-result-count]", {}],
    ['[data-codex-plugin-page="previous"]', {}], ['[data-codex-plugin-page="next"]', {}],
  ]);
  const sources = ["public", "full"].map((source) => ({
    dataset: { codexPluginSource: source }, attributes: {}, setAttribute(name, value) { this.attributes[name] = value; },
  }));
  const panel = { querySelector: (selector) => nodes.get(selector), querySelectorAll: () => sources };
  const context = vm.createContext({
    window: {},
    document: { querySelector: () => visible ? panel : null },
    escapeHtml: (value) => String(value).replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]),
    setTimeout: (callback) => { const id = nextTimer++; timers.set(id, callback); return id; },
    clearTimeout: (id) => timers.delete(id),
    postJson: async (path, payload) => { calls.push({ path, payload }); return await bridge(path, payload); },
  });
  vm.runInContext(`${source}\nglobalThis.ui = {
    state: codexPlusPluginMarketState, catalog: codexPlusPluginMarketCatalog,
    load: loadCodexPlusPluginMarket, render: renderCodexPlusPluginMarket,
    filtered: codexPlusPluginMarketFilteredPlugins, install: installCodexPlusPluginMarketPlugin,
    poll: pollCodexPlusPluginMarketInstall,
  };`, context);
  return {
    ui: context.ui, calls, nodes, sources, timers,
    hide: () => { visible = false; }, show: () => { visible = true; },
    runTimer: async () => { const [id, callback] = timers.entries().next().value; timers.delete(id); callback(); await flush(); },
  };
}

test("插件市场仅请求索引、按来源缓存，刷新才重新请求", async () => {
  const f = fixture(async () => ({ status: "ok", plugins: [plugin("one")], cached: true }));
  await f.ui.load("public");
  await f.ui.load("public");
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].path, "/plugin-market/list");
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0].payload)), { source: "public", refresh: false });
  assert.equal(f.ui.catalog("public").plugins.length, 1);
  await f.ui.load("public", true);
  assert.equal(f.calls.length, 2);
  assert.equal(f.calls[1].payload.refresh, true);
  assert.ok(f.calls.every((call) => call.path === "/plugin-market/list"));
});

test("插件结果分页 50、搜索名称/简介/标签、已安装筛选与 HTML 转义", async () => {
  const plugins = Array.from({ length: 121 }, (_, index) => plugin(String(index)));
  plugins[120] = plugin("tail", { tags: ["RareTag"], installed: true, description: '<img src="bad" onerror="alert(1)">' });
  const f = fixture(async () => ({ status: "ok", plugins }));
  await f.ui.load("public");
  const html = () => f.nodes.get("[data-codex-plugin-results]").innerHTML;
  assert.equal((html().match(/role="listitem"/g) || []).length, 50);
  f.ui.state.page = 3; f.ui.render();
  assert.equal((html().match(/role="listitem"/g) || []).length, 21);
  assert.equal(f.nodes.get('[data-codex-plugin-page="next"]').disabled, true);
  f.ui.state.query = "rareTAG"; f.ui.render();
  assert.equal(f.ui.state.page, 1);
  assert.equal((html().match(/role="listitem"/g) || []).length, 1);
  assert.match(html(), /&lt;img/);
  assert.doesNotMatch(html(), /<img/);
  f.ui.state.query = ""; f.ui.state.installedOnly = true; f.ui.render();
  assert.equal(f.ui.filtered().length, 1);
  assert.match(html(), /已安装/);
  assert.match(html(), /disabled/);
});

test("切换来源及关闭页面期间的旧索引响应不会写回错误视图", async () => {
  const pending = {};
  const f = fixture((_, { source }) => new Promise((resolve) => { pending[source] = resolve; }));
  const publicRequest = f.ui.load("public");
  f.ui.state.source = "full";
  const fullRequest = f.ui.load("full");
  pending.full({ status: "ok", plugins: [plugin("full-only")] }); await fullRequest;
  pending.public({ status: "ok", plugins: [plugin("public-only")] }); await publicRequest;
  const html = f.nodes.get("[data-codex-plugin-results]").innerHTML;
  assert.match(html, /full-only/);
  assert.doesNotMatch(html, /public-only/);
  f.hide();
  const refresh = f.ui.load("full", true);
  pending.full({ status: "ok", plugins: [plugin("new-full")] }); await refresh;
  assert.equal(f.nodes.get("[data-codex-plugin-results]").innerHTML, html);
  f.show(); f.ui.render();
  assert.match(f.nodes.get("[data-codex-plugin-results]").innerHTML, /new-full/);
});

test("安装桥接超时后查询同一任务，重复点击不重新发起下载", async () => {
  let statusCalls = 0;
  const installed = plugin("one", { installed: true, installedVersion: "1.0.0" });
  const f = fixture(async (path) => {
    if (path === "/plugin-market/list") return { status: "ok", plugins: [plugin("one")] };
    if (path === "/plugin-market/install") return { status: "failed", timeout: true, message: "请求超时" };
    if (path === "/plugin-market/install-status") {
      statusCalls += 1;
      return statusCalls === 1 ? { status: "ok", busy: true, stage: "downloading" } : { status: "ok", busy: false, stage: "complete", plugin: installed };
    }
    throw new Error(`Unexpected route: ${path}`);
  });
  await f.ui.load("public");
  await f.ui.install("public", "one");
  await f.ui.install("public", "one");
  assert.equal(f.calls.filter((call) => call.path === "/plugin-market/install").length, 1);
  assert.equal(f.ui.state.jobs["public:one"].busy, true);
  await f.runTimer();
  assert.equal(f.ui.state.jobs["public:one"].busy, true);
  await f.runTimer();
  assert.equal(f.ui.state.jobs["public:one"].busy, false);
  assert.equal(f.ui.catalog("public").plugins[0].installed, true);
  assert.match(f.ui.state.jobs["public:one"].message, /重启 Codex/);
  assert.equal(f.timers.size, 0);
});

test("明确安装失败停止轮询，成功安装使旧清单响应失效", async () => {
  let refreshResolve;
  let fail = true;
  const f = fixture(async (path, { refresh }) => {
    if (path === "/plugin-market/list" && refresh) return new Promise((resolve) => { refreshResolve = resolve; });
    if (path === "/plugin-market/list") return { status: "ok", plugins: [plugin("one")] };
    return fail ? { status: "failed", message: "哈希校验失败" } : { status: "ok", plugin: plugin("one", { installed: true }) };
  });
  await f.ui.load("public");
  await f.ui.install("public", "one");
  assert.equal(f.ui.state.jobs["public:one"].busy, false);
  assert.equal(f.ui.state.jobs["public:one"].failed, true);
  assert.equal(f.timers.size, 0);
  fail = false;
  const refresh = f.ui.load("public", true);
  await f.ui.install("public", "one");
  refreshResolve({ status: "ok", plugins: [plugin("one")] }); await refresh;
  assert.equal(f.ui.catalog("public").plugins[0].installed, true);
});

test("状态接口调用成功但 stage 为 failed 或未知时不能误报安装完成", async () => {
  for (const result of [
    { status: "ok", busy: false, stage: "failed", message: "上一次安装已中断，可重新安装。" },
    { status: "ok", busy: false, stage: "idle" },
    { status: "ok", busy: false, stage: "complete" },
  ]) {
    const f = fixture(async (path) => path === "/plugin-market/list" ? { status: "ok", plugins: [plugin("one")] }
      : path === "/plugin-market/install" ? { status: "failed", timeout: true } : result);
    await f.ui.load("public");
    await f.ui.install("public", "one");
    await f.runTimer();
    assert.equal(f.ui.state.jobs["public:one"].busy, false);
    assert.equal(f.ui.state.jobs["public:one"].failed, true);
    assert.equal(f.ui.catalog("public").plugins[0].installed, false);
    assert.equal(f.timers.size, 0);
  }
});

test("完整市场与公开市场同 id 的安装状态保持独立", async () => {
  const f = fixture(async (path) => path === "/plugin-market/list" ? { status: "ok", plugins: [plugin("one")] }
    : { status: "ok", plugin: plugin("one", { installed: true, installedVersion: "1.0.0" }) });
  await f.ui.load("public");
  await f.ui.load("full");
  await f.ui.install("public", "one");
  assert.equal(f.ui.catalog("public").plugins[0].installed, true);
  assert.equal(f.ui.catalog("full").plugins[0].installed, false);
  f.ui.state.source = "full"; f.ui.render();
  assert.equal(f.nodes.get("[data-codex-plugin-source-note]").hidden, false);
  assert.match(f.nodes.get("[data-codex-plugin-source-note]").textContent, /GitHub 私有仓库访问权限/);
  f.ui.state.source = "public"; f.ui.render();
  assert.equal(f.nodes.get("[data-codex-plugin-source-note]").hidden, true);
});

test("两个导航布局只保留原生插件入口，旧按钮清理、市场能力与第三方路由契约不变", () => {
  const navigation = readFileSync(`${fragmentRoot}/40-backend-settings.js`, "utf8");
  const entryNavigation = readFileSync(`${fragmentRoot}/50-navigation.js`, "utf8");
  const nativeAdapter = readFileSync(`${fragmentRoot}/62-plugin-market-adapter.js`, "utf8");
  const extensionApi = readFileSync(`${fragmentRoot}/91-extension-api.js`, "utf8");
  const manifest = JSON.parse(readFileSync(`${fragmentRoot}/manifest.json`, "utf8"));
  assert.doesNotMatch(navigation, /id: codexPlusRailPluginMarketId, label: "CodeX 插件市场"/);
  assert.doesNotMatch(navigation, /\[codexPlusRailPluginMarketId, "plugin-market"\]/);
  assert.doesNotMatch(navigation, /installCodexPlusPluginMarketSidebarNavigation\(/);
  assert.doesNotMatch(source, /function (?:codexPlusPluginMarketIconMarkup|installCodexPlusPluginMarketSidebarNavigation)\(/);
  assert.match(entryNavigation, /document\.getElementById\(codexPlusRailPluginMarketId\)\?\.remove\(\)/);
  assert.match(entryNavigation, /document\.getElementById\(codexPlusSidebarPluginMarketId\)\?\.remove\(\)/);
  assert.match(nativeAdapter, /function codexPlusNativePluginNavigationEntry\(/);
  assert.match(nativeAdapter, /function openCodexPlusNativePluginMarket\(/);
  assert.match(nativeAdapter, /native\.click\(\)/);
  assert.match(source, /markCodexPlusExtensionNode\(overlay, "builtin-plugin-market"\)/);
  assert.doesNotMatch(source, /(?:codexStateCall|sendRequest|postJson)\(["']plugin\/install["']/);
  assert.doesNotMatch(extensionApi, /["']\/plugin-market\/(?:install|list|install-status)["']/);
  assert.ok(manifest.fragments.some((fragment) => fragment.name === "61-plugin-market-ui.js"));
});
