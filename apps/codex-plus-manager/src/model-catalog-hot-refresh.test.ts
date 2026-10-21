import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";
import vm from "node:vm";

const CATALOG_URL = new URL(
  "../../../assets/inject/renderer-inject/70-model-catalog.js",
  import.meta.url,
);
it("manager settings save uses only the transactional catalog publisher", async () => {
  const source = await readFile(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");
  const save = source.slice(source.indexOf("pub fn save_settings("), source.indexOf("\n#[tauri::command]", source.indexOf("pub fn save_settings(")));
  assert.match(save, /aggregate_catalog::save_with_active_catalog_refresh/);
  assert.doesNotMatch(save, /relay_config::refresh_active_model_catalog_in_home/);
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

it("string model collections withdraw models from the previous aggregate", async () => {
  const source = await readFile(CATALOG_URL, "utf8");
  const block = source.match(/  function patchModelNameArray\([^]*?\n  \}/)?.[0];
  assert.ok(block);
  const patch = vm.runInNewContext(`(() => {${block}; return patchModelNameArray;})()`, {
    stringArrayLooksPatchable: Array.isArray,
    codexAggregateAllowedModelSet: () => new Set(["cpx-current/sol"]),
    codexPureApiAllowedModelSet: () => null,
    prepareModelCollection: () => false,
    finishModelCollection: () => {},
    sameModelCollection: (left: unknown[], right: unknown[]) => JSON.stringify(left) === JSON.stringify(right),
    codexPlusModelNames: () => ["cpx-current/sol"],
  });
  const models = ["cpx-old/kimi", "cpx-current/sol"];
  assert.equal(patch(models, "local"), true);
  assert.deepEqual(models, ["cpx-current/sol"]);
});

async function catalogRuntime() {
  const source = await readFile(CATALOG_URL, "utf8");
  const start = source.indexOf("  let codexModelCatalog = ");
  const end = source.indexOf("\n  function codexPlusModelMetadata", start);
  assert.ok(start >= 0 && end > start, "model catalog loader block not found");
  const reads: Array<ReturnType<typeof deferred<Record<string, unknown>>>> = [];
  let refreshes = 0;
  const context = vm.createContext({
    window: {}, Date, WeakMap, Set, Map, Promise,
    readCodexAppServerPreparation: (path: string) => {
      assert.equal(path, "/codex-model-catalog");
      const read = deferred<Record<string, unknown>>();
      reads.push(read);
      return read.promise;
    },
    renderCodexPlusMenu: () => undefined,
    scheduleCodexModelWhitelistRefresh: () => undefined,
    sendCodexPlusDiagnostic: () => undefined,
    codexPlusModelPickerCatalogChanged: () => undefined,
    refreshCachedLocalModelQueries: () => { refreshes++; },
    captureCodexAggregateSelectedModels: () => undefined,
    codexPlusNativeConfigCatalogSignature: () => "",
    refreshCodexNativeLocalUserConfig: () => false,
  });
  const api = vm.runInContext(
    `(function () {${source.slice(start, end)}\nreturn { observeCodexModelCatalogRevision, load: loadCodexModelCatalog, state: () => ({ revision: codexModelCatalogRevision, models: [...codexModelCatalog.models], epoch: codexModelCatalogAppliedEpoch }) };})()`,
    context,
  ) as {
    observeCodexModelCatalogRevision: (revision: unknown) => boolean;
    load: (force?: boolean) => Promise<{ status: string }>;
    state: () => { revision: string; models: string[]; epoch: number };
  };
  return { api, reads, refreshes: () => refreshes };
}

it("a rejected catalog read triggers native cache restoration", async () => {
  const { api, reads, refreshes } = await catalogRuntime();
  const initial = api.load(true);
  reads[0].resolve({ status: "ok", models: ["aggregate-model"] });
  await initial;
  assert.equal(refreshes(), 1);
  const failed = api.load(true);
  reads[1].reject(new Error("catalog unavailable"));
  assert.equal((await failed).status, "failed");
  assert.equal(refreshes(), 2, "failure must publish restoration, not just update catalog state");
  assert.deepEqual(Array.from(api.state().models), []);
});

it("configured default replaces a stale native default even when both models are available", async () => {
  const source = await readFile(CATALOG_URL, "utf8");
  const block = source.match(/  function patchModelDefaults\([^]*?\n  \}/)?.[0];
  assert.ok(block);
  const patch = vm.runInNewContext(`(() => {${block}; return patchModelDefaults;})()`, {
    codexPlusModelDefaultSnapshots: new WeakMap(),
    modelFilterPolicy: (allowed: Set<string> | null) => allowed ? "configured" : "",
    codexModelCatalog: { model: "configured-model" },
    codexPlusModelDescriptor: (model: string) => ({ model }),
  });
  const config = { defaultModel: "old-model", default_model: { model: "old-model" } };
  const allowed = new Set(["old-model", "configured-model"]);
  assert.equal(patch(config, allowed), true);
  assert.equal(config.defaultModel, "configured-model");
  assert.equal(config.default_model.model, "configured-model");
  assert.equal(patch(config, allowed), false);
  assert.equal(patch(config, null), true);
  assert.equal(config.defaultModel, "old-model", "leaving the managed catalog restores native defaults");
});

it("first modelCatalogRevision refreshes once and repeated reads are deduplicated", async () => {
  const { api, reads } = await catalogRuntime();
  assert.equal(api.observeCodexModelCatalogRevision("rev-a"), true);
  assert.equal(api.observeCodexModelCatalogRevision("rev-a"), false);
  assert.equal(reads.length, 1);
  reads[0].resolve({ status: "ok", models: ["model-a"] });
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(JSON.parse(JSON.stringify(api.state())), { revision: "rev-a", models: ["model-a"], epoch: 1 });
});

it("a late catalog response cannot replace a newer revision", async () => {
  const { api, reads } = await catalogRuntime();
  api.observeCodexModelCatalogRevision("rev-a");
  api.observeCodexModelCatalogRevision("rev-b");
  assert.equal(reads.length, 2);
  reads[1].resolve({ status: "ok", models: ["model-b"] });
  await Promise.resolve();
  await Promise.resolve();
  reads[0].resolve({ status: "ok", models: ["model-a"] });
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(JSON.parse(JSON.stringify(api.state())), { revision: "rev-b", models: ["model-b"], epoch: 1 });
});

it("production settings heartbeat and save reload changed catalogs with Fast and whitelist unlock disabled", async () => {
  const catalog = await readFile(CATALOG_URL, "utf8");
  const service = await readFile(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
  const backend = await readFile(new URL("../../../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");
  const loader = catalog.slice(catalog.indexOf("  let codexModelCatalog = "), catalog.indexOf("\n  function codexPlusModelMetadata"));
  const stateLoader = service.slice(service.indexOf("  async function loadBackendSettingsState("));
  const heartbeat = backend.slice(backend.indexOf("  let syncBackendSettingsInFlight ="), backend.indexOf("\n  async function setBackendSetting("));
  const setter = backend.slice(backend.indexOf("  async function setBackendSetting("), backend.indexOf("\n  function refreshCodexPlusBackendToggles("));
  let revision = "a";
  let reads = 0;
  const context = vm.createContext({
    window: {}, Date, WeakMap, Set, Map, Promise,
    postJson: async (path: string) => {
      assert.ok(["/settings/get", "/settings/set"].includes(path));
      return { enhancementsEnabled: true, modelCatalogRevision: revision };
    },
    readCodexAppServerPreparation: async () => { reads++; return { status: "ok", models: [revision] }; },
    renderCodexPlusMenu: () => {}, scheduleCodexModelWhitelistRefresh: () => {},
    sendCodexPlusDiagnostic: () => {}, codexPlusModelPickerCatalogChanged: () => {},
    refreshCachedLocalModelQueries: () => {}, captureCodexAggregateSelectedModels: () => {},
    codexPlusNativeConfigCatalogSignature: () => "", refreshCodexNativeLocalUserConfig: () => false,
    codexPlusSettings: () => ({ modelWhitelistUnlock: false, fastMode: false }),
    runScanStep: () => {}, syncOfficialUsagePolicy: () => {}, syncCodexPlusTypingEffects: () => {},
    installCodexPlusDictation: () => {}, refreshConversationView: () => {},
    refreshCodexPlusBackendToggles: () => {},
  });
  const api = vm.runInContext(`(function () {
    let codexPlusBackendSettingsSeq = 0, codexPlusBackendSettings = {}, codexPlusBackendSettingsLoaded = false;
    ${loader}\n${stateLoader}\n${heartbeat}\n${setter}
    return { heartbeat: syncBackendSettingsFromHeartbeat, save: () => setBackendSetting("providerSyncEnabled", true), models: () => [...codexModelCatalog.models] };
  })()`, context) as { heartbeat: () => Promise<void>; save: () => Promise<void>; models: () => string[] };
  await api.heartbeat();
  await Promise.resolve();
  assert.equal(reads, 1);
  assert.deepEqual(Array.from(api.models()), ["a"]);
  await api.heartbeat();
  assert.equal(reads, 1, "unchanged settings do not refetch");
  revision = "b";
  await api.heartbeat();
  await Promise.resolve();
  assert.equal(reads, 2);
  assert.deepEqual(Array.from(api.models()), ["b"]);
  revision = "c";
  await api.save();
  await Promise.resolve();
  assert.equal(reads, 3, "saving settings reloads immediately");
  assert.deepEqual(Array.from(api.models()), ["c"]);
  await api.heartbeat();
  assert.equal(reads, 3, "the following heartbeat deduplicates the saved revision");
});

it("a heartbeat started during save cannot roll settings and catalog revision back after save succeeds", async () => {
  const catalog = await readFile(CATALOG_URL, "utf8");
  const service = await readFile(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
  const backend = await readFile(new URL("../../../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");
  const loader = catalog.slice(catalog.indexOf("  let codexModelCatalog = "), catalog.indexOf("\n  function codexPlusModelMetadata"));
  const stateLoader = service.slice(service.indexOf("  async function loadBackendSettingsState("));
  const heartbeat = backend.slice(backend.indexOf("  let syncBackendSettingsInFlight ="), backend.indexOf("\n  async function setBackendSetting("));
  const setter = backend.slice(backend.indexOf("  async function setBackendSetting("), backend.indexOf("\n  function refreshCodexPlusBackendToggles("));
  const settingReads: Array<ReturnType<typeof deferred<Record<string, unknown>>>> = [];
  const settingWrites: Array<ReturnType<typeof deferred<Record<string, unknown>>>> = [];
  const catalogReads: Array<ReturnType<typeof deferred<Record<string, unknown>>>> = [];
  const context = vm.createContext({
    window: {}, Date, WeakMap, Set, Map, Promise,
    postJson: (path: string) => {
      const request = deferred<Record<string, unknown>>();
      if (path === "/settings/get") settingReads.push(request);
      else if (path === "/settings/set") settingWrites.push(request);
      else assert.fail(`unexpected settings path: ${path}`);
      return request.promise;
    },
    readCodexAppServerPreparation: (path: string) => {
      assert.equal(path, "/codex-model-catalog");
      const request = deferred<Record<string, unknown>>();
      catalogReads.push(request);
      return request.promise;
    },
    renderCodexPlusMenu: () => {}, scheduleCodexModelWhitelistRefresh: () => {},
    sendCodexPlusDiagnostic: () => {}, codexPlusModelPickerCatalogChanged: () => {},
    refreshCachedLocalModelQueries: () => {}, captureCodexAggregateSelectedModels: () => {},
    codexPlusNativeConfigCatalogSignature: () => "", refreshCodexNativeLocalUserConfig: () => false,
    codexPlusSettings: () => ({ modelWhitelistUnlock: false, fastMode: false }),
    runScanStep: () => {}, syncOfficialUsagePolicy: () => {}, syncCodexPlusTypingEffects: () => {},
    installCodexPlusDictation: () => {}, refreshConversationView: () => {},
    refreshCodexPlusBackendToggles: () => {},
  });
  const api = vm.runInContext(`(function () {
    let codexPlusBackendSettingsSeq = 0, codexPlusBackendSettings = { providerSyncEnabled: false }, codexPlusBackendSettingsLoaded = false;
    ${loader}\n${stateLoader}\n${heartbeat}\n${setter}
    return {
      heartbeat: syncBackendSettingsFromHeartbeat,
      save: () => setBackendSetting("providerSyncEnabled", true),
      settings: () => ({ ...codexPlusBackendSettings }),
      catalogState: () => ({ revision: codexModelCatalogRevision, models: [...codexModelCatalog.models] }),
    };
  })()`, context) as {
    heartbeat: () => Promise<void>;
    save: () => Promise<void>;
    settings: () => Record<string, unknown>;
    catalogState: () => { revision: string; models: string[] };
  };

  const save = api.save();
  const heartbeatRead = api.heartbeat();
  assert.equal(settingWrites.length, 1);
  assert.equal(settingReads.length, 1);

  settingWrites[0].resolve({ providerSyncEnabled: true, enhancementsEnabled: true, modelCatalogRevision: "new" });
  await save;
  assert.equal(catalogReads.length, 1);
  catalogReads[0].resolve({ status: "ok", models: ["new"] });
  await Promise.resolve();
  await Promise.resolve();

  settingReads[0].resolve({ providerSyncEnabled: false, enhancementsEnabled: true, modelCatalogRevision: "old" });
  await heartbeatRead;
  await Promise.resolve();
  assert.equal(catalogReads.length, 1, "stale heartbeat must not observe the old revision");
  assert.equal(api.settings().providerSyncEnabled, true);
  assert.deepEqual(JSON.parse(JSON.stringify(api.catalogState())), { revision: "new", models: ["new"] });
});

it("catalog refresh publishes local model query data, invalidates it once, and preserves remote caches", async () => {
  const source = await readFile(CATALOG_URL, "utf8");
  const start = source.indexOf("  function codexPlusLocalModelQuery(");
  const end = source.indexOf("\n  function statsigClients", start);
  assert.ok(start >= 0 && end > start, "model query cache refresh block not found");
  let catalogModels = ["model-new"];
  const localKey = ["models", "list", "local", "no-auth", 100, "custom", "catalog.json"];
  const remoteKey = ["models", "list", "ssh:test", "no-auth", 100, "custom", "catalog.json"];
  const data = new Map<unknown, unknown>([
    [localKey, { data: [{ model: "model-old" }], nextCursor: null }],
    [remoteKey, { data: [{ model: "remote-model" }], nextCursor: null }],
  ]);
  const queries = [localKey, remoteKey].map((queryKey) => ({ queryKey }));
  const invalidations: unknown[] = [];
  let publications = 0;
  const snapshots = new WeakMap<object, unknown>();
  const client = {
    getQueryCache: () => ({ findAll: () => queries }),
    setQueryData: (queryKey: unknown, updater: (current: unknown) => unknown) => {
      const current = data.get(queryKey);
      const next = updater(current);
      if (next !== current) publications += 1;
      const stored = next && typeof next === "object" && !Array.isArray(next) && Array.isArray((next as { data?: unknown }).data)
        ? { ...next, data: [...(next as { data: unknown[] }).data] }
        : next;
      data.set(queryKey, stored);
      return stored;
    },
    invalidateQueries: (filter: unknown) => { invalidations.push(filter); },
  };
  const create = new Function(
    "window",
    "findCodexQueryClient",
    "codexPlusModelCollectionSnapshots",
    "patchModelArray",
    "codexPlusModelQueryRefreshEpochs",
    "codexModelCatalogAppliedEpoch",
    "sendCodexPlusDiagnostic",
    "codexPlusModelUnlockEnabled",
    "refreshCodexNativeLocalUserConfig",
    `const sameModelCollection = (a, b) => !!b && a.length === b.length && a.every((item, i) => item === b[i]);
    ${source.slice(start, end)}\nreturn { refreshCachedLocalModelQueries, setEpoch: (value) => { codexModelCatalogAppliedEpoch = value; } };`,
  ) as (...args: unknown[]) => {
    refreshCachedLocalModelQueries: () => boolean;
    setEpoch: (value: number) => void;
  };
  const api = create(
    {},
    () => client,
    snapshots,
    (models: Array<{ model: string }>, _allowEmpty: boolean, hostId: string) => {
      assert.equal(hostId, "local");
      if (!snapshots.has(models)) snapshots.set(models, { original: [...models], patched: null, policy: "test" });
      const current = models.map((item) => item.model);
      if (JSON.stringify(current) === JSON.stringify(catalogModels)) return false;
      models.splice(0, models.length, ...catalogModels.map((model) => ({ model })));
      (snapshots.get(models) as { patched: unknown }).patched = [...models];
      return true;
    },
    new WeakMap(),
    1,
    () => undefined,
    () => true,
    () => false,
  );

  assert.equal(api.refreshCachedLocalModelQueries(), true);
  assert.deepEqual(data.get(localKey), { data: [{ model: "model-new" }], nextCursor: null });
  assert.deepEqual(data.get(remoteKey), { data: [{ model: "remote-model" }], nextCursor: null });
  assert.equal(snapshots.has((data.get(localKey) as { data: object }).data), true, "snapshot follows TanStack's stored array");
  assert.equal(publications, 1);
  assert.deepEqual(invalidations, [{ queryKey: localKey, exact: true }]);
  assert.equal(api.refreshCachedLocalModelQueries(), false, "same catalog epoch must not refetch repeatedly");

  catalogModels = ["model-newer"];
  api.setEpoch(2);
  assert.equal(api.refreshCachedLocalModelQueries(), true);
  assert.deepEqual(data.get(localKey), { data: [{ model: "model-newer" }], nextCursor: null });
  assert.equal(publications, 2);
  assert.equal(invalidations.length, 2);

  catalogModels = [];
  api.setEpoch(3);
  assert.equal(api.refreshCachedLocalModelQueries(), true);
  assert.deepEqual(data.get(localKey), { data: [], nextCursor: null }, "empty catalogs must withdraw stale models");
  assert.match(source, /patchStatsigModelWhitelist\(\);\s*refreshCachedLocalModelQueries\(\);/);
  assert.doesNotMatch(source, /if \(!codexPlusModelUnlockEnabled\(\) \|\| !codexPlusModelNames\(\)\.length\) return false/);
});

// TanStack默认的JSON结构共享：深相等节点保留旧引用，变更容器重新分配。
function replaceEqualDeep<T>(previous: T, next: T): T {
  if (previous === next) return previous;
  const array = Array.isArray(previous) && Array.isArray(next);
  if (array || (previous && next && typeof previous === "object" && typeof next === "object"
      && !Array.isArray(previous) && !Array.isArray(next))) {
    const a = previous as Record<string, unknown>;
    const b = next as Record<string, unknown>;
    const keys = Object.keys(b);
    const copy: Record<string, unknown> = array ? [] as unknown as Record<string, unknown> : {};
    let equal = Object.keys(a).length === keys.length;
    for (const key of keys) {
      copy[key] = replaceEqualDeep(a[key], b[key]);
      if (copy[key] !== a[key] || !Object.hasOwn(a, key)) equal = false;
    }
    return equal ? previous : copy as T;
  }
  return next;
}

async function aggregateQueryRuntime(structuralSharing = false) {
  const source = await readFile(CATALOG_URL, "utf8");
  const names = [
    "uniqueValues", "codexPlusModelUnlockEnabled", "codexPlusModelNames",
    "codexPlusModelMetadata", "modelReasoningEfforts", "applyCodexPlusModelMetadata",
    "codexPlusModelDescriptor", "modelArrayLooksPatchable", "modelFilterPolicy",
    "sameModelCollection", "prepareModelCollection", "finishModelCollection",
    "codexPureApiAllowedModelSet", "codexAggregateAllowedModelSet", "patchModelArray",
    "codexAggregateSelectedModelTarget", "codexAggregateModelPickers",
    "captureCodexAggregateSelectedModels", "syncCodexAggregateSelectedModel",
    "codexPlusConfiguredDefaultModel", "syncCodexConfiguredDefaultModel",
    "normalizeCodexConfiguredModelRequest", "modelResponseHostId",
    "codexPlusLocalModelQuery", "cloneCodexPlusModelCollection",
    "transferCodexPlusModelCollectionSnapshot",
    "patchCachedLocalModelQueryData", "codexPlusModelQueryCollection",
    "refreshCachedLocalModelQueries", "installCodexPlusModelQueryPublication",
    "codexPlusNativeConfigCatalogSignature", "refreshCodexNativeLocalUserConfig",
    "runCodexModelWhitelistRefreshPass",
  ];
  const functions = names.map((name) => {
    const start = source.indexOf(`  function ${name}(`);
    const end = source.indexOf("\n  }\n", start) + "\n  }\n".length;
    assert.ok(start >= 0 && end > start, `${name} not found`);
    return source.slice(start, end);
  }).join("\n");
  type Model = { model: string; displayName?: string; isDefault?: boolean };
  type Catalog = {
    status: string; model: string; models: string[];
    codex_plus_default_group_id: string;
    sources: Array<{ id: string; type: string }>;
    modelMetadata: Record<string, { displayName: string; codex_plus_group_id: string }>;
  };
  class Query {
    queryKey: string[];
    data: { data: Model[] };
    constructor(queryKey: string[], data: { data: Model[] }) {
      this.queryKey = queryKey;
      this.data = data;
    }
    setData(data: { data: Model[] }) {
      this.data = structuralSharing ? replaceEqualDeep(this.data, data) : data;
      return this.data;
    }
  }
  const local = new Query(["models", "list", "local"], { data: [] });
  const remote = new Query(["models", "list", "ssh:test"], { data: [{ model: "remote" }] });
  const queries = [local, remote];
  let lateResponse: Model[] | null = null;
  const client = {
    getQueryCache: () => ({ getAll: () => queries, findAll: () => queries }),
    setQueryData: (key: string[], updater: (current: unknown) => unknown) => {
      const query = queries.find((q) => q.queryKey === key)!;
      return query.setData(updater(query.data) as { data: Model[] });
    },
    invalidateQueries: () => {
      if (lateResponse) local.setData({ data: lateResponse.map((model) => ({ ...model })) });
    },
  };
  const nodes: object[] = [];
  const runtime = new Function("client", "document", `
    const window = {};
    let codexModelCatalog = {}, codexModelCatalogAppliedEpoch = 0, codexModelCatalogRevision = "";
    let codexPlusBackendSettingsLoaded = true, unlocked = false;
    let profile = { id: "agg", relayMode: "aggregate" };
    const codexPlusModelCollectionSnapshots = new WeakMap();
    const codexPlusModelQueryRefreshEpochs = new WeakMap();
    const codexPlusNativeConfigRefreshStates = new WeakMap();
    const codexPlusNativeConfigRefreshRuntime = {};
    const codexPlusSettings = () => ({ modelWhitelistUnlock: unlocked });
    const codexRemoteSessionActiveProfile = () => profile;
    const codexServiceTierModelFromValue = value => String(value || "");
    const findCodexQueryClient = () => client;
    const sendCodexPlusDiagnostic = () => {};
    const syncCodexRetainedThreadModels = () => {};
    ${functions}
    return {
      refresh: refreshCachedLocalModelQueries,
      install: () => installCodexPlusModelQueryPublication(client),
      catalog: value => { captureCodexAggregateSelectedModels(); codexModelCatalog = value; codexModelCatalogAppliedEpoch++; },
      profile: value => { profile = value; },
      unlock: value => { unlocked = value; },
      syncSelection: syncCodexAggregateSelectedModel,
      selectionTarget: codexAggregateSelectedModelTarget,
      runRefreshPass: runCodexModelWhitelistRefreshPass,
      syncDefault: syncCodexConfiguredDefaultModel,
      normalize: normalizeCodexConfiguredModelRequest,
    };
  `)(client, { querySelectorAll: () => nodes }) as {
    refresh: () => boolean; install: () => void;
    catalog: (value: Catalog) => void;
    profile: (value: { id: string; relayMode: string }) => void;
    unlock: (value: boolean) => void;
    syncSelection: () => boolean;
    selectionTarget: (model: string, host: string) => string;
    runRefreshPass: () => boolean;
    syncDefault: () => void;
    normalize: (method: string, params: any, host: string) => any;
  };
  const catalog = (defaultProvider: "a" | "b"): Catalog => {
    const models = ["a", "b"].flatMap((provider) =>
      Array.from({ length: provider === "a" ? 4 : 7 }, (_, i) =>
        provider === defaultProvider ? `model-${i}` : `cpx-${provider}/model-${i}`));
    return {
      status: "ok", model: models[0], models,
      codex_plus_default_group_id: defaultProvider,
      sources: [{ id: "aggregate:agg", type: "aggregate_model_catalog" }],
      modelMetadata: Object.fromEntries(models.map((model) => [model, {
        displayName: `Label ${model}`, codex_plus_group_id: model.startsWith("cpx-") ? model.slice(4, 5) : defaultProvider,
      }])),
    };
  };
  return { runtime, local, remote, catalog, nodes, setLate: (models: Model[]) => { lateResponse = models; } };
}

it("aggregate default changes replace stale slugs and keep display names even with whitelist unlock disabled", async () => {
  const { runtime, local, remote, catalog, setLate } = await aggregateQueryRuntime();
  const before = catalog("a"), after = catalog("b");
  local.data = { data: before.models.map((model) => ({ model, displayName: `Old ${model}` })) };
  setLate(local.data.data);
  runtime.catalog(after);
  const remoteData = remote.data;
  assert.equal(runtime.refresh(), true);
  assert.deepEqual(local.data.data.map((m) => m.model), after.models, "late invalidation must not restore old slugs");
  assert.equal(local.data.data.filter((m) => m.model.startsWith("cpx-a/")).length, 4);
  assert.equal(local.data.data.filter((m) => !m.model.startsWith("cpx-")).length, 7);
  for (const model of local.data.data) assert.equal(model.displayName, after.modelMetadata[model.model].displayName);
  assert.equal(local.data.data.find((m) => m.model === after.model)?.displayName, `Label ${after.model}`, "selected model must resolve instead of Custom");
  assert.deepEqual(local.data.data.filter((m) => m.isDefault).map((m) => m.model), [after.model]);
  assert.equal(remote.data, remoteData);
  const installed = Object.getPrototypeOf(local).setData;
  runtime.install();
  assert.equal(Object.getPrototypeOf(local).setData, installed, "reinstallation must not stack wrappers");
  local.setData({ data: before.models.map((model) => ({ model })) });
  assert.deepEqual(local.data.data.map((m) => m.model), after.models);
  runtime.catalog(before);
  assert.equal(runtime.refresh(), true);
  assert.deepEqual(local.data.data.map((m) => m.model), before.models, "switching back must work");
  remote.setData(remoteData);
  assert.equal(remote.data, remoteData, "remote publications stay untouched");
});

it("new local composers replace an unlisted native default with a named configured model, while existing sessions stay intact", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  const current = catalog("b");
  current.model = "model-0";
  runtime.catalog(current);
  const calls: unknown[][] = [];
  const props = { model: "unconfigured-compat", reasoningEffort: "low",
    models: current.models.map(model => ({ model })),
    onSelectModel: (...args: unknown[]) => calls.push(args) };
  const owner = { permissionsHostId: "local", conversationId: "existing" };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: owner } } });
  runtime.syncDefault();
  assert.equal(calls.length, 0, "existing conversations use their saved model identity");
  owner.conversationId = "";
  runtime.syncDefault();
  assert.deepEqual(calls, [["model-0", "low"]]);
  runtime.syncDefault();
  assert.equal(calls.length, 1, "do not repeatedly select while React settles");
  props.model = "model-1";
  runtime.syncDefault();
  assert.equal(calls.length, 1, "an explicit available model remains selected");
});

it("pool catalogs withdraw aggregate prefixes and unconfigured compat models, preserve member order, and normalize actual requests", async () => {
  const { runtime, local, catalog } = await aggregateQueryRuntime();
  runtime.catalog(catalog("a"));
  local.data = { data: [{ model: "cpx-a/model-0" }, { model: "unconfigured-compat" }] };
  const pool = { ...catalog("b"), codex_plus_default_group_id: "",
    model: "model-0", models: ["model-1", "model-0"],
    sources: [{ id: "aggregate:agg", type: "aggregate_pool_catalog" }] };
  runtime.catalog(pool);
  runtime.refresh();
  assert.deepEqual(local.data.data.map(m => m.model), ["model-1", "model-0"]);
  assert.deepEqual(local.data.data.filter(m => m.isDefault).map(m => m.model), ["model-0"]);
  const params = { threadId: "one", model: "cpx-a/model-0", effort: "high",
    collaborationMode: { settings: { model: "cpx-a/model-0", reasoning_effort: "high" } } };
  const next = runtime.normalize("turn/start", params, "local");
  assert.equal(next.model, "model-0");
  assert.equal(next.collaborationMode.settings.model, "model-0");
  assert.equal(next.effort, "high");
  assert.equal(params.model, "cpx-a/model-0", "do not mutate the saved request");
  assert.equal(runtime.normalize("turn/start", params, "ssh:test"), params);
  assert.equal(runtime.normalize("thread/start", { model: "unconfigured-compat" }, "local").model, "model-0");
  assert.equal(runtime.normalize("thread/start", {}, "local").model, "model-0");
  assert.equal(runtime.normalize("send-cli-request-for-host", { hostId: "ssh:test", method: "turn/start", params }, "local").params, params);
});

it("aggregate projection requires a successful catalog owned by the active profile", async () => {
  const { runtime, local, catalog } = await aggregateQueryRuntime();
  const original = { data: [{ model: "native" }] };
  local.data = original;
  const wrong = catalog("b");
  wrong.sources[0].id = "aggregate:other";
  runtime.catalog(wrong);
  assert.equal(runtime.refresh(), false);
  assert.equal(local.data, original);
  runtime.catalog({ ...catalog("b"), status: "failed" });
  assert.equal(runtime.refresh(), false);
  runtime.catalog(catalog("b"));
  runtime.profile({ id: "agg", relayMode: "pureApi" });
  assert.equal(runtime.refresh(), false, "pure API still respects the whitelist switch");
});

it("an empty authoritative aggregate catalog withdraws all stale models", async () => {
  const { runtime, local, catalog } = await aggregateQueryRuntime();
  local.data = { data: [{ model: "old" }] };
  runtime.catalog({ ...catalog("b"), model: "", models: [], modelMetadata: {} });
  assert.equal(runtime.refresh(), true);
  assert.deepEqual(local.data.data, []);
});

for (const structuralSharing of [false, true]) for (const late of [false, true]) for (const exit of ["official", "failed", "unconfigured"]) {
  it(`native models are restored after aggregate exit (sharing=${structuralSharing}, late=${late}, exit=${exit})`, async () => {
    const { runtime, local, remote, catalog } = await aggregateQueryRuntime(structuralSharing);
    const native = [{ model: "native-one" }, { model: "native-two" }];
    local.data = { data: native.map((model) => ({ ...model })) };
    const remoteData = remote.data;
    runtime.catalog(catalog("a"));
    runtime.refresh();
    assert.equal(local.data.data.length, 11);
    runtime.catalog(catalog("b"));
    runtime.refresh();
    if (late) local.setData({ data: native.map((model) => ({ ...model })) });
    if (exit === "official") runtime.profile({ id: "official", relayMode: "official" });
    runtime.catalog({ ...catalog("b"), status: exit === "unconfigured" ? "not_configured" : "failed", model: "", models: [], modelMetadata: {}, sources: [] });
    assert.equal(runtime.refresh(), true);
    assert.deepEqual(local.data.data, native);
    assert.equal(remote.data, remoteData);
    assert.equal(runtime.refresh(), false);
  });
}

for (const sharing of [false, true]) {
  it(`aggregate exit restores native metadata even for overlapping model IDs (sharing=${sharing})`, async () => {
    const { runtime, local, catalog } = await aggregateQueryRuntime(sharing);
    const native = [{
      model: "model-0", displayName: "Native Original", hidden: true,
      isDefault: false, description: "Native description",
    }, { model: "native-other", displayName: "Other native" }];
    local.data = { data: native.map((model) => ({ ...model })) };
    runtime.catalog(catalog("a"));
    runtime.refresh();
    assert.equal(local.data.data[0].displayName, "Label model-0");
    runtime.catalog(catalog("b"));
    runtime.refresh();
    runtime.profile({ id: "official", relayMode: "official" });
    runtime.catalog({ ...catalog("b"), status: "not_configured", model: "", models: [], modelMetadata: {}, sources: [] });
    assert.equal(runtime.refresh(), true);
    assert.deepEqual(local.data.data, native);
  });
}
it("unpublished aggregate edits withdraw deleted models from local queries and late responses without adding new models", async () => {
  const { runtime, local, remote, catalog, nodes, setLate } = await aggregateQueryRuntime();
  const before = catalog("a");
  local.data = { data: before.models.map((model) => ({ model, isDefault: model === before.model })) };
  runtime.catalog(before);
  runtime.refresh();
  const deleted = before.models.at(-1)!;
  const retainedDefault = local.data.data.find((model) => model.isDefault)!.model;
  const stale = local.data.data.map((model) => ({ ...model }));
  setLate(stale);
  const remoteData = remote.data;
  const after = { ...before, status: "not_applied", model: "new-unpublished",
    models: ["new-unpublished", ...before.models.filter((model) => model !== deleted)] };
  const selections: unknown[][] = [];
  nodes.push({ __reactFiber$test: { memoizedProps: { model: deleted, reasoningEffort: "high",
    models: stale, onSelectModel: (...args: unknown[]) => selections.push(args) },
    return: { memoizedProps: { permissionsHostId: "local" } } } });
  runtime.catalog(after);
  assert.equal(runtime.refresh(), true);
  const expected = after.models.filter((model) => model !== "new-unpublished");
  assert.deepEqual(local.data.data.map((model) => model.model), expected, "late response cannot reintroduce the deleted model");
  assert.deepEqual(local.data.data.filter((model) => model.isDefault).map((model) => model.model), [retainedDefault],
    "unpublished edits cannot rewrite native defaults");
  assert.equal(remote.data, remoteData);
  runtime.syncDefault();
  runtime.syncSelection();
  assert.deepEqual(selections, []);
  const params = { model: deleted };
  assert.equal(runtime.normalize("thread/start", params, "local"), params);
  local.setData({ data: stale });
  assert.deepEqual(local.data.data.map((model) => model.model), expected);
  runtime.unlock(true);
  runtime.catalog(after);
  runtime.refresh();
  assert.deepEqual(local.data.data.map((model) => model.model), expected,
    "whitelist unlock must not add unavailable unpublished models either");
  runtime.catalog({ ...after, models: [], model: "" });
  runtime.refresh();
  assert.deepEqual(local.data.data, [], "deleting the last model must also withdraw stale rows");
});

it("unpublished deletion projection requires current aggregate identity and a concrete models slice", async () => {
  const { runtime, local, catalog } = await aggregateQueryRuntime();
  const original = { data: [{ model: "native" }] };
  local.data = original;
  const desired = { ...catalog("b"), status: "not_applied" };
  desired.sources[0].id = "aggregate:other";
  runtime.catalog(desired);
  assert.equal(runtime.refresh(), false);
  assert.equal(local.data, original);
  const missing = { ...catalog("b"), status: "not_applied" };
  Reflect.deleteProperty(missing, "models");
  runtime.catalog(missing);
  assert.equal(runtime.refresh(), false, "a missing model list is not an empty authoritative catalog");
  assert.equal(local.data, original);
  runtime.catalog({ ...catalog("b"), status: "failed" });
  assert.equal(runtime.refresh(), false);
  assert.equal(local.data, original);
});

it("a newly default member's retired prefix migrates through the native picker without changing effort", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  runtime.catalog(catalog("a"));
  const selections: unknown[][] = [];
  const props = {
    model: "cpx-a/model-0", reasoningEffort: "high",
    models: catalog("a").models.map((model) => ({ model })),
    onSelectModel: (...args: unknown[]) => { selections.push(args); },
  };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: { permissionsHostId: "local", conversationId: "one" } } } });
  runtime.refresh();
  runtime.runRefreshPass();
  assert.deepEqual(selections, [["model-0", "high"]]);
  assert.equal(runtime.syncSelection(), false, "same pending selection is not repeated");
  props.model = "model-0";
  assert.equal(runtime.syncSelection(), false, "a canonical ID remains unchanged");
  runtime.catalog(catalog("b"));
  props.models = catalog("b").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), true, "the previous default keeps its provider");
  assert.deepEqual(selections.at(-1), ["cpx-a/model-0", "high"]);
  props.model = "cpx-b/model-2";
  props.models = catalog("b").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), true);
  assert.deepEqual(selections.at(-1), ["model-2", "high"]);
});

