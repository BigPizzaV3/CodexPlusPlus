import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import { MANAGER_DATA_SOURCES, managerDataRequest, managerExtensionPages, managerNavPages, managerPageOverrides } from "./extension-manager-pages.ts";
import { handleManagerRequest, MANAGER_PANEL_BOOTSTRAP, type ManagerBridgeDeps } from "./extension-manager-bridge.ts";

const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
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

test("能顶替的页面都是管理器里真有的路由，且不含停用拓展的入口", () => {
  const routeIds = new Set([...app.matchAll(/\{ id: "([A-Za-z]+)",/g)].map((match) => match[1]));
  const replaceable = rustList("MANAGER_REPLACEABLE_PAGES");
  for (const id of replaceable) assert.ok(routeIds.has(id), `${id} 不是管理器路由`);
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
  // 侧栏：网关分组的拓展页面跟在聚合网关后面，其余两组合成一节「拓展页面」。
  assert.match(app, /const gatewayExtensionItems = managerNavPages\(extensionPages, "gateway"\);/);
  assert.match(app, /\(\["management", "tools"\] as ManagerNavGroup\[\]\)\.flatMap\(\(group\) => managerNavPages\(extensionPages, group\)\)/);
  assert.match(app, /\{extensionPageView \?\? \(<>/);
  assert.match(app, /查看内置页面/);
});
