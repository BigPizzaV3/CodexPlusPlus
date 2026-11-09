import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
const declarations = source.slice(source.indexOf("  const previousCodexModelCatalogRuntime"),
  source.indexOf("  if (window.__CODEX_PLUS_TEST_SERVICE_TIER__)"));
const functions = source.slice(source.indexOf("  function codexPlusModelUnlockEnabled()"),
  source.indexOf("  function codexPerModelContextEnabled("));

function runtime(sharedWindow: any = {}, initialSettings?: any) {
  let now = 10000;
  let settings = initialSettings || {
    relayProfilesEnabled: true, activeRelayId: "gateway", modelCatalogFingerprint: "catalog-1",
    relayProfiles: [{ id: "gateway", relayMode: "pureApi", modelList: "a/sol\nb/terra" }],
  };
  const pending: Array<{ resolve: (value: any) => void; reject: (error: Error) => void }> = [];
  const invalidated: any[] = [];
  const timers: Array<() => void> = sharedWindow.testTimers ||= [];
  const listeners: Map<string, Array<(event: any) => void>> = sharedWindow.testListeners ||= new Map();
  sharedWindow.setTimeout ||= (callback: () => void) => timers.push(callback);
  sharedWindow.addEventListener ||= (type: string, callback: (event: any) => void) => {
    if (!listeners.has(type)) listeners.set(type, []);
    listeners.get(type)!.push(callback);
  };
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
    codexRemoteSessionActiveProfile: () => settings.relayProfilesEnabled
      ? settings.relayProfiles.find((entry: any) => entry.id === settings.activeRelayId) : null,
    codexServiceTierModelFromValue: (value: any) => value,
    renderCodexPlusMenu: () => {}, scheduleCodexModelWhitelistRefresh: () => {},
    sendCodexPlusDiagnostic: () => {},
    uniqueValues: (values: any[]) => [...new Set(values.filter(value => typeof value === "string" && value.trim()))],
    readCodexAppServerPreparation: () => new Promise((resolve, reject) => pending.push({ resolve, reject })),
    findCodexQueryClient: () => sharedWindow.testQueryClient || ({
      getQueryCache: () => ({
        getAll: () => [...queries, { queryKey: ["threads", "local"] }],
        findAll: () => [],
      }),
      invalidateQueries: (filter: any) => invalidated.push(filter.queryKey),
    }),
  });
  const api = vm.runInContext(`(() => {${declarations}${functions}
    return { load: loadCodexModelCatalog, refresh: refreshCodexModelCatalogForSettings,
      patch: patchModelContainer, install: installModelJsonResponsePatch,
      installMessages: patchAppServerModelMessages, record: recordModelListRequest,
      patchMcp: patchMcpModelResponseData, patchAppResult: patchAppServerModelResult,
      allowed: codexPureApiAllowedModelSet,
      patchStatsig: patchStatsigModelDynamicConfig,
      installStatsig: patchStatsigModelWhitelist,
      catalog: () => codexModelCatalog }; })()`, context);
  return {
    api, pending, invalidated, timers, window: sharedWindow, settings: () => settings,
    emit: (type: string, event: any) => listeners.get(type)?.forEach(callback => callback(event)),
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
async function load(r: ReturnType<typeof runtime>, models: string[], metadata = {}) {
  const request = r.api.load(true);
  r.pending.shift()!.resolve(catalog(models, metadata));
  await request;
}

function queryClient(entries: Array<{ queryKey: any[]; state: { data: any } }>) {
  const pending: Array<{ key: any[]; resolve: () => void }> = [];
  const client = {
    getQueryCache: () => ({
      getAll: () => entries,
      findAll: ({ queryKey, predicate }: { queryKey: any[]; predicate: (entry: any) => boolean }) =>
        entries.filter(entry => queryKey.every((part, index) => entry.queryKey[index] === part) && predicate(entry)),
    }),
    setQueryData: (key: any[], update: (value: any) => any) => {
      const entry = entries.find(value => JSON.stringify(value.queryKey) === JSON.stringify(key));
      if (entry) entry.state.data = update(entry.state.data);
    },
    invalidateQueries: ({ queryKey }: { queryKey: any[] }) =>
      new Promise<void>(resolve => pending.push({ key: queryKey, resolve })),
  };
  return { client, pending };
}

const nativeQueryKey = (host = "local", provider = "gateway") =>
  ["models", "list", host, "no-auth", 100, provider, "fixture-catalog"];

it("invalidates local queries without modifying a same-key last page", async () => {
  const local = { queryKey: nativeQueryKey(), state: { data: { data: [
    { model: "a/old", displayName: "Old", isDefault: true },
    { model: "native-only" },
  ], nextCursor: null, model: "selected", reasoningEffort: "ultra" } } };
  const remote = { queryKey: nativeQueryKey("remote"), state: { data: structuredClone(local.state.data) } };
  const { client, pending } = queryClient([local, remote]);
  const original = local.state.data;
  const originalRemote = remote.state.data;
  const shared = { testQueryClient: client };
  const r = runtime(shared);
  await load(r, ["a/old", "a/new"], { "a/new": { displayName: "New model" } });
  assert.equal(local.state.data, original);
  assert.equal(remote.state.data, originalRemote);
  assert.deepEqual(original.data.map(entry => entry.model), ["a/old", "native-only"]);
  assert.deepEqual(pending.map(item => item.key), [local.queryKey]);
  const next = runtime(shared, r.settings());
  await load(next, ["a/current"]);
  pending.find(item => item.key === local.queryKey)!.resolve();
  await settle();
  assert.equal(local.state.data, original);
  assert.equal(local.state.data.model, "selected");
  assert.equal(local.state.data.reasoningEffort, "ultra");
});

it("preserves native descriptors when a catalog change reintroduces a filtered model", async () => {
  const local = { hostId: "local", models: [
    { model: "a/old", displayName: "Old" },
    { model: "a/returning", description: "Original native descriptor", nativeCapability: true },
  ] };
  const r = runtime();
  await load(r, ["a/old"]);
  r.api.patch(local);
  assert.deepEqual(Array.from(local.models, entry => entry.model), ["a/old"]);

  r.update({ ...r.settings(), modelCatalogFingerprint: "catalog-2" });
  r.pending.shift()!.resolve(catalog(["a/old", "a/returning"]));
  await settle();
  r.api.patch(local);
  const restored = local.models.find(entry => entry.model === "a/returning");
  assert.equal(restored?.description, "Original native descriptor");
  assert.equal(restored?.nativeCapability, true);
});

it("does not restore an obsolete snapshot after native model data changes", async () => {
  const local = { hostId: "local", models: [
    { model: "a/old", description: "Old" }, { model: "a/returning", description: "Stale" },
  ] };
  const r = runtime();
  await load(r, ["a/old"]);
  r.api.patch(local);
  local.models.splice(0, 1, { model: "a/returning", description: "Fresh native data" });

  r.update({ ...r.settings(), modelCatalogFingerprint: "catalog-2" });
  r.pending.shift()!.resolve(catalog(["a/returning"]));
  await settle();
  r.api.patch(local);
  assert.deepEqual(Array.from(local.models, entry => entry.model), ["a/returning"]);
  assert.equal(local.models[0].description, "Fresh native data");
});

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

it("retries a failed catalog refresh without requiring another settings event", async () => {
  const r = runtime();
  await load(r, ["a/old"]);
  r.update({ ...r.settings(), modelCatalogFingerprint: "catalog-2" });
  r.pending.shift()!.reject(new Error("temporary failure"));
  await settle();
  assert.equal(r.api.catalog().model, "a/old");
  assert.equal(r.timers.length, 1);

  r.tick(10000);
  r.timers.shift()!();
  assert.equal(r.pending.length, 1, "the failed refresh must retry after its cooldown");
  r.pending.shift()!.resolve(catalog(["a/new"]));
  await settle();
  assert.equal(r.api.catalog().model, "a/new");
});

it("keeps retrying transient failures without bypassing the cooldown", async () => {
  const r = runtime();
  await load(r, ["a/old"]);
  const failed = r.api.load(true);
  r.pending.shift()!.reject(new Error("first failure"));
  await failed;
  r.tick(5000);
  r.timers.shift()!();
  assert.equal(r.pending.length, 0);
  assert.equal(r.timers.length, 1, "an early timer must retain the remaining cooldown");
  r.tick(5000);
  r.timers.shift()!();
  r.pending.shift()!.reject(new Error("second failure"));
  await settle();
  assert.equal(r.api.catalog().model, "a/old");
  assert.equal(r.timers.length, 1);
  r.tick(10000);
  r.timers.shift()!();
  r.pending.shift()!.resolve(catalog(["a/recovered"]));
  await settle();
  assert.equal(r.api.catalog().model, "a/recovered");
});

it("discards retry timers after a source change or successful refresh", async () => {
  for (const action of ["source", "success"]) {
    const r = runtime();
    await load(r, ["a/old"]);
    const failed = r.api.load(true);
    r.pending.shift()!.reject(new Error("temporary failure"));
    await failed;
    assert.equal(r.timers.length, 1);
    if (action === "source") {
      r.update({ ...r.settings(), activeRelayId: "other", modelCatalogFingerprint: "other" });
    } else {
      void r.api.load(true);
    }
    r.pending.shift()!.resolve(catalog(["a/new"]));
    await settle();
    r.tick(10000);
    r.timers.shift()!();
    assert.equal(r.pending.length, 0, "an obsolete retry must not initiate another request");
    assert.equal(r.api.catalog().model, "a/new");
  }
});

it("continues a failed catalog retry after reinjection", async () => {
  const r = runtime();
  await load(r, ["a/old"]);
  const failed = r.api.load(true);
  r.pending.shift()!.reject(new Error("temporary failure"));
  await failed;
  const next = runtime(r.window, r.settings());
  r.tick(10000);
  next.tick(10000);
  for (const timer of r.timers.splice(0)) timer();
  assert.equal(r.pending.length, 0, "the old runtime must not issue the retry");
  assert.equal(next.pending.length, 1, "the current runtime must retain the retry");
  next.pending.shift()!.resolve(catalog(["a/new"]));
  await settle();
  assert.equal(next.api.catalog().model, "a/new");
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

it("changes source when the active supplier keeps its ID but changes mode or protocol", async () => {
  for (const change of [{ relayMode: "official" }, { protocol: "chat" }]) {
    const r = runtime({}, {
      relayProfilesEnabled: true, activeRelayId: "gateway", modelCatalogFingerprint: "unchanged",
      relayProfiles: [{ id: "gateway", relayMode: "pureApi", protocol: "responses" }],
    });
    await load(r, ["a/old"]);
    const selected = { hostId: "local", model: "user-selected", reasoningEffort: "ultra",
      models: [{ model: "native" }] };
    const oldRequest = r.api.load(true);
    const stale = r.pending.shift()!;
    const settings = r.settings();
    r.update({ ...settings, relayProfiles: [{ ...settings.relayProfiles[0], ...change }] });
    assert.equal(r.api.catalog().status, "loading");
    assert.equal(r.pending.length, 1, "a changed source must trigger a fresh catalog read");
    r.pending.shift()!.reject(new Error("unavailable"));
    await settle();
    stale.resolve(catalog(["a/stale"]));
    await oldRequest;
    assert.deepEqual([...r.api.catalog().models], []);
    assert.equal(r.api.catalog().status, "failed");
    r.api.patch(selected);
    assert.equal(selected.model, "user-selected");
    assert.equal(selected.reasoningEffort, "ultra");
  }
});

it("invalidates the previous catalog when the same supplier changes connection identity", async () => {
  const r = runtime({}, {
    relayProfilesEnabled: true, activeRelayId: "gateway", modelCatalogFingerprint: "unchanged",
    modelCatalogSourceIdentity: "source-1",
    relayProfiles: [{ id: "gateway", relayMode: "pureApi", protocol: "responses" }],
  });
  await load(r, ["a/old"]);
  const oldRequest = r.api.load(true);
  const stale = r.pending.shift()!;
  r.update({ ...r.settings(), modelCatalogSourceIdentity: "source-2" });
  assert.equal(r.api.catalog().status, "loading");
  assert.equal(r.pending.length, 1);
  r.pending.shift()!.reject(new Error("unavailable"));
  await settle();
  stale.resolve(catalog(["a/stale"]));
  await oldRequest;
  assert.deepEqual([...r.api.catalog().models], []);
  assert.equal(r.api.catalog().status, "failed");
});

it("catalog changes notify the installed picker without requiring it during initialization", async () => {
  const r = runtime();
  await load(r, ["a/sol"]);
  const notifications: any[] = [];
  r.window.__codexPlusModelPickerGroupsRuntime = { catalogChanged: (...args: any[]) => { notifications.push(args); } };
  r.update({ ...r.settings(), activeRelayId: "other", modelCatalogFingerprint: "other-catalog" });
  assert.equal(notifications.length, 1, "clearing the previous supplier notifies the picker");
  assert.equal(notifications[0][0].sourceChanged, true);
  r.pending.shift()!.resolve(catalog(["b/terra"]));
  await settle();
  assert.equal(notifications.length, 2, "publishing the new catalog notifies the picker");
  assert.equal(notifications[1][0].status, "loading");
  assert.deepEqual(Array.from(notifications[1][1].models), ["b/terra"]);
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

it("recognizes only a verified local pure API multi-instance catalog when relay switching is disabled", async () => {
  const profile = { id: "gateway", relayMode: "pureApi", modelList: "a/sol\nb/terra" };
  const worker = { relayProfilesEnabled: false, codexMultiInstance: true, activeRelayId: "gateway",
    modelCatalogFingerprint: "worker-1", relayProfiles: [profile] };
  for (const [settings, allowed] of [
    [worker, true],
    [{ ...worker, codexMultiInstance: false }, false],
    [{ ...worker, codexMultiInstance: undefined }, false],
    [{ ...worker, activeRelayId: "other" }, false],
    [{ ...worker, relayProfiles: [profile, profile] }, false],
    [{ ...worker, relayProfiles: [{ ...profile, relayMode: "aggregate" }] }, false],
  ] as const) {
    const r = runtime({}, settings);
    await load(r, ["a/sol", "b/terra"]);
    assert.equal(!!r.api.allowed("local"), allowed);
    assert.equal(r.api.allowed("remote"), null);
    assert.equal(r.api.allowed(""), null);
    if (allowed) {
      const result = { data: [{ model: "native" }], nextCursor: null,
        model: "previous-model", reasoningEffort: "high" };
      r.api.patchAppResult("model/list", result, "local", { cursor: null });
      assert.deepEqual(result.data.map(entry => entry.model), ["a/sol", "b/terra"]);
      assert.equal(result.model, "previous-model");
      assert.equal(result.reasoningEffort, "high");
    }
  }
  const r = runtime({}, worker);
  const request = r.api.load(true);
  r.pending.shift()!.resolve({ ...catalog(["foreign"]), model_provider: "other",
    sources: [{ id: "relay-profile:other" }] });
  await request;
  assert.equal(r.api.allowed("local"), null);
  const result = { data: [{ model: "native" }], nextCursor: null };
  r.api.patchAppResult("model/list", result, "local", { cursor: null });
  assert.deepEqual(result.data.map(entry => entry.model), ["native"]);
});

it("preserves model injection for official multi-instance profiles", async () => {
  const r = runtime({}, { relayProfilesEnabled: false, codexMultiInstance: true,
    activeRelayId: "gateway", relayProfiles: [{ id: "gateway", relayMode: "official" }] });
  await load(r, ["official/model"]);
  const result = { data: [{ model: "native" }], nextCursor: null };
  r.api.patchAppResult("model/list", result, "local", { cursor: null });
  assert.deepEqual(result.data.map(entry => entry.model), ["native", "official/model"]);
});

it("refreshes complete multi-instance model responses without changing the current selection", async () => {
  const worker = { relayProfilesEnabled: false, codexMultiInstance: true, activeRelayId: "gateway",
    modelCatalogFingerprint: "worker-1",
    relayProfiles: [{ id: "gateway", relayMode: "pureApi", modelList: "a/sol" }] };
  const r = runtime({}, worker);
  await load(r, ["a/sol", "b/old"]);
  const result = { data: [{ model: "native" }], nextCursor: null,
    model: "previous-model", reasoningEffort: "high" };
  r.api.patchAppResult("model/list", result, "local", { cursor: null });
  assert.deepEqual(result.data.map(entry => entry.model), ["a/sol", "b/old"]);

  r.update({ ...worker, modelCatalogFingerprint: "worker-2" });
  r.pending.shift()!.resolve(catalog(["a/sol", "b/new"]));
  await settle();
  r.api.patchAppResult("model/list", result, "local", { cursor: null });
  assert.deepEqual(result.data.map(entry => entry.model), ["a/sol", "b/new"]);
  assert.equal(result.model, "previous-model");
  assert.equal(result.reasoningEffort, "high");
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

it("does not rewrite an HTTP response with an unknown page position", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  r.api.install();
  for (const nextCursor of ["next", null]) {
    const value = { hostId: "local", data: [{ model: "native" }], nextCursor };
    const original = structuredClone(value);
    assert.deepEqual(await new r.window.testResponse(value).json(), original);
  }
});

it("does not inject local models into model responses with an unknown host", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  r.api.install();
  const http = { models: [{ model: "native" }] };
  assert.deepEqual(await new r.window.testResponse(http).json(), { models: [{ model: "native" }] });
  const app = { data: [{ model: "native" }], nextCursor: null };
  r.api.patchAppResult("model/list", app, "", { cursor: null });
  assert.deepEqual(app.data, [{ model: "native" }]);
  r.api.record("no-host", "", { cursor: null });
  const mcp = { type: "mcp-response", message: { id: "no-host",
    result: { data: [{ model: "native" }], nextCursor: null } } };
  assert.equal(r.api.patchMcp(mcp), false);
  assert.deepEqual(mcp.message.result.data, [{ model: "native" }]);
});

it("restores unchanged shared local snapshots for unknown hosts without replacing newer data", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  r.api.install();
  for (const path of ["http", "app", "mcp"]) {
    const original = [{ model: "a/custom" }, { model: "native", description: "original" }];
    const models = original.slice();
    r.api.patch({ hostId: "local", models });
    assert.deepEqual(models.map(entry => entry.model), ["a/custom"]);
    if (path === "http") {
      await new r.window.testResponse({ models }).json();
    } else if (path === "app") {
      r.api.patchAppResult("model/list", { data: models, nextCursor: null }, "", { cursor: null });
    } else {
      r.api.record(path, "", { cursor: null });
      assert.equal(r.api.patchMcp({ type: "mcp-response", message: { id: path,
        result: { data: models, nextCursor: null } } }), true);
    }
    assert.deepEqual(models, original, `${path} must restore the original native objects`);
  }
  const models = [{ model: "a/custom" }, { model: "old-native" }];
  r.api.patch({ hostId: "local", models });
  models.splice(0, models.length, { model: "new-native" });
  r.api.patchAppResult("model/list", { data: models, nextCursor: null }, "", { cursor: null });
  assert.deepEqual(models, [{ model: "new-native" }]);
});

it("does not leak additions from an unfiltered local list to another host", async () => {
  const settings = {
    relayProfilesEnabled: true, activeRelayId: "gateway", modelCatalogFingerprint: "catalog-1",
    relayProfiles: [{ id: "gateway", relayMode: "official" }],
  };
  const r = runtime({}, settings);
  await load(r, ["a/custom"]);
  const models = [{ model: "native" }];
  r.api.patchAppResult("model/list", { data: models, nextCursor: null }, "local", { cursor: null });
  assert.deepEqual(models.map(entry => entry.model), ["native", "a/custom"]);
  r.api.patchAppResult("model/list", { data: models, nextCursor: null }, "ssh:remote", { cursor: null });
  assert.deepEqual(models, [{ model: "native" }]);
});

it("refreshes and restores unfiltered local names, metadata and sets", async () => {
  const settings = {
    relayProfilesEnabled: true, activeRelayId: "gateway", modelCatalogFingerprint: "catalog-1",
    relayProfiles: [{ id: "gateway", relayMode: "official" }],
  };
  const r = runtime({}, settings);
  await load(r, ["a/old"], { "a/old": { displayName: "Local name" } });
  const native = { model: "a/old", displayName: "Native name" };
  const container = { models: [native], availableModels: ["native"], available_models: new Set(["native"]) };
  r.api.patch(container, "local");
  assert.equal(container.models[0].displayName, "Local name");
  assert.deepEqual(container.availableModels, ["native", "a/old"]);
  assert.deepEqual([...container.available_models], ["native", "a/old"]);

  r.update({ ...settings, modelCatalogFingerprint: "catalog-2" });
  r.pending.shift()!.resolve(catalog(["a/new"]));
  await settle();
  r.api.patch(container, "local");
  assert.equal(container.models[0], native);
  assert.deepEqual(container.models.map(entry => entry.model), ["a/old", "a/new"]);
  assert.deepEqual(container.availableModels, ["native", "a/new"]);
  assert.deepEqual([...container.available_models], ["native", "a/new"]);
  r.api.patch(container, "ssh:remote");
  assert.deepEqual(container.models, [native]);
  assert.deepEqual(container.availableModels, ["native"]);
  assert.deepEqual([...container.available_models], ["native"]);
});

it("patches the first MCP and app-server model page even when it has a next cursor", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  r.api.installMessages();
  for (const [cursor, nextCursor, shouldPatch] of [
    [null, null, true],
    [null, "next", true],
    ["previous", null, false],
  ] as const) {
    const id = `${cursor}-${nextCursor}`;
    const request = { id, method: "model/list", params: { hostId: "local", cursor } };
    r.emit("codex-message-from-view", { detail: { type: "mcp-request", request } });
    const response = { type: "mcp-response", message: { id,
      result: { data: [{ model: "native" }], nextCursor } } };
    r.api.patchMcp(response);
    assert.deepEqual(response.message.result.data.map(entry => entry.model), [shouldPatch ? "a/custom" : "native"]);
    const appResult = { data: [{ model: "native" }], nextCursor };
    r.api.patchAppResult("model/list", appResult, "local", { cursor });
    assert.deepEqual(appResult.data.map(entry => entry.model), [shouldPatch ? "a/custom" : "native"]);
  }
});

it("adds all local models to a native first page containing more than 100 entries", async () => {
  const models = Array.from({ length: 101 }, (_, index) => `provider/model-${index + 1}`);
  const r = runtime();
  await load(r, models);
  r.api.installMessages();
  const request = { id: "over-100", method: "model/list",
    params: { hostId: "local", cursor: null, limit: 100 } };
  r.emit("codex-message-from-view", { detail: { type: "mcp-request", request } });
  const response = {
    type: "mcp-response",
    message: {
      id: request.id,
      result: { data: models.slice(0, 100).map(model => ({ model })), nextCursor: "page-2" },
    },
  };
  assert.equal(r.api.patchMcp(response), true);
  assert.equal(response.message.result.data.length, 101);
  assert.deepEqual(response.message.result.data.map(entry => entry.model), models);
});

it("does not infer a first page from an older listener or a reused request id", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  r.api.record("legacy", "local");
  const legacy = { type: "mcp-response", message: { id: "legacy",
    result: { data: [{ model: "native" }], nextCursor: null } } };
  assert.equal(r.api.patchMcp(legacy), false);
  assert.equal(legacy.message.result.data[0].model, "native");

  r.api.record("collision", "local", { cursor: null });
  r.api.record("collision", "local", { cursor: "previous" });
  const collision = { type: "mcp-response", message: { id: "collision",
    result: { data: [{ model: "native" }], nextCursor: null } } };
  assert.equal(r.api.patchMcp(collision), false);
  assert.equal(collision.message.result.data[0].model, "native");

  const result = { data: [{ model: "native" }], next_cursor: null };
  r.api.patchAppResult("model/list", result, "local");
  assert.equal(result.data[0].model, "native");
});

it("does not treat an unknown request cursor as the first page", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  const cases = [
    { cursor: undefined },
    { method: "model/list" },
    { method: "model/list", params: { cursor: undefined } },
  ];
  for (const [index, params] of cases.entries()) {
    const result = { data: [{ model: "native" }], nextCursor: null };
    r.api.patchAppResult("model/list", result, "local", params);
    assert.equal(result.data[0].model, "native");

    r.api.record(`unknown-${index}`, "local", params);
    const response = { type: "mcp-response", message: { id: `unknown-${index}`,
      result: { data: [{ model: "native" }], nextCursor: null } } };
    assert.equal(r.api.patchMcp(response), false);
    assert.equal(response.message.result.data[0].model, "native");
  }
});

it("does not patch unpaginated responses when request parameters are unavailable", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  const appResult = { data: [{ model: "native" }] };
  r.api.patchAppResult("model/list", appResult, "local");
  assert.equal(appResult.data[0].model, "native");
  r.api.record("unknown-params", "local");
  const mcp = { type: "mcp-response", message: { id: "unknown-params",
    result: { data: [{ model: "native" }] } } };
  assert.equal(r.api.patchMcp(mcp), false);
  assert.equal(mcp.message.result.data[0].model, "native");
});