it("selection migration requires the same provider, current models, local host and an unlocked picker", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  const current = catalog("a");
  runtime.catalog(current);
  assert.equal(runtime.selectionTarget("cpx-b/model-0", "local"), "", "other member stays explicitly routed");
  assert.equal(runtime.selectionTarget("cpx-other/model-0", "local"), "", "removed member is not guessed");
  assert.equal(runtime.selectionTarget("cpx-a/missing", "local"), "", "unavailable model is not replaced");
  assert.equal(runtime.selectionTarget("cpx-a/model-0", "ssh:test"), "", "remote catalog is not local");
  current.modelMetadata["model-0"].codex_plus_group_id = "b";
  assert.equal(runtime.selectionTarget("cpx-a/model-0", "local"), "", "bare metadata must confirm ownership");
  current.modelMetadata["model-0"].codex_plus_group_id = "a";
  let calls = 0;
  const props = {
    model: "cpx-a/model-0", reasoningEffort: "ultra",
    models: [] as Array<{ model: string }>, lockedModelSlug: "",
    modelOptionsDisabled: false, modelSelectionDisabled: false,
    onSelectModel: () => { calls++; },
  };
  const host = { permissionsHostId: "local" };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: host } } });
  assert.equal(runtime.syncSelection(), false, "wait for React's model list");
  props.models = current.models.map((model) => ({ model }));
  host.permissionsHostId = "ssh:test";
  assert.equal(runtime.syncSelection(), false);
  host.permissionsHostId = "";
  assert.equal(runtime.syncSelection(), false, "unknown host is not guessed");
  host.permissionsHostId = "local";
  props.lockedModelSlug = props.model;
  assert.equal(runtime.syncSelection(), false);
  props.lockedModelSlug = "";
  props.modelOptionsDisabled = true;
  assert.equal(runtime.syncSelection(), false);
  props.modelOptionsDisabled = false;
  props.modelSelectionDisabled = true;
  assert.equal(runtime.syncSelection(), false);
  props.modelSelectionDisabled = false;
  assert.equal(runtime.syncSelection(), true);
  assert.equal(calls, 1);
});

