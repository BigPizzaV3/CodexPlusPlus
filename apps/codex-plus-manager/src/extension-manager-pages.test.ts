import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import {
  keepUserScriptRuntime, MANAGER_DATA_SOURCES, managerDataRequest, managerExtensionPages, managerNavPages, managerPageOverrideConflicts,
  managerPageOverrides, scriptManifestError,
} from "./extension-manager-pages.ts";
import { handleManagerRequest, MANAGER_PANEL_BOOTSTRAP, type ManagerBridgeDeps } from "./extension-manager-bridge.ts";
import { isRouteAlias } from "./route-aliases.ts";
import { readAppSource } from "./app-source.test-helper.ts";

const app = readAppSource();
const rust = readFileSync(new URL("../../../crates/codex-plus-core/src/user_scripts.rs", import.meta.url), "utf8");

/** Rust 侧 `pub const NAME: &[&str] = &[...]` 里的字符串。 */
function rustList(name: string): string[] {
  const start = rust.indexOf(`pub const ${name}: &[&str] = &[`);
  assert.ok(start >= 0, name);
  const body = rust.slice(start, rust.indexOf("];", start));
  return [...body.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
}

const inventory = {
  enabled: true,
  scripts: [
    { key: "user:a", name: "A", enabled: true, error: "", package: { managerPage: { title: "看板", nav: "management" as const, icon: "bar-chart", replaces: ["gatewayUsage"], data: ["requestLogs"] } } },
    { key: "user:b", name: "B", enabled: true, error: "", package: { managerPage: { title: "B 页", replaces: ["gatewayUsage", "hub"] } } },
    { key: "user:off", name: "Off", enabled: false, error: "", package: { managerPage: { title: "停用", nav: "tools" as const } } },
    { key: "user:broken", name: "Broken", enabled: true, error: "清单有误", package: { managerPage: { title: "坏", nav: "tools" as const } } },
    { key: "user:plain", name: "Plain", enabled: true, error: "", package: null },
  ],
};

test("拓展页面只取启用中、清单无误的；先声明的拓展优先顶替", () => {
  const pages = managerExtensionPages(inventory);
  assert.deepEqual(pages.map((page) => page.key), ["user:a", "user:b"]);
  assert.deepEqual(managerNavPages(pages, "management").map((page) => page.key), ["user:a"]);
  assert.deepEqual(managerNavPages(pages, "tools"), []);
  const overrides = managerPageOverrides(pages);
  assert.equal(overrides.gatewayUsage.key, "user:a");
  assert.equal(overrides.hub.key, "user:b");
  assert.deepEqual(managerExtensionPages({ ...inventory, enabled: false }), [], "用户脚本总开关关着时没有拓展页面");
});

test("管理器页面可用与否只看清单错误：拓展在 Codex 页面里运行出错不让它的页面消失", () => {
  const page = { managerPage: { title: "看板", nav: "tools" as const, replaces: ["gatewayUsage"] } };
  const script = (key: string, extra: Record<string, unknown>) => ({ key, name: key, enabled: true, package: page, ...extra });
  const pages = managerExtensionPages({
    enabled: true,
    scripts: [
      // 带运行状态的清单：renderer 在 Codex 里抛了错，清单本身没问题。
      script("user:runtime-error", { status: "failed", error: "TypeError: boom", manifestError: "" }),
      // 清单本身有错：error 与 manifestError 是同一句。
      script("user:bad-manifest", { status: "failed", error: "codexpp.json 不是合法 JSON", manifestError: "codexpp.json 不是合法 JSON" }),
      // 旧版后端没有 manifestError：退回看 error。
      script("user:legacy-error", { status: "failed", error: "旧后端报的错" }),
      script("user:legacy-ok", { status: "loaded", error: "" }),
    ],
  });
  assert.deepEqual(pages.map((item) => item.key), ["user:runtime-error", "user:legacy-ok"]);
  // 运行出错的拓展照样顶替它声明的页面：不带运行状态的清单再回来时结果不变。
  assert.equal(managerPageOverrides(pages).gatewayUsage.key, "user:runtime-error");
  assert.equal(scriptManifestError({ error: "运行时报错", manifestError: "" }), "");
  assert.equal(scriptManifestError({ error: "坏清单", manifestError: "坏清单" }), "坏清单");
  assert.equal(scriptManifestError({ error: "旧后端" }), "旧后端");
  assert.equal(scriptManifestError({}), "");
});

test("两个拓展顶替同一页：先声明的生效，后者得到「被谁抢先了哪些页」", () => {
  const page = (key: string, replaces: string[]) => ({ key, title: `${key} 页`, nav: null, icon: null, replaces, data: [] });
  const first = page("user:first", ["gatewayUsage", "hub"]);
  const second = page("user:second", ["hub", "sessions"]);
  const third = page("user:third", ["gatewayUsage", "hub", "sessions", "context", "context"]);
  const conflicts = managerPageOverrideConflicts([first, second, third]);
  assert.deepEqual(Object.keys(conflicts), ["user:second", "user:third"], "没有冲突的拓展不出现");
  assert.deepEqual(conflicts["user:second"], [{ by: first, routes: ["hub"] }]);
  // 按抢先者归并；自己顶替上的（context）不算，重复声明的路由只算一次。
  assert.deepEqual(conflicts["user:third"], [{ by: first, routes: ["gatewayUsage", "hub"] }, { by: second, routes: ["sessions"] }]);
  // 与 managerPageOverrides 是同一条规则：冲突里的抢先者就是实际生效的那个。
  const overrides = managerPageOverrides([first, second, third]);
  for (const list of Object.values(conflicts)) {
    for (const conflict of list) for (const route of conflict.routes) assert.equal(overrides[route], conflict.by);
  }
  assert.equal(overrides.context.key, "user:third");
  assert.deepEqual(managerPageOverrideConflicts([first]), {});
  assert.deepEqual(managerPageOverrideConflicts([]), {});
  // 从清单一路算过来：停用的拓展不参与，自然也不算抢先者。
  const pages = managerExtensionPages(inventory);
  assert.deepEqual(managerPageOverrideConflicts(pages), { "user:b": [{ by: pages[0], routes: ["gatewayUsage"] }] });
});

test("不带运行状态的清单沿用上一份的运行状态；清单错误、新增、启停变化以新清单为准", () => {
  const item = (key: string, status: string, extra: Record<string, unknown> = {}) => ({ key, name: key, enabled: status !== "disabled", status, error: "", manifestError: "", ...extra });
  // 上一份：问过 Codex 页面的清单。
  const previous = {
    enabled: true,
    scripts: [
      item("user:loaded", "loaded"),
      item("user:crashed", "failed", { error: "TypeError: boom" }),
      item("user:pending", "unknown"),
      item("user:off", "disabled"),
      item("user:toggled-off", "loaded"),
      item("user:was-bad", "failed", { error: "坏清单", manifestError: "坏清单" }),
      item("user:now-bad", "loaded"),
      item("user:removed", "loaded"),
    ],
  };
  // 新的一份：只扫了磁盘，启用的一律是兜底的 not_loaded。
  const next = {
    enabled: true,
    skip_next_start: true,
    scripts: [
      item("user:loaded", "not_loaded", { name: "改过名" }),
      item("user:crashed", "not_loaded"),
      item("user:pending", "not_loaded"),
      item("user:off", "not_loaded", { enabled: true }),
      item("user:toggled-off", "disabled"),
      item("user:was-bad", "not_loaded"),
      item("user:now-bad", "failed", { error: "新的坏清单", manifestError: "新的坏清单" }),
      item("user:added", "not_loaded"),
    ],
  };
  const merged = keepUserScriptRuntime(previous, next);
  const byKey = Object.fromEntries(merged.scripts.map((script) => [script.key, script]));
  // 同一个 key、启停没变、新状态是兜底值：沿用 status / error，其余字段用新的。
  assert.deepEqual([byKey["user:loaded"].status, byKey["user:loaded"].error, byKey["user:loaded"].name], ["loaded", "", "改过名"]);
  assert.deepEqual([byKey["user:crashed"].status, byKey["user:crashed"].error], ["failed", "TypeError: boom"]);
  assert.equal(byKey["user:pending"].status, "unknown");
  // 启停变了：以新清单为准。
  assert.equal(byKey["user:off"].status, "not_loaded");
  assert.equal(byKey["user:toggled-off"].status, "disabled");
  // 上一份是清单错误（现在修好了）：不把旧的 failed 带过来。
  assert.deepEqual([byKey["user:was-bad"].status, byKey["user:was-bad"].error], ["not_loaded", ""]);
  // 新清单里是清单错误：以新清单为准。
  assert.deepEqual([byKey["user:now-bad"].status, byKey["user:now-bad"].error], ["failed", "新的坏清单"]);
  // 新增的用新清单的，删除的不留。
  assert.equal(byKey["user:added"].status, "not_loaded");
  assert.equal("user:removed" in byKey, false);
  assert.deepEqual(merged.scripts.map((script) => script.key), next.scripts.map((script) => script.key), "顺序与新清单一致");
  assert.equal(merged.skip_next_start, true, "清单其余字段用新的");
  assert.equal(next.scripts[0].status, "not_loaded", "不改传进来的清单");
});

test("沿用运行状态的边界：总开关、没有上一份、没有可沿用的", () => {
  const item = (key: string, status: string, enabled = true) => ({ key, enabled, status, error: "", manifestError: "" });
  const loaded = { enabled: true, scripts: [item("user:a", "loaded"), item("user:b", "loaded")] };
  // 总开关关掉：新清单里全是 disabled，照新清单显示。
  const off = { enabled: false, scripts: [item("user:a", "disabled"), item("user:b", "disabled")] };
  assert.equal(keepUserScriptRuntime(loaded, off), off);
  // 总开关再打开：上一份的 disabled 不是运行状态，不带过来（各拓展自己的 enabled 没变过）。
  const on = { enabled: true, scripts: [item("user:a", "not_loaded"), item("user:b", "not_loaded")] };
  assert.equal(keepUserScriptRuntime(off, on), on);
  // 上一份也是「未加载」（Codex 没开时刷新得到的）：没有要沿用的，原样返回同一个对象。
  assert.equal(keepUserScriptRuntime(on, on), on);
  const again = { enabled: true, scripts: [item("user:a", "not_loaded"), item("user:b", "not_loaded")] };
  assert.equal(keepUserScriptRuntime(on, again), again);
  // 没有上一份、清单为空或缺失时原样返回。
  assert.equal(keepUserScriptRuntime(null, on), on);
  assert.equal(keepUserScriptRuntime(undefined, on), on);
  assert.equal(keepUserScriptRuntime({ enabled: true }, on), on);
  assert.equal(keepUserScriptRuntime(loaded, undefined), undefined);
  const empty = { enabled: true, scripts: [] };
  assert.equal(keepUserScriptRuntime(loaded, empty), empty);
  // 旧版后端没有 manifestError：上一份的 failed 分不清是清单错还是运行错，按清单错处理（以新清单为准）；loaded 照常沿用。
  const legacy = { enabled: true, scripts: [{ key: "user:a", enabled: true, status: "failed", error: "说不清" }, { key: "user:b", enabled: true, status: "loaded", error: "" }] };
  const legacyNext = { enabled: true, scripts: [{ key: "user:a", enabled: true, status: "not_loaded", error: "" }, { key: "user:b", enabled: true, status: "not_loaded", error: "" }] };
  assert.deepEqual(keepUserScriptRuntime(legacy, legacyNext).scripts.map((script) => script.status), ["not_loaded", "loaded"]);
});

test("能顶替的页面都是管理器里真有的路由，且不含停用拓展的入口", () => {
  const routeIds = new Set([...app.matchAll(/\{ id: "([A-Za-z]+)",/g)].map((match) => match[1]));
  const replaceable = rustList("MANAGER_REPLACEABLE_PAGES");
  // 旧 id（relayEnvironment、recommendations…）已并进别的页面，留在清单里只是兼容，不会再被顶替。
  for (const id of replaceable) assert.ok(routeIds.has(id) || isRouteAlias(id), `${id} 不是管理器路由`);
  for (const id of ["userScripts", "enhance", "settings", "extension"]) assert.ok(!replaceable.includes(id), `${id} 不能被顶替`);
});

test("拓展页面能读的数据与 Rust 侧清单一致", () => {
  assert.deepEqual(Object.keys(MANAGER_DATA_SOURCES).sort(), rustList("MANAGER_DATA_SOURCES").sort());
  assert.deepEqual(MANAGER_DATA_SOURCES.gatewayUsage.args({}), { range: "today" });
  assert.deepEqual(MANAGER_DATA_SOURCES.localSessions.args({ limit: 5 }), { request: { offset: 0, limit: 5 } });
});

test("公共数据只走专用只读命令，身份取宿主且不转发任意参数", () => {
  const names = ["gatewayProviders", "gatewayRouting", "gatewayHealth", "gatewayQuota", "installedSkills", "installedPlugins", "agentCollaboration"];
  for (const name of names) {
    const request = managerDataRequest(name, { key: "user:other", name: "settings", command: "delete", path: "/private", refresh: true, limit: 9999 }, names, "user:panel");
    assert.equal(request.command, "extension_manager_data");
    assert.deepEqual(request.args, { key: "user:panel", name, params: name === "agentCollaboration" ? { limit: 200 } : {} });
  }
  assert.throws(() => managerDataRequest("gatewayProviders", {}, [], "user:panel"), /没有声明/);
  assert.throws(() => managerDataRequest("settings", {}, ["settings"], "user:panel"), /没有声明/);
  assert.throws(() => managerDataRequest("toString", {}, ["toString"], "user:panel"), /没有声明/);
  assert.deepEqual(MANAGER_DATA_SOURCES.agentCollaboration.args({ limit: -10 }), { name: "agentCollaboration", params: { limit: 1 } });
  assert.deepEqual(MANAGER_DATA_SOURCES.agentCollaboration.args({ limit: 3.7 }), { name: "agentCollaboration", params: { limit: 3 } });
  assert.deepEqual(MANAGER_DATA_SOURCES.agentCollaboration.args({ limit: Infinity }), { name: "agentCollaboration", params: { limit: 50 } });
  // 旧四类接口维持既有返回与参数契约。
  assert.deepEqual(managerDataRequest("gatewayUsage", {}, ["gatewayUsage"], "user:panel"), { command: "load_gateway_usage", args: { range: "today" } });
});

test("公开数据命令已注册，不调用刷新、安装、协作写快照或序列化完整网关", () => {
  const commands = readFileSync(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(lib, /commands::extension_manager_data,/);
  const body = commands.slice(commands.indexOf("pub async fn extension_manager_data("), commands.indexOf("pub async fn extension_manager_backend_call("));
  assert.match(body, /authorize_manager_data/);
  assert.match(body, /extension_summary/);
  assert.match(body, /installed_summary/);
  assert.doesNotMatch(body, /fetch_balances|refresh_quota|load_gateway\(|agent_collaboration_request|\.snapshot\(\)|set_settings|save_config|to_value\(&gateway\)/);
});

test("面板桥接的 data / navigate：没给能力或页面不认得时回错误", async () => {
  const base: ManagerBridgeDeps = {
    config: () => ({}), saveConfig: async () => true, backendCall: async () => null, asset: async () => "",
    toast: () => {}, resize: () => {}, theme: () => "dark",
  };
  const ask = (deps: ManagerBridgeDeps, method: string, params: unknown) => handleManagerRequest({ __codexPlusManager: 1, id: 1, method, params }, deps);
  assert.equal((await ask(base, "data", { name: "requestLogs" })).ok, false);
  const seen: unknown[] = [];
  const deps = { ...base, data: async (name: string, params: Record<string, unknown>) => { seen.push([name, params]); return { entries: [] }; }, navigate: (target: string) => target === "hub" };
  assert.deepEqual(await ask(deps, "data", { name: "requestLogs", params: { limit: 3 } }), { __codexPlusManager: 1, id: 1, ok: true, value: { entries: [] } });
  assert.deepEqual(seen, [["requestLogs", { limit: 3 }]]);
  assert.equal((await ask(deps, "navigate", { target: "hub" })).ok, true);
  assert.equal((await ask(deps, "navigate", { target: "nope" })).ok, false);
  // 页面里的 window.codexPlusManager 也有这两个方法。
  const posted: unknown[] = [];
  const sandbox: Record<string, unknown> = { parent: { postMessage: (message: unknown) => posted.push(message) }, addEventListener: () => {}, document: { documentElement: { dataset: {} } } };
  sandbox.window = sandbox;
  vm.runInNewContext(MANAGER_PANEL_BOOTSTRAP, sandbox);
  const api = sandbox.codexPlusManager as { data: (name: string, params: unknown) => Promise<unknown>; navigate: (target: string) => Promise<unknown> };
  void api.data("requestLogs", { limit: 1 });
  void api.navigate("hub");
  assert.deepEqual(posted.map((message) => (message as { method: string }).method), ["data", "navigate"]);
});

test("App 把拓展页面接进侧栏、命令面板与页面渲染", () => {
  // 侧栏：本地服务不再占一组，声明在 gateway 分组的拓展页面和其余两组合成一节「拓展页面」（分组名照旧认）。
  assert.match(app, /\(\["gateway", "management", "tools"\] as ManagerNavGroup\[\]\)\.flatMap\(\(group\) => managerNavPages\(extensionPages, group\)\)/);
  assert.match(app, /\{extensionPageView \?\? \(<>/);
  assert.match(app, /查看内置页面/);
});