it("retains missing MCP request parameters when recording pagination context", async () => {
  const r = runtime();
  await load(r, ["a/custom"]);
  r.api.installMessages();
  for (const params of [undefined, null]) {
    const request = { id: `missing-${params}`, method: "model/list", hostId: "local", params };
    r.emit("codex-message-from-view", { detail: { type: "mcp-request", request } });
    assert.equal((request.params as any).includeHidden, true);
    const response = { type: "mcp-response", message: { id: request.id,
      result: { data: [{ model: "native" }], nextCursor: null } } };
    assert.equal(r.api.patchMcp(response), false);
    assert.equal(response.message.result.data[0].model, "native");
  }
});

it("in-flight model/list requests retain their host and are consumed once after reinjection", async () => {
  const r = runtime();
  await load(r, ["a/old"]);
  r.api.installMessages();
  const request = { id: 17, method: "model/list", params: { hostId: "local" } };
  r.emit("codex-message-from-view", { detail: { type: "mcp-request", request } });
  assert.equal((request.params as any).includeHidden, true);
  const next = runtime(r.window, r.settings());
  await load(next, ["a/new"]);
  next.api.installMessages();
  const response = { type: "mcp-response", message: { id: 17, result: { models: [{ model: "native" }] } } };
  next.emit("message", { data: response });
  assert.deepEqual(response.message.result.models.map(entry => entry.model), ["a/new"]);
  const duplicate = { type: "mcp-response", message: { id: 17, result: { models: [{ model: "native" }] } } };
  assert.equal(next.api.patchMcp(duplicate), false);
  assert.equal(duplicate.message.result.models[0].model, "native");
});