it("default changes preserve provider identity through stale lists, rerenders and repeated switches", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  runtime.catalog(catalog("a"));
  const selections: unknown[][] = [];
  const props = {
    model: "model-0", reasoningEffort: "ultra",
    models: catalog("a").models.map((model) => ({ model })),
    onSelectModel: (...args: unknown[]) => { selections.push(args); },
  };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: { permissionsHostId: "local", conversationId: "thread-a" } } } });
  runtime.catalog(catalog("b"));
  assert.equal(runtime.syncSelection(), false, "wait until the target exists in React's list");
  props.onSelectModel = (...args: unknown[]) => { selections.push(args); };
  runtime.catalog(catalog("b"));
  props.models = catalog("b").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), true, "a rerender must not lose the old provider");
  assert.deepEqual(selections.at(-1), ["cpx-a/model-0", "ultra"]);
  props.model = "cpx-a/model-0";
  assert.equal(runtime.syncSelection(), false);
  runtime.catalog(catalog("a"));
  props.models = catalog("a").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), true);
  assert.deepEqual(selections.at(-1), ["model-0", "ultra"]);
  props.model = "model-0";
  runtime.syncSelection();
  props.model = "cpx-b/model-2";
  runtime.catalog(catalog("b"));
  props.models = catalog("b").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), true);
  assert.deepEqual(selections.at(-1), ["model-2", "ultra"], "an explicit new selection uses its own provider");
});

