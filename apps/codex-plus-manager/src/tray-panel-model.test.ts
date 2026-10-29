import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { GatewayPayload, GatewayPoolEntry, UsageSummary } from "./gateway-model.ts";
import {
  activeRelayItem,
  codexEnhanced,
  codexState,
  gatewayConnected,
  gatewayIntroNeeded,
  parseTrayRequest,
  readStoredTrayTab,
  restoreTrayTab,
  storeTrayTab,
  trayQuotaRows,
  trayRelayItems,
  usageGlance,
  TRAY_TAB_STORAGE_KEY,
  TRAY_TABS,
} from "./tray-panel-model.ts";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");

function memoryStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => void data.set(key, value),
  };
}

const throwingStorage = {
  getItem: (): string | null => {
    throw new Error("blocked");
  },
  setItem: () => {
    throw new Error("blocked");
  },
};

test("托盘面板三个页签，网关只是其中之一且排在最后；默认是概览；没有旧版的供应商切换", () => {
  assert.deepEqual([...TRAY_TABS], ["home", "usage", "gateway"]);
  assert.equal(restoreTrayTab("providers"), "home", "上次停在已去掉的「供应商」页签时回到概览");
  assert.equal(restoreTrayTab(null), "home");
  assert.equal(restoreTrayTab("gateway"), "gateway");
  assert.equal(restoreTrayTab("routing"), "home", "不认识的旧值回到概览");
});

test("记住上次的页签：读写 localStorage，存储不可用时不抛错", () => {
  const storage = memoryStorage();
  storeTrayTab("usage", storage);
  assert.equal(storage.data.get(TRAY_TAB_STORAGE_KEY), "usage");
  assert.equal(readStoredTrayTab(storage), "usage");
  assert.doesNotThrow(() => storeTrayTab("gateway", throwingStorage));
  assert.equal(readStoredTrayTab(throwingStorage), "home");
  assert.equal(readStoredTrayTab(null), "home");
});

test("Codex 状态：未找到 / 运行中 / 未运行 / 检查中，增强功能看调试连接", () => {
  assert.equal(codexState(null), "unknown");
  assert.equal(codexState({ codex_app: { status: "missing", path: null } }), "missing");
  assert.equal(codexState({ codex_app: { status: "found", path: "/A" }, runtime_health: { codex_app: { status: "running" } } }), "running");
  assert.equal(codexState({ codex_app: { status: "found", path: "/A" }, runtime_health: { codex_app: { status: "stopped" } } }), "stopped");
  assert.equal(codexState({ codex_app: { status: "found", path: "/A" }, runtime_health: { codex_app: { status: "not_checked" } } }), "unknown");
  assert.equal(codexEnhanced({ runtime_health: { codex_app: { status: "running" }, debugger: { status: "connected" } } }), true);
  assert.equal(codexEnhanced({ runtime_health: { codex_app: { status: "stopped" }, debugger: { status: "connected" } } }), false);
});

test("供应商列表：当前的排最前，模型去掉窗口后缀，区分网关 / 聚合 / 协议转换 / 官方", () => {
  const settings = {
    relayProfilesEnabled: true,
    activeRelayId: "b",
    relayProfiles: [
      { id: "a", name: "DeepSeek", model: "deepseek-v4-pro[1M]", baseUrl: "https://api.deepseek.com/v1", protocol: "responses", relayMode: "pureApi" },
      { id: "b", name: " Kimi ", model: "kimi-k2", baseUrl: "https://api.moonshot.cn/v1", protocol: "chatCompletions" },
      { id: "codex-plus-gateway", name: "Codex++ 网关", model: "", baseUrl: "http://127.0.0.1:57321/codex" },
      { id: "c", name: "备用", relayMode: "aggregate" },
      { id: "d", name: "ChatGPT", relayMode: "official" },
    ],
  };
  const items = trayRelayItems(settings);
  assert.deepEqual(items.map((item) => item.id), ["b", "a", "codex-plus-gateway", "c", "d"]);
  assert.deepEqual(items.map((item) => item.kind), ["conversion", "direct", "gateway", "aggregate", "official"]);
  assert.equal(items[0].name, "Kimi");
  assert.equal(items[1].detail, "deepseek-v4-pro · api.deepseek.com");
  assert.equal(activeRelayItem(settings)?.id, "b");
  assert.deepEqual(trayRelayItems(null), []);
  assert.equal(activeRelayItem({ activeRelayId: "x", relayProfiles: [] }), null);
});