it("an old request timeout cannot expire a reused ID after reinjection", async () => {
  for (const reinject of [false, true]) {
    const r = runtime();
    await load(r, ["a/current"]);
    r.api.record("reused", "local", {});
    assert.equal(r.api.patchMcp({ type: "mcp-response", message: { id: "reused", result: { data: [] } } }), true);
    const next = reinject ? runtime(r.window, r.settings()) : r;
    next.api.record("reused", "local", {});
    r.timers[0]();
    const response = { type: "mcp-response", message: { id: "reused", result: { data: [{ model: "native" }] } } };
    assert.equal(next.api.patchMcp(response), true);
    assert.deepEqual(response.message.result.data.map(entry => entry.model), ["a/current"]);
    next.api.record("expired", "local");
    next.timers.at(-1)!();
    assert.equal(next.api.patchMcp({ type: "mcp-response", message: { id: "expired", result: { data: [] } } }), false);
  }
});

it("colliding local and remote request IDs bypass catalog patches even after reinjection", async () => {
  for (const hosts of [["local", "remote"], ["remote", "local"]]) {
    const r = runtime();
    await load(r, ["shared-model"], { "shared-model": { hidden: true } });
    r.api.record("collision", hosts[0]);
    const next = runtime(r.window, r.settings());
    next.api.record("collision", hosts[1]);
    next.api.record("collision", hosts[1]);
    const response = { type: "mcp-response", hostId: "local", message: { id: "collision",
      result: { models: [{ model: "shared-model", hidden: false }, { model: "remote-native" }] } } };
    const original = structuredClone(response);
    assert.equal(next.api.patchMcp(response), false);
    assert.deepEqual(response, original);
    const duplicate = structuredClone(original);
    assert.equal(next.api.patchMcp(duplicate), false);
    assert.deepEqual(duplicate, original);
    next.api.record("unknown", "");
    const unknown = { type: "mcp-response", message: { id: "unknown", result: { data: [{ model: "native" }] } } };
    assert.equal(next.api.patchMcp(unknown), false);
    assert.deepEqual(unknown.message.result.data.map(entry => entry.model), ["native"]);
  }
});

