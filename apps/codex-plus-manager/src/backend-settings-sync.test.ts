import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// Codex 页面里设置开关与后端的同步：原先借会话分享开关测，分享下线后改用 Markdown 导出开关。
const fragment = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");

test("the in-app toggle persists a backend update rather than a temporary local preference", async () => {
  const style = fragment("10-style.js");
  const writes: unknown[] = [];
  const context = vm.createContext({ window: {}, codexPlusBackendSettings: {}, conversationViewDefaultWidth: 900,
    codexPlusSettingsKey: "settings", localStorage: { getItem: () => "{}", setItem: () => { throw new Error("must use backend"); } },
    setBackendSetting: async (key: string, value: unknown) => { writes.push([key, value]); },
    loadBackendSettings: async () => {}, syncStepwisePanel: () => {},
  });
  const start = style.indexOf("  function defaultCodexPlusSettings()");
  const end = style.indexOf("  // Dream skin runtime", start);
  const setterStart = style.indexOf("  function setCodexPlusSetting(");
  const setterEnd = style.indexOf("  function syncStepwisePanel(", setterStart);
  vm.runInContext(`${style.slice(start, end)}\n${style.slice(setterStart, setterEnd)}`, context);
  context.setCodexPlusSetting("markdownExport", false); await Promise.resolve();
  assert.deepEqual(writes, [["codexAppMarkdownExport", false]]);
  // 设置页的开关按清单生成：清单字段映射到本页的 key，由 setCodexPlusSetting 写后端。
  const catalog = JSON.parse(readFileSync(new URL("../../../crates/codex-plus-core/assets/enhancement-settings.json", import.meta.url), "utf8"));
  const item = catalog.items.find((entry: { key: string }) => entry.key === "codexAppMarkdownExport");
  assert.ok(item?.surfaces.includes("codex"));
  assert.match(style, /markdownExport: "codexAppMarkdownExport"/);
});

test("backend refresh notifies builtins and skips failed heartbeat reads", async () => {
  const source = fragment("40-backend-settings.js");
  let loaded = true, toggleSyncs = 0;
  let settingsEvents = 0;
  const context = vm.createContext({ codexPlusSettings: () => ({ conversationView: false }),
    loadBackendSettingsState: async () => loaded, syncOfficialUsagePolicy: () => {},
    emitCodexPlusSettingsLoaded: () => { settingsEvents += 1; },
    renderCodexPlusMenu: () => {}, runScanStep: (fn: () => void) => fn(),
    syncCodexPlusBackendToggleButtons: () => { toggleSyncs += 1; },
  });
  const start = source.indexOf("  let syncBackendSettingsInFlight");
  const end = source.indexOf("  async function setBackendSetting(", start);
  vm.runInContext(source.slice(start, end), context);
  // 心跳读到设置后同步设置页开关（管理器里改的值实时反映到 Codex）；读取失败不动。
  await context.syncBackendSettingsFromHeartbeat(); assert.equal(toggleSyncs, 1);
  assert.equal(settingsEvents, 1);
  loaded = false; await context.syncBackendSettingsFromHeartbeat(); assert.equal(toggleSyncs, 1);
  assert.equal(settingsEvents, 1, "failed reads do not notify builtins (Weixin sync, typing effects…)");
});

test("optimistic backend writes apply to the page before their response arrives", async () => {
  const source = fragment("40-backend-settings.js");
  const applied: unknown[] = [];
  let finish: (value: unknown) => void = () => {};
  const context = vm.createContext({ codexPlusBackendSettingsSeq: 0, codexPlusBackendSettings: {},
    postJson: () => new Promise((resolve) => { finish = resolve; }),
    refreshCodexPlusBackendToggles: () => { applied.push(context.codexPlusBackendSettings.codexAppMarkdownExport); },
  });
  const start = source.indexOf("  async function setBackendSetting(");
  const end = source.indexOf("  function refreshCodexPlusBackendToggles(", start);
  vm.runInContext(source.slice(start, end), context);
  const saved = context.setBackendSetting("codexAppMarkdownExport", false);
  assert.deepEqual(applied, [false], "开关在响应回来之前已生效");
  finish({ codexAppMarkdownExport: false }); await saved;
  assert.equal(context.codexPlusBackendSettings.codexAppMarkdownExport, false);
});

test("heartbeat reports changed keys and relay events for settings changed outside the page", async () => {
  const source = fragment("40-backend-settings.js");
  const events: Array<[string, any]> = [];
  // 事件参数是沙箱里建的对象，原型不同，转成普通 JSON 再比。
  const plain = () => JSON.parse(JSON.stringify(events));
  const profile = (id: string) => ({ id, name: id.toUpperCase(), model: "m", contextWindow: "", autoCompactLimit: "" });
  let next: Record<string, unknown> = {};
  const context: any = vm.createContext({
    codexPlusSettings: () => ({ conversationView: false }),
    syncOfficialUsagePolicy: () => {}, renderCodexPlusMenu: () => {}, syncCodexPlusBackendToggleButtons: () => {},
    codexPlusEmitExtensionEvent: (event: string, [detail]: unknown[]) => events.push([event, detail]),
    codexPlusBackendSettingsLoaded: false,
    codexPlusBackendSettings: {},
  });
  context.loadBackendSettingsState = async () => {
    context.codexPlusBackendSettings = { ...context.codexPlusBackendSettings, ...next };
    context.codexPlusBackendSettingsLoaded = true;
    return true;
  };
  vm.runInContext(source.slice(source.indexOf("  function emitCodexPlusSettingsLoaded("), source.indexOf("  /**\n   * 在 Codex 页面里改了一项设置后")), context);
  vm.runInContext(source.slice(source.indexOf("  let syncBackendSettingsInFlight"), source.indexOf("  async function setBackendSetting(")), context);

  // 第一次读到设置：不带 changedKeys，也不发供应商事件。
  next = { relayProfilesEnabled: true, activeRelayId: "a", activeRelayProfilePublic: profile("a"), modelCatalogFingerprint: "f1" };
  await context.syncBackendSettingsFromHeartbeat();
  assert.deepEqual(plain(), [["settings/loaded", { reason: "heartbeat" }]]);

  // 没变：changedKeys 为空数组。
  events.length = 0;
  await context.syncBackendSettingsFromHeartbeat();
  assert.deepEqual(plain(), [["settings/loaded", { reason: "heartbeat", changedKeys: [] }]]);

  // 管理器里换了供应商：只报字段名，再发 relayProfile/switched 与 modelCatalog/changed。
  events.length = 0;
  next = { activeRelayId: "b", activeRelayProfilePublic: profile("b"), modelCatalogFingerprint: "f2" };
  await context.syncBackendSettingsFromHeartbeat();
  assert.deepEqual(events.map(([event]) => event), ["settings/loaded", "relayProfile/switched", "modelCatalog/changed"]);
  assert.deepEqual(plain()[0][1].changedKeys.sort(), ["activeRelayId", "activeRelayProfilePublic", "modelCatalogFingerprint"]);
  assert.equal(events[1][1].previous.id, "a");
  assert.equal(events[1][1].profile.id, "b");
  assert.equal(events[2][1].profile.id, "b");

  // 同一供应商只改了模型列表：只发 modelCatalog/changed。
  events.length = 0;
  next = { modelCatalogFingerprint: "f3" };
  await context.syncBackendSettingsFromHeartbeat();
  assert.deepEqual(events.map(([event]) => event), ["settings/loaded", "modelCatalog/changed"]);
});
