import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";

// 原生浏览器状态只在 Codex 的 Codex++ 设置页里显示（46-service-settings.js），文案也只有那一份。
const codexSettingsSource = () => readFile(new URL("../../../assets/inject/renderer-inject/46-service-settings.js", import.meta.url), "utf8");
const codexCatalogPage = () => readFile(new URL("../../../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");

type BrowserConnection = {
  state: string;
  browsers: { family: string; headerEnabled: boolean | null; recognized?: boolean }[];
  failedChecks: number;
};
type Labels = {
  codexPlusBrowserConnectionLabel: (connection: BrowserConnection) => string;
  codexPlusBrowserHeaderLabel: (enabled: boolean | null) => string;
  codexPlusNativeBrowserPatchLabel: (state: string) => string;
};

/** 在沙箱里跑一遍分片，取出三个文案函数（它们不碰 DOM，只要分片加载时用到的全局在就行）。 */
async function shardLabels(): Promise<Labels> {
  const context = vm.createContext({
    window: {},
    console,
    URL,
    CSS: { escape: (value: unknown) => String(value) },
    document: { activeElement: null, querySelectorAll: () => [], querySelector: () => null },
    setTimeout: () => 0,
    clearTimeout: () => {},
    escapeHtml: (value: unknown) => String(value ?? ""),
    codexPlusBackendSettings: {},
    codexPlusBackendSettingsSeq: 0,
    codexPlusIsWindowsPlatform: true,
    codexPlusIsMacPlatform: false,
  });
  vm.runInContext(await codexSettingsSource(), context);
  return context as unknown as Labels;
}

test("adapter refusal is distinct from browser availability and header state", async () => {
  const { codexPlusNativeBrowserPatchLabel: patch } = await shardLabels();
  assert.match(patch("runtime_unverified"), /未应用旧版兼容补丁/);
  assert.doesNotMatch(patch("blocked"), /未启用兼容/);
  assert.match(patch("blocked"), /文件/);
});

test("a connected Edge or Chrome is explicitly available regardless of patch or header state", async () => {
  const { codexPlusBrowserConnectionLabel: connection } = await shardLabels();
  for (const headerEnabled of [true, false, null]) {
    assert.equal(connection({ state: "available", failedChecks: 0, browsers: [{ family: "edge", headerEnabled }] }), "Edge 浏览器可用");
  }
  assert.equal(connection({ state: "available", failedChecks: 1,
    browsers: [{ family: "chrome", headerEnabled: false }, { family: "edge", headerEnabled: true }] }),
  "Chrome / Edge 浏览器可用");
  assert.equal(connection({ state: "available", failedChecks: 0, browsers: [] }), "浏览器状态检测失败");
});

test("disconnect, check failure, and header setting have distinct labels", async () => {
  const { codexPlusBrowserConnectionLabel: connection, codexPlusBrowserHeaderLabel: header } = await shardLabels();
  assert.equal(connection({ state: "disconnected", failedChecks: 0, browsers: [] }), "浏览器未连接");
  assert.equal(connection({ state: "check_failed", failedChecks: 1, browsers: [] }), "浏览器状态检测失败");
  assert.match(header(true), /已开启.*扩展报告/);
  assert.match(header(false), /已关闭.*扩展报告/);
  assert.match(header(null), /未提供/);
});

test("unrecognized extension is distinct from a disconnected one", async () => {
  // issue #2209：同 family 但 ID 未登记 = 连上了但版本不认识，不该显示成「没连上」。
  const { codexPlusBrowserConnectionLabel: connection } = await shardLabels();
  assert.equal(connection({ state: "available", failedChecks: 0,
    browsers: [{ family: "edge", headerEnabled: true, recognized: false }] }), "Edge 浏览器可用");
  // 后缀只在明确未登记（false）时出现；已登记与旧后端不带该字段（undefined）都不加。
  const page = await codexSettingsSource();
  assert.ok(page.includes('browser.recognized === false ? "（扩展版本未登记）" : ""'));
});

test("patch diagnostics are separate from live availability and refresh only on demand", async () => {
  const page = await codexSettingsSource();
  assert.ok(page.includes("codexPlusBrowserConnectionLabel(connection)"));
  assert.ok(page.includes("codexPlusNativeBrowserPatchLabel(compatibility.state)"));
  assert.ok(page.includes('postJson("/native-browser/status", {})'));
  // 只在 Windows 上、第一次渲染时检测一次，之后靠「刷新」按钮；检测中不重复发请求。
  assert.ok(page.includes('groupId === "plugins" && codexPlusIsWindowsPlatform'));
  assert.ok(page.includes("if (state.nativeBrowserBusy) return;"));
  assert.ok(!page.includes("setInterval("));
});

test("patch diagnostics describe only patch state, not browser availability", async () => {
  const { codexPlusNativeBrowserPatchLabel: patch } = await shardLabels();
  assert.match(patch("prepared"), /兼容服务文件/);
  assert.match(patch("restored"), /扩展保留/);
  assert.match(patch("stale"), /过期/);
  assert.equal(patch("new-state"), "无法读取兼容补丁状态");
  for (const state of ["prepared", "blocked", "runtime_unverified", "new-state"]) {
    assert.doesNotMatch(patch(state), /浏览器.*可用/);
  }
});

test("consent precedes enabling in Codex, default is off, and status is read-only", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const page = await codexCatalogPage();
  assert.match(app, /codexAppNativeBrowserRequireIdentification: false/);
  // 开启前在 Codex 设置页里确认，取消就不写。
  assert.match(page, /if \(next && key === "codexAppNativeBrowserRequireIdentification"\) \{[\s\S]*?codexPlusAsk\(/);
  assert.match(page, /x-browser-agent/);
  assert.match(page, /不会关闭扩展已保存的标识设置/);
  assert.match(page, /Edge \/ Chrome 稳定版扩展/);
  // 开关文案来自设置项清单，只在 Windows 的 Codex 设置页里显示。
  const catalog = JSON.parse(await readFile(new URL("../../../crates/codex-plus-core/assets/enhancement-settings.json", import.meta.url), "utf8"));
  const item = catalog.items.find((candidate: { key: string }) => candidate.key === "codexAppNativeBrowserRequireIdentification");
  assert.match(item.title, /原生 Edge \/ Chrome 请求标识兼容（实验）/);
  assert.match(item.description, /此兼容补丁仅适配 Windows 上的 Edge \/ Chrome/);
  assert.deepEqual(item.platforms, ["windows"]);
  assert.deepEqual(item.surfaces, ["codex"]);
  assert.doesNotMatch(app, /NativeBrowserStatusView|native-browser-settings/);
  const { codexPlusNativeBrowserPatchLabel: patch } = await shardLabels();
  assert.match(patch("unsupported"), /Edge \/ Chrome/);
  // 状态那一节只读：不写设置、不重启。
  const status = await codexSettingsSource();
  const section = status.slice(status.indexOf("function renderCodexPlusNativeBrowserStatusSection"), status.indexOf("/** 清单分组后面要接的服务配置节"));
  assert.ok(section.length > 100);
  assert.doesNotMatch(section, /\/settings\/set|restart/);
});