it("explicitly remote model collections, metadata and defaults remain untouched", async () => {
  const r = runtime();
  await load(r, ["shared-model"], { "shared-model": { hidden: true, displayName: "Local name" } });
  for (const models of [[{ model: "shared-model", hidden: false, displayName: "Remote name" }], ["shared-model"]]) {
    const value = { hostId: "remote", models, data: [{ model: "shared-model", hidden: false }],
      availableModels: new Set(["remote-native"]), available_models: ["remote-native"],
      hiddenModels: ["shared-model"], hidden_models: ["shared-model"], defaultModel: "remote-native",
      model: "remote-selected", reasoningEffort: "ultra" };
    const original = structuredClone(value);
    assert.equal(r.api.patch(value, "local"), false, "explicit remote host overrides the local hint");
    assert.deepEqual(value, original);
    r.api.install();
    assert.deepEqual(await new r.window.testResponse(value).json(), original);
  }
});

it("MCP requests keep remote identity across reinjection and conflicting local response hints", async () => {
  const r = runtime();
  await load(r, ["shared-model"], { "shared-model": { hidden: true } });
  r.api.record("remote-request", "remote");
  const next = runtime(r.window, r.settings());
  const response = { type: "mcp-response", hostId: "local", message: { id: "remote-request",
    result: { models: [{ model: "shared-model", hidden: false }, { model: "remote-native" }] } } };
  const original = structuredClone(response);
  assert.equal(next.api.patchMcp(response), false);
  assert.deepEqual(response, original);
  next.api.record("nested-remote", "local");
  const nested = { type: "mcp-response", message: { id: "nested-remote",
    result: { hostId: "remote", models: [{ model: "shared-model", hidden: false }] } } };
  const nestedOriginal = structuredClone(nested);
  assert.equal(next.api.patchMcp(nested), false);
  assert.deepEqual(nested, nestedOriginal);
  assert.equal(next.api.patch(nested, "local"), false);
  assert.deepEqual(nested, nestedOriginal);
});