function gateway(pool: Partial<GatewayPoolEntry>[], enabled: boolean): GatewayPayload {
  return {
    gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
    port: 57321,
    helperRunning: enabled,
    pool: pool as GatewayPoolEntry[],
    groups: [],
    agents: [{ agent: "codex", name: "Codex", enabled, baseUrl: "", defaultModel: "", models: [] }],
    health: { keys: [], providers: [] },
    codexProviders: [],
  } as unknown as GatewayPayload;
}

test("网关页签：没有供应商也没有 Agent 接入时给说明，否则显示各 Agent", () => {
  assert.equal(gatewayIntroNeeded(null), true);
  assert.equal(gatewayIntroNeeded(gateway([], false)), true);
  assert.equal(gatewayIntroNeeded(gateway([{ id: "p" }], false)), false);
  assert.equal(gatewayIntroNeeded(gateway([], true)), false);
  assert.equal(gatewayConnected(gateway([], true)), true);
  assert.equal(gatewayConnected(null), false);
});

test("用量速览：成功率、token、按 token 排前几个模型", () => {
  const counts = (calls: number, errors: number, input: number, output: number) => ({ calls, errors, inputTokens: input, outputTokens: output, cachedTokens: 0, reasoningTokens: 0 });
  const summary: UsageSummary = {
    totals: counts(10, 1, 900, 100),
    days: [],
    byAgent: [],
    byProvider: [],
    byModel: [
      { key: "small", label: "small", ...counts(2, 0, 90, 10) },
      { key: "big", label: "big", ...counts(8, 1, 810, 90) },
      { key: "idle", label: "idle", ...counts(0, 0, 0, 0) },
    ],
    costs: [],
  };
  const glance = usageGlance(summary)!;
  assert.equal(glance.calls, 10);
  assert.equal(glance.successRate, 0.9);
  assert.deepEqual(glance.topModels.map((model) => model.key), ["big", "small"]);
  assert.equal(glance.topModels[0].share, 0.9);
  assert.equal(usageGlance(null), null);
  assert.equal(usageGlance({ ...summary, totals: counts(0, 0, 0, 0), byModel: [] })!.successRate, null);
});

test("订阅额度：只列有额度的启用供应商，过了重置时间按满额", () => {
  const now = 1_000_000;
  const rows = trayQuotaRows([
    { id: "a", name: "ChatGPT Pro · me", plan: "Pro", enabled: true, quota: { updatedAtMs: 0, windows: [
      { windowMinutes: 300, usedPercent: 40, resetsAtMs: now + 1000 },
      { windowMinutes: 10080, usedPercent: 90, resetsAtMs: now - 1 },
    ] } },
    { id: "b", name: "关掉的", enabled: false, quota: { updatedAtMs: 0, windows: [{ windowMinutes: 300, usedPercent: 10, resetsAtMs: null }] } },
    { id: "c", name: "普通供应商" },
  ] as GatewayPoolEntry[], now);
  assert.deepEqual(rows.map((row) => row.providerId), ["a"]);
  assert.deepEqual(rows[0].windows.map((window) => window.remaining), [0.6, 1]);
  assert.deepEqual(trayQuotaRows(undefined, now), []);
});

test("主窗口只接受格式正确的面板请求", () => {
  assert.deepEqual(parseTrayRequest({ id: "1", kind: "navigate", route: "gatewayRouting" }), { id: "1", kind: "navigate", route: "gatewayRouting" });
  assert.equal(parseTrayRequest({ id: "1", kind: "navigate", route: "relay" }), null, "旧版供应商页不从面板打开");
  assert.equal(parseTrayRequest({ id: "1", kind: "navigate", route: "dreamSkin" }), null, "不在白名单里的页面不打开");
  assert.equal(parseTrayRequest({ id: "2", kind: "switchRelay", profileId: "a" }), null, "面板不再切换直连供应商");
  assert.deepEqual(parseTrayRequest({ id: "3", kind: "restart" }), { id: "3", kind: "restart" });
  assert.equal(parseTrayRequest({ kind: "launch" }), null);
  assert.equal(parseTrayRequest({ id: "4", kind: "deleteEverything" }), null);
  assert.equal(parseTrayRequest(null), null);
});

