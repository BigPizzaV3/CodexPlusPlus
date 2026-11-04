import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
const declarations = source.slice(source.indexOf("  const previousCodexModelCatalogRuntime"),
  source.indexOf("  if (window.__CODEX_PLUS_TEST_SERVICE_TIER__)"));
const functions = source.slice(source.indexOf("  function codexPlusModelUnlockEnabled()"),
  source.indexOf("  function appServerModelRequestMethod("));

function runtime(sharedWindow: any = {}, initialSettings?: any) {
  let now = 10000;
  let settings = initialSettings || {
    relayProfilesEnabled: true, activeRelayId: "gateway", modelCatalogFingerprint: "catalog-1",
    relayProfiles: [{ id: "gateway", relayMode: "pureApi", modelList: "a/sol\nb/terra" }],
  };
  const pending: Array<{ resolve: (value: any) => void; reject: (error: Error) => void }> = [];
  const invalidated: any[] = [];
  const queries = ["local", "remote"].map(host => ({ queryKey: ["models", host] }));
  const Response = sharedWindow.testResponse ||= class {
    payload: any;
    constructor(payload: any) { this.payload = payload; }
    async json() { return this.payload; }
  };
  const context = vm.createContext({
    window: sharedWindow, Response, Map, Set, WeakMap, Promise, Date: { now: () => now },
    codexPlusBackendSettings: settings, codexPlusBackendSettingsLoaded: true,
    codexPlusSettings: () => ({ modelWhitelistUnlock: true }),
    codexRemoteSessionActiveProfile: () => settings.relayProfiles.find((entry: any) => entry.id === settings.activeRelayId),
    codexServiceTierModelFromValue: (value: any) => value,
    renderCodexPlusMenu: () => {}, scheduleCodexModelWhitelistRefresh: () => {},
    sendCodexPlusDiagnostic: () => {},
    uniqueValues: (values: any[]) => [...new Set(values.filter(value => typeof value === "string" && value.trim()))],
    readCodexAppServerPreparation: () => new Promise((resolve, reject) => pending.push({ resolve, reject })),
    findCodexQueryClient: () => ({
      getQueryCache: () => ({ getAll: () => [...queries, { queryKey: ["threads", "local"] }] }),
      invalidateQueries: (filter: any) => invalidated.push(filter.queryKey),
    }),
  });
  const api = vm.runInContext(`(() => {${declarations}${functions}
    return { load: loadCodexModelCatalog, refresh: refreshCodexModelCatalogForSettings,
      patch: patchModelContainer, install: installModelJsonResponsePatch,
      catalog: () => codexModelCatalog }; })()`, context);
  return {
    api, pending, invalidated, window: sharedWindow, settings: () => settings,
    tick: (ms: number) => { now += ms; },
    update: (next: any) => {
      const previous = settings;
      settings = next;
      context.codexPlusBackendSettings = next;
      api.refresh(previous, next);
    },
  };
}
const catalog = (models: string[], metadata = {}) => ({
  status: "ok", model_provider: "gateway", model: models[0], default_model: models[0],
  models, modelMetadata: metadata,
  sources: [{ id: "relay-profile:gateway", type: "relay_profile_model_list" }],
});
const settle = async () => { for (let index = 0; index < 15; index++) await Promise.resolve(); };

it("catalog fingerprint changes bypass cache and invalidate only local model queries", async () => {
  const r = runtime();
  const first = r.api.load();
  r.pending.shift()!.resolve(catalog(["a/sol", "b/terra"]));
  await first;
  const value = { hostId: "local", models: [{ model: "native" }], defaultModel: "a/sol" };
  r.api.patch(value);
  assert.deepEqual(value.models.map(entry => entry.model), ["a/sol", "b/terra"]);
  r.update({ ...r.settings(), modelCatalogFingerprint: "catalog-2" });
  assert.equal(r.pending.length, 1);
  r.pending.shift()!.resolve(catalog(["a/sol", "group/coding"]));
  await settle();
  r.api.patch(value);
  assert.deepEqual(value.models.map(entry => entry.model), ["a/sol", "group/coding"]);
  assert.equal(value.defaultModel, "a/sol");
  assert.deepEqual(JSON.parse(JSON.stringify(r.invalidated)), [["models", "local"], ["models", "local"]]);
  r.update({ ...r.settings(), codexAppTypingEffects: true });
  assert.equal(r.pending.length, 0);
});