it("remote access restores native collections previously filtered through a shared local cache", async () => {
  const r = runtime();
  await load(r, ["shared-model", "a/local-only"], { "shared-model": { hidden: true, displayName: "Local name" } });
  const value = { models: [{ model: "shared-model", hidden: false, displayName: "Remote name" }, { model: "remote-native" }],
    availableModels: ["shared-model", "remote-native"], available_models: new Set(["shared-model", "remote-native"]),
    defaultModel: "remote-native" };
  const original = structuredClone(value);
  const nativeShared = value.models[0];
  r.api.patch(value, "local");
  assert.deepEqual(value.models.map(entry => entry.model), ["shared-model", "a/local-only"]);
  assert.equal(nativeShared.hidden, false);
  assert.equal(nativeShared.displayName, "Remote name");
  r.api.patch(value, "remote");
  assert.deepEqual(value, original);
  const config = { value: { available_models: ["remote-native"], defaultModel: "remote-native" } };
  r.api.patchStatsig(config, "local");
  r.api.patchStatsig(config, "remote");
  assert.deepEqual([...config.value.available_models], ["remote-native"]);
  assert.equal(config.value.defaultModel, "remote-native");
  r.api.patchStatsig(config);
  r.api.patchStatsig(config, "remote");
  assert.deepEqual([...config.value.available_models], ["remote-native"]);
});

