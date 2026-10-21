import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";
import vm from "node:vm";

const sourceUrl = new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url);

async function runtime() {
  const source = await readFile(sourceUrl, "utf8");
  const names = [
    "codexPlusNativeConfigCatalogSignature", "refreshCodexNativeLocalUserConfig",
    "codexPlusLocalModelQuery", "refreshCachedLocalModelQueries", "codexPlusModelQueryCollection",
  ];
  const functions = names.map((name) => {
    const start = source.indexOf(`  function ${name}(`);
    const end = source.indexOf("\n  }\n", start) + 5;
    assert.ok(start >= 0 && end > start, name);
    return source.slice(start, end);
  }).join("\n");
  type ConfigData = { readSucceeded: boolean; response: { config: Record<string, unknown> } };
  const fresh = (): ConfigData => ({ readSucceeded: true, response: { config: {
    model_provider: "custom", model_catalog_json: "aggregate.json",
  } } });
  let read: () => Promise<ConfigData> = async () => fresh();
  const fetches: unknown[][] = [];
  const local = {
    queryKey: ["config", "user", "local"],
    options: { enabled: false, queryFn: () => read() },
    state: { data: { readSucceeded: true, response: { config: {} } } as ConfigData },
    async fetch(...args: unknown[]) {
      fetches.push(args);
      const data = await this.options.queryFn();
      this.state.data = data;
      return data;
    },
  };
  const remote = { ...local, queryKey: ["config", "user", "ssh:example"], state: { data: fresh() } };
  const unrelated = { ...local, queryKey: ["config", "analytics", "local"], state: { data: fresh() } };
  const models = ["gpt-1", "gpt-2", "gpt-3", "gpt-4", ...Array.from({ length: 8 }, (_, i) => `cpx-other/model-${i}`)];
  const modelQuery = { queryKey: ["models", "list", "local"], state: { data: { data: models.map((model) => ({ model, hidden: false })) } } };
  const queries: unknown[] = [local, remote, unrelated, modelQuery];
  const timers = new Map<number, { callback: () => void; due: number }>();
  let now = 10000, timerId = 0, enabled = true;
  const client = {
    getQueryCache: () => ({ getAll: () => queries }),
    setQueryData: () => modelQuery.state.data,
    invalidateQueries: () => undefined,
  };
  const context = vm.createContext({
    window: {
      setTimeout: (callback: () => void, ms: number) => { const id = ++timerId; timers.set(id, { callback, due: now + ms }); return id; },
      clearTimeout: (id: number) => timers.delete(id),
    },
    Date: { now: () => now },
    client, WeakMap, Promise, JSON,
    codexPlusModelUnlockEnabled: () => false,
    codexAggregateAllowedModelSet: () => enabled ? new Set(models) : null,
    findCodexQueryClient: () => client,
    installCodexPlusModelQueryPublication: () => undefined,
    patchCachedLocalModelQueryData: (value: unknown) => value,
    sendCodexPlusDiagnostic: () => undefined,
  });
  const injection = `(function () {
    const codexPlusNativeConfigRefreshRuntime = window.__codexPlusNativeConfigRefreshRuntime
      || (window.__codexPlusNativeConfigRefreshRuntime = { states: new WeakMap() });
    const codexPlusNativeConfigRefreshStates = codexPlusNativeConfigRefreshRuntime.states;
    const codexPlusModelQueryRefreshEpochs = new WeakMap();
    const codexPlusModelCollectionSnapshots = new WeakMap();
    let codexModelCatalog = { status: "ok", models: ${JSON.stringify(models)} };
    let codexModelCatalogRevision = "a", codexModelCatalogAppliedEpoch = 1;
    ${functions}
    codexPlusNativeConfigRefreshRuntime.signature = codexPlusNativeConfigCatalogSignature;
    codexPlusNativeConfigRefreshRuntime.refresh = refreshCodexNativeLocalUserConfig;
    return { refresh: refreshCachedLocalModelQueries,
       revision: (value) => { codexModelCatalogRevision = value; codexModelCatalogAppliedEpoch++; },
       poll: () => { codexModelCatalogAppliedEpoch++; },
       status: (value) => { codexModelCatalog.status = value; } };
  })()`;
  type Api = { refresh: () => boolean; revision: (value: string) => void; poll: () => void; status: (value: string) => void };
  const api = vm.runInContext(injection, context) as Api;
  // 原生 API-key observer 用 config/user 判断是否配置了自定义目录，再决定是否应用默认模型名单。
  const selectedModels = () => {
    const config = local.state.data.response.config;
    return modelQuery.state.data.data.filter((m) => !!config.model_catalog_json || !m.model.startsWith("cpx-"));
  };
  const flush = async () => { for (let i = 0; i < 16; i++) await Promise.resolve(); };
  return {
    api, local, remote, unrelated, queries, fetches, fresh, selectedModels, flush,
    reinject: () => vm.runInContext(injection, context) as Api,
    reader: (fn: typeof read) => { read = fn; },
    enabled: (value: boolean) => { enabled = value; },
    advance: async (ms: number) => {
      now += ms;
      for (const [id, timer] of [...timers]) if (timer.due <= now) { timers.delete(id); timer.callback(); }
      await flush();
    },
    timers,
  };
}

it("a disabled native user-config query is reread so all 12 aggregate models survive observer filtering", async () => {
  const r = await runtime();
  assert.equal(r.selectedModels().length, 4, "publishing 12 raw models alone leaves the stale native filter active");
  const remote = r.remote.state.data, unrelated = r.unrelated.state.data;
  r.api.refresh();
  await r.flush();
  assert.equal(r.selectedModels().length, 12);
  assert.equal(r.local.options.enabled, false, "retain native observer policy");
  assert.equal(r.fetches.length, 1);
  assert.equal(r.fetches[0][0], undefined);
  assert.deepEqual({ ...r.fetches[0][1] as object }, { cancelRefetch: true });
  assert.equal(r.remote.state.data, remote);
  assert.equal(r.unrelated.state.data, unrelated);
});