it("late responses and reinjection cannot overwrite the current configuration", async () => {
  const r = runtime();
  const old = r.api.load();
  const stale = r.pending.shift()!;
  r.update({ ...r.settings(), modelCatalogFingerprint: "catalog-2" });
  r.pending.shift()!.resolve(catalog(["a/new"]));
  await settle();
  stale.resolve(catalog(["a/stale"]));
  await old;
  assert.equal(r.api.catalog().model, "a/new");
  const inFlight = r.api.load(true);
  const previous = r.pending.shift()!;
  const next = runtime(r.window, r.settings());
  const fresh = next.api.load(true);
  next.pending.shift()!.resolve(catalog(["a/current"]));
  await fresh;
  previous.resolve(catalog(["a/obsolete"]));
  await inFlight;
  assert.equal(next.api.catalog().model, "a/current");
});

it("failed and missing catalogs retain a successful snapshot with ten-second backoff", async () => {
  for (const status of ["failed", "missing"]) {
    const r = runtime();
    const first = r.api.load();
    r.pending.shift()!.resolve(catalog(["a/sol", "b/terra"]));
    await first;
    const failed = r.api.load(true);
    r.pending.shift()!.resolve({ status, model: "current", default_model: "default", models: [] });
    const failure = await failed;
    assert.equal(failure.model, "current");
    assert.equal(failure.default_model, "default");
    assert.equal(r.api.catalog().model, "a/sol");
    for (let elapsed = 200; elapsed < 10000; elapsed += 200) {
      r.tick(200);
      await r.api.load();
      r.update({ ...r.settings() });
      assert.equal(r.pending.length, 0);
    }
    const next = runtime(r.window, r.settings());
    next.tick(9999);
    await next.api.load();
    assert.equal(next.pending.length, 0, "reinjection preserves the previous retry deadline");
    next.tick(1);
    const retry = next.api.load();
    next.pending.shift()!.resolve(catalog(["a/recovered"]));
    await retry;
    assert.equal(next.api.catalog().model, "a/recovered");
  }
});

it("a new supplier does not inherit a previous supplier's successful snapshot on failure", async () => {
  const r = runtime();
  const first = r.api.load();
  r.pending.shift()!.resolve(catalog(["a/sol"]));
  await first;
  r.update({ ...r.settings(), activeRelayId: "other", modelCatalogFingerprint: "other-catalog" });
  assert.equal(r.api.catalog().status, "loading");
  r.pending.shift()!.reject(new Error("unavailable"));
  await settle();
  assert.deepEqual([...r.api.catalog().models], []);
  assert.equal(r.api.catalog().status, "failed");
});

it("catalog patches respect hidden entries and never alter selected model or reasoning fields", async () => {
  const r = runtime();
  const first = r.api.load();
  r.pending.shift()!.resolve(catalog(["group/1", "a/sol"], { "group/1": { hidden: true } }));
  await first;
  const value = { hostId: "local", model: "saved-model", reasoningEffort: "ultra",
    models: [{ model: "group/1", hidden: false }, { model: "a/sol", hidden: true }],
    hiddenModels: ["group/1", "a/sol"], defaultModel: "group/1" };
  r.api.patch(value);
  assert.equal(value.model, "saved-model");
  assert.equal(value.reasoningEffort, "ultra");
  assert.equal(value.defaultModel, "group/1");
  assert.equal(value.models[0].hidden, true);
  assert.equal(value.models[1].hidden, false);
  assert.deepEqual([...value.hiddenModels], ["group/1"]);
});

it("response wrappers use the current runtime after hot reinjection", async () => {
  const r = runtime();
  const first = r.api.load();
  r.pending.shift()!.resolve(catalog(["a/old"]));
  await first;
  r.api.install();
  const next = runtime(r.window, r.settings());
  const fresh = next.api.load(true);
  next.pending.shift()!.resolve(catalog(["a/new"]));
  await fresh;
  next.api.install();
  const response = new r.window.testResponse({ hostId: "local", models: [{ model: "native" }] });
  const value = await response.json();
  assert.deepEqual(value.models.map((entry: any) => entry.model), ["a/new"]);
});