it("hostless Statsig refresh retracts only owned additions and preserves native and external models", async () => {
  const r = runtime();
  await load(r, ["a/deleted", "native-overlap"]);
  const config = { value: { available_models: ["native", "native-overlap"],
    defaultModel: "native", model: "selected-model", reasoningEffort: "ultra" } };
  r.api.patchStatsig(config);
  assert.deepEqual([...config.value.available_models], ["native", "native-overlap", "a/deleted"]);
  config.value.available_models.push("external-update");
  const next = runtime(r.window, r.settings());
  await load(next, ["a/current"]);
  next.api.patchStatsig(config);
  assert.deepEqual([...config.value.available_models], ["native", "native-overlap", "external-update", "a/current"]);
  assert.equal(config.value.defaultModel, "native");
  assert.equal(config.value.model, "selected-model");
  assert.equal(config.value.reasoningEffort, "ultra");
  await load(next, ["a/latest"]);
  next.api.patchStatsig(config);
  assert.deepEqual([...config.value.available_models], ["native", "native-overlap", "external-update", "a/latest"]);
  const unknownOrigin = { value: { available_models: ["native", "a/deleted"] } };
  next.api.patchStatsig(unknownOrigin);
  assert.deepEqual([...unknownOrigin.value.available_models], ["native", "a/deleted", "a/latest"],
    "no host or ownership evidence permits deleting an existing native name");
});