it("removed models and reused composer nodes cannot reroute a selection to another provider", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  runtime.catalog(catalog("a"));
  const selections: unknown[][] = [];
  const props = {
    model: "model-0", reasoningEffort: "high",
    models: catalog("a").models.map((model) => ({ model })),
    onSelectModel: (...args: unknown[]) => { selections.push(args); },
  };
  const owner = { permissionsHostId: "local", conversationId: "one" };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: owner } } });
  const after = catalog("b");
  after.models = after.models.filter((model) => model !== "cpx-a/model-0");
  runtime.catalog(after);
  props.models = after.models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), false, "do not substitute the new default's same-name model");
  owner.conversationId = "two";
  runtime.catalog(catalog("b"));
  props.models = catalog("b").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), false, "a reused DOM node is a different session");
  assert.deepEqual(selections, []);
});

it("conversation changes before a pending migration cannot inherit the old supplier", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  runtime.catalog(catalog("a"));
  const selections: unknown[][] = [];
  const props = {
    model: "model-0", reasoningEffort: "high",
    models: catalog("a").models.map((model) => ({ model })),
    onSelectModel: (...args: unknown[]) => { selections.push(args); },
  };
  const owner = { permissionsHostId: "local", conversationId: "one" };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: owner } } });
  runtime.catalog(catalog("b"));
  owner.conversationId = "two";
  props.models = catalog("b").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), false);
  assert.deepEqual(selections, []);
});