test("托盘面板改成多页签：网关只是一个页签，记住页签，改配置的操作交给主窗口", () => {
  const panel = read("./TrayPanel.tsx");
  assert.match(panel, /role="tablist"/);
  assert.match(panel, /TRAY_TABS\.map/);
  assert.match(panel, /storeTrayTab\(next\)/);
  assert.match(panel, /readStoredTrayTab\(\)/);
  assert.match(panel, /emit\(TRAY_REQUEST_EVENT/);
  // 面板自己不调切换供应商 / 启动 Codex 的命令，免得绕过主窗口的校验与冲突弹窗。
  assert.doesNotMatch(panel, /"switch_relay_profile"|"launch_codex_plus"|"restart_codex_plus"|"stop_codex_plus"/);
  const app = read("./App.tsx");
  assert.match(app, /listen<unknown>\(TRAY_REQUEST_EVENT/);
  assert.match(app, /emit\(TRAY_RESULT_EVENT, result\)/);
  assert.doesNotMatch(panel, /ProvidersTab|switchRelay/);
});

test("「点击托盘图标弹出快捷面板」从网关设置移到整体设置，仍存 gateway.json 的 trayPanel", () => {
  const gatewayPage = read("./GatewayPage.tsx");
  assert.doesNotMatch(gatewayPage, /点击托盘图标弹出快捷面板|save_gateway_tray_panel/);
  const app = read("./App.tsx");
  const start = app.indexOf("function GlobalSettingsScreen(");
  const globalSettings = app.slice(start, app.indexOf("\nfunction ", start + 10));
  assert.match(globalSettings, /点击托盘图标弹出快捷面板/);
  assert.match(app, /invoke<boolean>\("set_tray_panel_enabled"/);
  const lib = read("../src-tauri/src/lib.rs");
  assert.match(lib, /tray_panel::tray_panel_enabled,/);
  assert.match(lib, /tray_panel::set_tray_panel_enabled,/);
  assert.doesNotMatch(lib, /save_gateway_tray_panel/);
  const rust = read("../src-tauri/src/tray_panel.rs");
  assert.match(rust, /settings\.tray_panel = enabled/);
});

test("轻量模式（#2677）：面板发请求前先让后端按需重建主窗口，主窗口监听就绪后通知后端，开关在整体设置里", () => {
  const panel = read("./TrayPanel.tsx");
  const request = panel.slice(panel.indexOf("const request = async"), panel.indexOf("const action = "));
  // 两处发请求（启动类操作、打开某页）都先等主窗口就绪再发。
  assert.equal(request.match(/await prepareMainWindow\(\);\r?\n\s+await emit\(TRAY_REQUEST_EVENT/g)?.length, 2);
  assert.match(panel, /invoke<boolean>\("main_window_prepare"\)/);
  const app = read("./App.tsx");
  assert.match(app, /stopListening = unlisten;\r?\n\s+\/\/.*\r?\n\s+void emit\(MAIN_WINDOW_READY_EVENT\)/);
  // 面板请求没做完时不释放主窗口。
  assert.match(app, /if \(trayRequestsInFlight\.current === 0\) void invoke\("main_window_release_when_hidden"\)/);
  const start = app.indexOf("function GlobalSettingsScreen(");
  const globalSettings = app.slice(start, app.indexOf("\nfunction ", start + 10));
  assert.match(globalSettings, /关闭窗口后释放界面内存/);
  assert.match(app, /invoke<boolean>\("set_lightweight_mode_enabled"/);
  const lib = read("../src-tauri/src/lib.rs");
  assert.match(lib, /lightweight::main_window_prepare,/);
  assert.match(lib, /lightweight::set_lightweight_mode_enabled,/);
  const rust = read("../src-tauri/src/lightweight.rs");
  assert.match(rust, /settings\.lightweight_on_close = enabled/);
  assert.match(rust, /pub const MAIN_WINDOW_READY_EVENT: &str = "codex-plus-main-window-ready";/);
  assert.match(read("./tray-panel-model.ts"), /export const MAIN_WINDOW_READY_EVENT = "codex-plus-main-window-ready";/);
});

test("托盘事件先按类型筛掉鼠标移动，开关读内存缓存而不是每次读 gateway.json", () => {
  const rust = read("../src-tauri/src/tray_panel.rs");
  const handler = rust.slice(rust.indexOf("pub fn handle_tray_event"), rust.indexOf("pub fn hide"));
  assert.ok(handler.indexOf("panel_event(event)") < handler.indexOf("enabled()"), "先筛事件再看开关");
  const enabled = rust.slice(rust.indexOf("pub fn enabled()"), rust.indexOf("fn set_cached"));
  assert.doesNotMatch(enabled, /GatewayStore/);
  assert.match(rust, /const WIDTH: f64 = 380\.0;/);
  assert.match(rust, /const HEIGHT: f64 = 540\.0;/);
});