it("Statsig can restore a local native snapshot without treating it as an owned addition", async () => {
  const r = runtime();
  await load(r, ["a/old"]);
  const config = { value: { available_models: ["native"], defaultModel: "native" } };
  r.api.patchStatsig(config, "local");
  assert.deepEqual([...config.value.available_models], ["a/old"]);
  assert.equal(config.value.defaultModel, "a/old");
  await load(r, ["a/new"]);
  r.api.patchStatsig(config);
  assert.deepEqual([...config.value.available_models], ["native", "a/new"]);
  assert.equal(config.value.defaultModel, "native");
  await load(r, ["a/last"]);
  r.api.patchStatsig(config);
  assert.deepEqual([...config.value.available_models], ["native", "a/last"]);
});

it("Statsig skips explicit remote values and clients even when local names overlap", async () => {
  const r = runtime();
  await load(r, ["shared-model"]);
  const remote = { value: { hostId: "remote", available_models: ["remote-native"], defaultModel: "remote-native" } };
  const original = structuredClone(remote);
  assert.equal(r.api.patchStatsig(remote, "local"), remote);
  assert.deepEqual(remote, original);
  const config = { value: { available_models: ["remote-native"] } };
  r.window.__STATSIG__ = { firstInstance: { hostId: "remote", getDynamicConfig: () => config } };
  r.api.installStatsig();
  assert.deepEqual(r.window.__STATSIG__.firstInstance.getDynamicConfig("107580212"), config);
  assert.deepEqual(config.value.available_models, ["remote-native"]);
});