it("the host conversationId takes priority over a stale nearer threadId", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  runtime.catalog(catalog("a"));
  const selections: unknown[][] = [];
  const props = {
    threadId: "stale-thread", model: "model-0", reasoningEffort: "high",
    models: catalog("a").models.map((model) => ({ model })),
    onSelectModel: (...args: unknown[]) => { selections.push(args); },
  };
  const owner = { permissionsHostId: "local", conversationId: "one" };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: owner } } });
  runtime.catalog(catalog("b"));
  owner.conversationId = "two";
  props.models = catalog("b").models.map((model) => ({ model }));
  assert.equal(runtime.syncSelection(), false);
  assert.deepEqual(selections, []);
});

it("a reused remote or unidentified picker clears a pending local supplier identity", async () => {
  for (const kind of ["remote", "unknown-conversation"]) {
    const { runtime, catalog, nodes } = await aggregateQueryRuntime();
    runtime.catalog(catalog("a"));
    const selections: unknown[][] = [];
    const props = {
      model: "model-0", reasoningEffort: "high",
      models: catalog("a").models.map((model) => ({ model })),
      onSelectModel: (...args: unknown[]) => { selections.push(args); },
    };
    const owner = { permissionsHostId: "local", conversationId: "one" };
    nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: owner } } });
    runtime.catalog(catalog("b"));
    if (kind === "remote") owner.permissionsHostId = "ssh:test";
    else owner.conversationId = "";
    runtime.catalog(catalog("b"));
    assert.equal(runtime.syncSelection(), false);
    owner.permissionsHostId = "local";
    owner.conversationId = "one";
    props.models = catalog("b").models.map((model) => ({ model }));
    assert.equal(runtime.syncSelection(), false);
    assert.deepEqual(selections, []);
  }
});