it("catalog polling and repeated scans do not reread config, but a changed revision does", async () => {
  const r = await runtime();
  r.api.refresh(); r.api.refresh();
  await r.flush();
  r.api.poll(); r.api.refresh();
  await r.flush();
  assert.equal(r.fetches.length, 1);
  r.api.revision("b"); r.api.refresh();
  await r.flush();
  assert.equal(r.fetches.length, 2);
});

it("a config query mounted after the model epoch was published still gets refreshed", async () => {
  const r = await runtime();
  r.queries.splice(r.queries.indexOf(r.local), 1);
  r.api.refresh(); await r.flush();
  assert.equal(r.fetches.length, 0);
  r.queries.push(r.local);
  r.api.refresh(); await r.flush();
  assert.equal(r.selectedModels().length, 12);
});

it("fallback and rejected reads retry with backoff and stop after native config recovers", async () => {
  const r = await runtime();
  let count = 0;
  r.reader(async () => {
    count++;
    if (count === 1) return { readSucceeded: false, response: { config: {} } };
    if (count === 2) throw new Error("temporarily unavailable");
    return r.fresh();
  });
  r.api.refresh(); await r.flush();
  assert.equal(r.selectedModels().length, 4);
  r.api.refresh(); await r.flush();
  assert.equal(count, 1);
  await r.advance(1000);
  assert.equal(count, 2);
  await r.advance(1999);
  assert.equal(count, 2);
  await r.advance(1);
  assert.equal(count, 3);
  assert.equal(r.selectedModels().length, 12);
  assert.equal(r.timers.size, 0);
});

it("switching catalog while a native read is pending requests the newest config after it settles", async () => {
  const r = await runtime();
  let resolve!: (value: ReturnType<typeof r.fresh>) => void;
  r.reader(() => new Promise((done) => { resolve = done; }));
  r.api.refresh(); await r.flush();
  r.api.revision("b"); r.api.refresh();
  assert.equal(r.fetches.length, 1);
  r.reader(async () => r.fresh());
  resolve({ readSucceeded: true, response: { config: {} } });
  await r.flush();
  assert.equal(r.fetches.length, 2);
  assert.equal(r.selectedModels().length, 12);
});

it("switching back after a failed intermediate read cannot reuse an obsolete successful marker", async () => {
  const r = await runtime();
  r.api.refresh(); await r.flush();
  let resolve!: (value: ReturnType<typeof r.fresh>) => void;
  r.reader(() => new Promise((done) => { resolve = done; }));
  r.api.revision("b"); r.api.refresh(); await r.flush();
  r.api.revision("a"); r.api.refresh();
  r.reader(async () => r.fresh());
  resolve({ readSucceeded: false, response: { config: {} } });
  await r.flush();
  assert.equal(r.fetches.length, 3);
  assert.equal(r.selectedModels().length, 12);
  assert.equal(r.timers.size, 0);
});

it("an A to B to A switch during one pending read still forces a final native read", async () => {
  const r = await runtime();
  let resolve!: (value: ReturnType<typeof r.fresh>) => void;
  r.reader(() => new Promise((done) => { resolve = done; }));
  r.api.refresh(); await r.flush();
  r.api.revision("b"); r.api.refresh();
  r.api.revision("a"); r.api.refresh();
  r.reader(async () => r.fresh());
  resolve({ readSucceeded: true, response: { config: {} } });
  await r.flush();
  assert.equal(r.fetches.length, 2);
  assert.equal(r.selectedModels().length, 12);
});

it("failure retries stop when aggregate/unlock is disabled or catalog becomes unavailable", async () => {
  const r = await runtime();
  r.reader(async () => { throw new Error("offline"); });
  r.api.refresh(); await r.flush();
  r.enabled(false);
  await r.advance(1000);
  assert.equal(r.fetches.length, 1);
  assert.equal(r.timers.size, 0);
  r.enabled(true); r.api.status("failed"); r.api.refresh(); await r.flush();
  assert.equal(r.fetches.length, 1);
});

it("reinjection reuses a pending read and follows the new runtime's catalog", async () => {
  const r = await runtime();
  let resolve!: (value: ReturnType<typeof r.fresh>) => void;
  r.reader(() => new Promise((done) => { resolve = done; }));
  r.api.refresh(); await r.flush();
  const newer = r.reinject();
  newer.revision("b"); newer.refresh(); await r.flush();
  assert.equal(r.fetches.length, 1, "reinjection must not cancel and duplicate a pending native read");
  r.reader(async () => r.fresh());
  resolve(r.fresh()); await r.flush();
  assert.equal(r.fetches.length, 2, "the old completion must use the latest runtime and catalog");
  newer.refresh(); await r.flush();
  assert.equal(r.fetches.length, 2);
});

it("reinjection retains just one failure retry chain and uses the latest enabled state", async () => {
  const r = await runtime();
  r.reader(async () => { throw new Error("offline"); });
  r.api.refresh(); await r.flush();
  const newer = r.reinject();
  newer.refresh(); await r.flush();
  assert.equal(r.fetches.length, 1);
  assert.equal(r.timers.size, 1);
  await r.advance(1000);
  assert.equal(r.fetches.length, 2);
  assert.equal(r.timers.size, 1);
  newer.status("failed");
  await r.advance(2000);
  assert.equal(r.fetches.length, 2);
  assert.equal(r.timers.size, 0);
});