it("Statsig upgrades unwrap a saved original getter and use the newest runtime", async () => {
  for (const version of ["3", "4"]) {
    const r = runtime();
    await load(r, ["a/old"]);
    let legacyCalls = 0;
    const config = { value: { available_models: ["native"] } };
    const client = {
      __codexPlusModelWhitelistPatched: version,
      __codexPlusModelWhitelistOriginalGet: function () { assert.equal(this, client); return config; },
      getDynamicConfig: (_name: string) => { legacyCalls++; return { value: { available_models: ["legacy-stale"] } }; },
    };
    r.window.__STATSIG__ = { firstInstance: client };
    r.api.installStatsig();
    assert.equal(legacyCalls, 0);
    assert.deepEqual([...config.value.available_models], ["native", "a/old"]);
    const next = runtime(r.window, r.settings());
    await load(next, ["a/new"]);
    next.api.installStatsig();
    assert.deepEqual([...client.getDynamicConfig("107580212").value.available_models], ["native", "a/new"]);
    assert.equal(legacyCalls, 0);
  }
});

it("a legacy Statsig wrapper without a saved getter is not relabeled as a raw getter", async () => {
  const r = runtime();
  await load(r, ["a/current"]);
  const legacy = () => ({ value: { available_models: ["native", "unowned-legacy"] } });
  const client: any = { __codexPlusModelWhitelistPatched: "3", getDynamicConfig: legacy };
  r.window.__STATSIG__ = { firstInstance: client };
  r.api.installStatsig();
  assert.equal(client.getDynamicConfig, legacy);
  assert.equal(client.__codexPlusModelWhitelistOriginalGet, undefined);
  assert.equal(client.__codexPlusModelWhitelistPatched, "3");
});