for (const sharing of [false, true]) for (const late of [false, true]) {
  it(`overlapping native model metadata survives aggregate refresh and exit (sharing=${sharing}, late=${late})`, async () => {
    const { runtime, local, catalog } = await aggregateQueryRuntime(sharing);
    const original = [{ model: "model-0", displayName: "Native display", hidden: true, isDefault: false,
      defaultReasoningEffort: "low", supportedReasoningEfforts: [{ reasoningEffort: "low" }], contextWindow: 100 }];
    local.data = { data: original.map(item => ({ ...item })) };
    for (const provider of ["a", "b", "a"] as const) {
      const next = catalog(provider);
      if (next.modelMetadata["model-0"]) Object.assign(next.modelMetadata["model-0"], {
        defaultReasoningEffort: "high", contextWindow: 200,
        supportedReasoningEfforts: [{ reasoningEffort: "high" }],
      });
      runtime.catalog(next);
      runtime.refresh();
    }
    if (late) local.setData({ data: original.map(item => ({ ...item })) });
    runtime.profile({ id: "official", relayMode: "official" });
    runtime.catalog({ ...catalog("a"), status: "failed", model: "", models: [], modelMetadata: {}, sources: [] });
    runtime.refresh();
    assert.deepEqual(local.data.data, original);
    assert.equal(runtime.refresh(), false);
  });
}

