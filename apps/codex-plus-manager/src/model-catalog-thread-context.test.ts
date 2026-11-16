import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import vm from "node:vm";

// #3137：创建聊天（codexThreadRuntimeContext）在配置指纹未变时必须复用已校验
// 目录快照，不能每次都强刷并在 2s 竞速里被偶发的慢桥接掐成超时。

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
const declarations = source.slice(source.indexOf("  const previousCodexModelCatalogRuntime"),
  source.indexOf("  if (window.__CODEX_PLUS_TEST_SERVICE_TIER__)"));
const functions = source.slice(source.indexOf("  function codexPlusModelUnlockEnabled()"),
  source.indexOf("  function noteCodexThreadRuntimeContext("));

function runtime(sharedWindow: any = {}, initialSettings?: any) {
  let now = 10000;
  let settings = initialSettings || {
    relayProfilesEnabled: true, activeRelayId: "gateway", modelCatalogFingerprint: "catalog-1",
    relayProfiles: [{ id: "gateway", relayMode: "pureApi", modelList: "a/sol\nb/terra" }],
  };
  const pending: Array<{ resolve: (value: any) => void; reject: (error: Error) => void }> = [];
  const timers: Array<() => void> = sharedWindow.testTimers ||= [];
  sharedWindow.setTimeout ||= (callback: () => void) => timers.push(callback);
  const context = vm.createContext({
    window: sharedWindow, Map, Set, WeakMap, Promise, Date: { now: () => now },
    codexPlusBackendSettings: settings, codexPlusBackendSettingsLoaded: true,
    codexPlusSettings: () => ({ modelWhitelistUnlock: true }),
    codexRemoteSessionActiveProfile: () => settings.relayProfilesEnabled
      ? settings.relayProfiles.find((entry: any) => entry.id === settings.activeRelayId) : null,
    codexServiceTierModelFromValue: (value: any) => value,
    renderCodexPlusMenu: () => {}, scheduleCodexModelWhitelistRefresh: () => {},
    sendCodexPlusDiagnostic: () => {},
    uniqueValues: (values: any[]) => [...new Set(values.filter((value: any) => typeof value === "string" && value.trim()))],
    readCodexAppServerPreparation: () => new Promise((resolve, reject) => pending.push({ resolve, reject })),
    findCodexQueryClient: () => null,
    codexThreadContextSignature: (config: any) => JSON.stringify(config),
  });
  const api = vm.runInContext(`(() => {${declarations}${functions}
    return { load: loadCodexModelCatalog, refresh: refreshCodexModelCatalogForSettings,
      threadContext: codexThreadRuntimeContext, catalog: () => codexModelCatalog }; })()`, context);
  return {
    api, pending, timers, window: sharedWindow, settings: () => settings,
    tick: (ms: number) => { now += ms; },
    update: (next: any) => {
      const previous = settings;
      settings = next;
      context.codexPlusBackendSettings = next;
      api.refresh(previous, next);
    },
  };
}
const settle = async () => { for (let index = 0; index < 15; index++) await Promise.resolve(); };
const catalogWithContext = (models: string[]) => ({
  status: "ok", model_provider: "gateway", model: models[0], default_model: models[0],
  models, modelMetadata: {},
  sources: [{ id: "relay-profile:gateway", type: "relay_profile_model_list" }],
  runtimeContext: { status: "ok", models: Object.fromEntries(models.map(model =>
    [model, { model_context_window: 100, model_auto_compact_token_limit: 50 }])) },
});
async function load(r: ReturnType<typeof runtime>, models: string[]) {
  const request = r.api.load(true);
  r.pending.shift()!.resolve(catalogWithContext(models));
  await request;
}

it("reuses a validated snapshot for chat start without issuing a forced request", async () => {
  const r = runtime();
  await load(r, ["a/sol"]);
  const context = await r.api.threadContext({ model: "a/sol" });
  assert.equal(r.pending.length, 0, "an unchanged fingerprint must not force a new catalog read");
  assert.equal(context.model, "a/sol");
  assert.equal(context.config.model_context_window, 100);
});

it("refreshes an expired snapshot in the background without blocking chat start", async () => {
  const r = runtime();
  await load(r, ["a/sol"]);
  await r.api.threadContext({ model: "a/sol" });
  r.tick(10000);
  await r.api.threadContext({ model: "a/sol" });
  assert.equal(r.pending.length, 1, "an expired snapshot must trigger a background refresh");
  r.pending.shift()!.resolve(catalogWithContext(["a/sol", "b/terra"]));
  await settle();
  assert.deepEqual(r.api.catalog().models, ["a/sol", "b/terra"]);
});

it("forces a real refresh when the fingerprint changed and no snapshot is validated", async () => {
  const r = runtime();
  await load(r, ["a/sol"]);
  await r.api.threadContext({ model: "a/sol" });
  r.update({ ...r.settings(), modelCatalogFingerprint: "catalog-2" });
  assert.equal(r.pending.length, 1, "a changed fingerprint makes the settings refresh force a read");
  r.pending.shift()!.resolve({ status: "failed", models: [] });
  await settle();
  const requested = r.api.threadContext({ model: "b/terra" });
  assert.equal(r.pending.length, 1, "without a validated snapshot, thread start must force a fresh read");
  r.pending.shift()!.resolve(catalogWithContext(["b/terra"]));
  const context = await requested;
  assert.equal(context.model, "b/terra");
});

it("still surfaces a real failure when no validated snapshot exists", async () => {
  const r = runtime();
  const requested = r.api.threadContext({ model: "a/sol" });
  assert.equal(r.pending.length, 1);
  r.pending.shift()!.reject(new Error("Codex++ preparation read timed out: /codex-model-catalog"));
  await assert.rejects(() => requested, /timed out|读取上下文配置失败/);
});
