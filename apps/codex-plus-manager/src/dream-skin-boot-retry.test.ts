import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// #2680：重启 Codex 后首次后端读取超时、页面被启动壳自救重载时，皮肤不能因为读不到设置而不生效。
const fragment = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");

function slice(source: string, startMarker: string, endMarker: string) {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start);
  assert.ok(start >= 0 && end > start, `missing ${startMarker}`);
  return source.slice(start, end);
}

function refreshHarness(options: { external: boolean; loaded: boolean; enabled: boolean; paused?: boolean; installed: boolean }) {
  const calls: string[] = [];
  const window: Record<string, unknown> = {
    __CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__: options.external,
    __CODEX_DREAM_SKIN_STATE__: options.installed ? { ensure: () => calls.push("ensure") } : undefined,
  };
  const context = vm.createContext({
    window,
    codexPlusBackendSettingsLoaded: options.loaded,
    codexPlusSettings: () => ({ dreamSkinEnabled: options.enabled, dreamSkinPaused: !!options.paused, dreamSkinThemeConfig: {} }),
    ensureDreamSkinMainSurface: () => calls.push("main-surface"),
    ensureDreamSkinCompanion: () => calls.push("companion"),
    cleanupDreamSkin: () => calls.push("cleanup"),
    installDreamSkin: () => calls.push("install"),
  });
  const style = fragment("10-style.js");
  vm.runInContext(slice(style, "  function dreamSkinRefreshAction(", "  function applyDreamSkinLiveUpdate("), context);
  return { context, calls };
}

test("dream skin refresh action does not wait for backend settings when the external runtime is installed", () => {
  const { context } = refreshHarness({ external: true, loaded: false, enabled: false, installed: true });
  const action = context.dreamSkinRefreshAction as (input: Record<string, boolean>) => string;
  const base = { enabled: false, paused: false, backendLoaded: false, externalRuntime: true, runtimeInstalled: true };
  assert.equal(action(base), "ensure");
  assert.equal(action({ ...base, runtimeInstalled: false }), "idle");
  assert.equal(action({ ...base, backendLoaded: true }), "cleanup");
  assert.equal(action({ ...base, backendLoaded: true, enabled: true }), "ensure");
  assert.equal(action({ ...base, backendLoaded: true, enabled: true, paused: true }), "cleanup");
  assert.equal(action({ ...base, externalRuntime: false, enabled: true }), "install");
  assert.equal(action({ ...base, externalRuntime: false }), "cleanup");
});

test("refreshDreamSkin applies the main surface compatibility before backend settings load", () => {
  const pending = refreshHarness({ external: true, loaded: false, enabled: false, installed: true });
  (pending.context.refreshDreamSkin as () => void)();
  assert.deepEqual(pending.calls, ["main-surface", "ensure", "companion"]);

  // 注入时皮肤未开启（运行时不在页面里）：设置未读到前不擅自加兼容标记。
  const notInstalled = refreshHarness({ external: true, loaded: false, enabled: false, installed: false });
  (notInstalled.context.refreshDreamSkin as () => void)();
  assert.deepEqual(notInstalled.calls, ["companion"]);

  // 设置读到且明确关闭时照旧清理。
  const disabled = refreshHarness({ external: true, loaded: true, enabled: false, installed: true });
  (disabled.context.refreshDreamSkin as () => void)();
  assert.deepEqual(disabled.calls, ["cleanup"]);
});

test("startup settings loading keeps retrying with backoff instead of giving up after 15 seconds", () => {
  const context = vm.createContext({});
  vm.runInContext(slice(fragment("40-backend-settings.js"), "  function backendSettingsStartupRetryDelay(", "  function loadBackendSettingsForStartup("), context);
  const delay = context.backendSettingsStartupRetryDelay as (attempt: number) => number;
  assert.equal(delay(0), 250);
  assert.equal(delay(59), 250);
  assert.equal(delay(60), 500);
  assert.equal(delay(61), 1000);
  assert.equal(delay(63), 4000);
  assert.equal(delay(64), 5000);
  assert.equal(delay(89), 5000);
  assert.equal(delay(90), -1);
  let total = 0;
  for (let attempt = 0; delay(attempt) >= 0; attempt += 1) total += delay(attempt);
  assert.ok(total > 60_000 && total < 180_000, `total retry window ${total}ms`);
});

test("startup and heartbeat settings loads re-apply the dream skin", async () => {
  const source = fragment("40-backend-settings.js");
  let refreshes = 0;
  let loaded = true;
  const context = vm.createContext({
    codexPlusSettings: () => ({ conversationView: false }),
    loadBackendSettingsState: async () => loaded,
    codexRemoteSessionProviderOverrideEnabled: () => false,
    refreshCodexPlusBackendToggles: () => {},
    syncOfficialUsagePolicy: () => {}, syncCodexPlusTypingEffects: () => {},
    renderCodexPlusMenu: () => {}, refreshConversationView: () => {},
    syncCodexPlusBackendToggleButtons: () => {}, runScanStep: (fn: () => void) => fn(),
    refreshDreamSkin: () => { refreshes += 1; },
    globalThis: {},
  });
  vm.runInContext(slice(source, "  async function loadBackendSettings(", "  /**\n   * 启动期读取后端设置的重试间隔"), context);
  vm.runInContext(slice(source, "  let syncBackendSettingsInFlight", "  async function setBackendSetting("), context);
  await (context.loadBackendSettings as () => Promise<boolean>)();
  assert.equal(refreshes, 1);
  await (context.syncBackendSettingsFromHeartbeat as () => Promise<void>)();
  assert.equal(refreshes, 2);
  loaded = false;
  await (context.loadBackendSettings as () => Promise<boolean>)();
  await (context.syncBackendSettingsFromHeartbeat as () => Promise<void>)();
  assert.equal(refreshes, 2, "failed reads keep the current skin state");
});