it("callback reuse across conversations does not suppress ID migration", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  runtime.catalog(catalog("a"));
  const selections: unknown[][] = [];
  const props = {
    model: "cpx-a/model-0", reasoningEffort: "high",
    models: catalog("a").models.map(model => ({ model })),
    onSelectModel: (...args: unknown[]) => { selections.push(args); },
  };
  const owner = { permissionsHostId: "local", conversationId: "one" };
  nodes.push({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: owner } } });
  assert.equal(runtime.syncSelection(), true);
  assert.equal(runtime.syncSelection(), false, "one pending migration per conversation");
  owner.conversationId = "two";
  assert.equal(runtime.syncSelection(), true);
  assert.equal(runtime.syncSelection(), false, "the second conversation also deduplicates");
  assert.deepEqual(selections, [["model-0", "high"], ["model-0", "high"]]);
});

it("migration attempts are scoped to picker nodes, profiles and host lifetimes", async () => {
  const { runtime, catalog, nodes } = await aggregateQueryRuntime();
  const current = catalog("a");
  runtime.catalog(current);
  let calls = 0;
  const props = {
    model: "cpx-a/model-0", reasoningEffort: "high",
    models: current.models.map(model => ({ model })),
    onSelectModel: () => { calls++; },
  };
  const owner = { permissionsHostId: "local", conversationId: "one" };
  const picker = () => ({ __reactFiber$test: { memoizedProps: props, return: { memoizedProps: owner } } });
  nodes.push(picker());
  assert.equal(runtime.syncSelection(), true);
  assert.equal(runtime.syncSelection(), false);
  nodes.push(picker());
  assert.equal(runtime.syncSelection(), true, "a separate picker is not suppressed by a shared callback");
  assert.equal(calls, 2);
  current.sources[0].id = "aggregate:other";
  runtime.profile({ id: "other", relayMode: "aggregate" });
  assert.equal(runtime.syncSelection(), true, "a different profile has its own attempt state");
  assert.equal(calls, 4);
  owner.permissionsHostId = "ssh:test";
  assert.equal(runtime.syncSelection(), false);
  owner.permissionsHostId = "local";
  assert.equal(runtime.syncSelection(), true, "remote reuse clears old local attempts");
  assert.equal(calls, 6);
});
