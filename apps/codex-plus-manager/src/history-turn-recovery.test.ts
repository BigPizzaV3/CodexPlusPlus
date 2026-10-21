import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";

const fragments = path.resolve(import.meta.dirname, "../../../assets/inject/renderer-inject");
const source = readFileSync(path.join(fragments, "85-history-turn-recovery.js"), "utf8");
const nativeSource = "var defaultManager=derive(AppScope,({get:g})=>{let m=g(rawManagers,localHost);if(m==null)throw Error(`Missing default AppServerManager`);return m});export{rawManagers as managerToken};";
const uuid = (n: number) => `10000000-0000-0000-0000-${String(n).padStart(12, "0")}`;
const threadId = uuid(999);

function turn(n: number, loaded = false) {
  return {
    turnId: uuid(n), status: "completed", items: loaded ? [{ type: "agentMessage" }] : [],
    params: { threadId, input: [] },
    itemsPagination: { hasLoadedOldest: loaded, isLoadingOlder: false, olderCursor: null },
  };
}

function conversation(groups = [[turn(1, true), turn(2), turn(3, true)]]) {
  const entitiesByKey = Object.fromEntries(groups.flat().map((entry) => [entry.turnId, entry]));
  return {
    resumeState: "resumed", requests: [], threadRuntimeStatus: { type: "idle" },
    turnsPagination: { isLoadingOlder: false },
    turnHistory: {
      kind: "canonical",
      history: { entitiesByKey, islands: groups.map((turns) => ({ entries: turns.map((t) => ({ value: t.turnId })) })) },
    },
  };
}

function harness(options: { enabled?: boolean; data?: ReturnType<typeof conversation> } = {}) {
  let enabled = options.enabled ?? true;
  let data = options.data ?? conversation();
  let currentId = threadId;
  let host = "local";
  let now = 100000;
  let discoverySource = nativeSource;
  let fetches = 0;
  let scopeReads = 0;
  let nextTimer = 0;
  const timers = new Map<number, { at: number; fn: () => unknown }>();
  const calls: unknown[][] = [];
  const notices: string[] = [];
  const signal = { kind: "signal-family", scope: { __scopeBrand: "AppScope" }, resolve() {}, read() {} };
  let load: (...args: unknown[]) => Promise<void> = async (_thread, id) => {
    const entry = data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "userMessage" });
    entry.itemsPagination.hasLoadedOldest = true;
  };
  const manager = {
    hostId: "local", getHostId: () => "local",
    getConversation: (id: string) => id === currentId ? data : null,
    pagination: { itemPagination: { loadTurnItems: (...args: unknown[]) => { calls.push(args); return load(...args); } } },
  };
  const scope = {
    scope: signal.scope,
    node: { familyBindings: new Map([[signal, new Map([["local", manager]])]]) },
    get(key: unknown, h: string) {
      assert.equal(key, signal);
      assert.equal(h, "local");
      scopeReads++;
      return manager;
    },
  };
  const marker = {
    getAttribute: () => `local:${currentId}`,
    __reactFiber$fixture: { memoizedState: { memoizedState: { current: scope }, next: null }, return: null },
  };
  const document = {
    hidden: false, documentElement: { lang: "zh-CN" },
    querySelector: (selector: string) => selector.includes("data-above-composer-conversation-id") ? marker : null,
    getElementById: () => null,
  };
  const window = {
    location: { search: "" },
    setTimeout(fn: () => unknown, ms: number) { const id = ++nextTimer; timers.set(id, { at: now + ms, fn }); return id; },
    clearTimeout(id: number) { timers.delete(id); },
  };
  const context = vm.createContext({
    window, document, Map, URL, AbortController,
    navigator: { language: "zh-CN" }, Date: { now: () => now },
    codexPlusSettings: () => ({ historyTurnRecovery: enabled }),
    currentSessionRef: () => ({ session_id: currentId, host_id: host }),
    locationThreadId: () => "",
    normalizedCodexThreadUuid: (id: unknown) => typeof id === "string" && /^[a-f\d]{8}(?:-[a-f\d]{4}){3}-[a-f\d]{12}$/i.test(id) ? id : "",
    codexAppAssetUrl: () => "app://-/assets/app-initial-fixture.js",
    codexAppAssetUrlFromScriptText: async () => "",
    loadOptionalCodexAppModule: async () => ({ managerToken: signal }),
    fetch: async () => { fetches++; return { ok: true, text: async () => discoverySource }; },
    showToast: (text: string) => notices.push(text),
  });
  function inject() {
    vm.runInContext(`(() => { ${source}
      globalThis.testApi = {
        tick: historyTurnRecoveryTick, sync: syncCodexPlusHistoryTurnRecovery,
        signal: historyTurnRecoverySignal, islands: historyTurnRecoveryIslands,
        busy: historyTurnRecoveryBusy, candidates: historyTurnRecoveryCandidates,
        state: historyTurnRecoveryState, ledger: historyTurnRecoveryLedger,
      };
    })();`, context);
  }
  inject();
  return {
    context, signal, scope, manager, marker, document, window, calls, notices, timers,
    get api() { return context.testApi; },
    get fetches() { return fetches; }, get scopeReads() { return scopeReads; },
    get data() { return data; }, set data(value) { data = value; },
    enable(value: boolean) { enabled = value; context.testApi.sync(); },
    navigate(id: string, h = "local") { currentId = id; host = h; },
    setLoader(fn: typeof load) { load = fn; },
    setDiscovery(value: string) { discoverySource = value; },
    elapse(ms: number) { now += ms; },
    inject,
    tick() {
      window.clearTimeout(context.testApi.state.timer);
      return context.testApi.tick();
    },
    fireNextTimer() {
      const [id, timer] = [...timers].sort((a, b) => a[1].at - b[1].at)[0];
      now = timer.at;
      timers.delete(id);
      return timer.fn();
    },
  };
}

test("history recovery is opt-in and both settings surfaces share its backend key", () => {
  const style = readFileSync(path.join(fragments, "10-style.js"), "utf8");
  const defaults = style.slice(style.indexOf("  function defaultCodexPlusSettings()"), style.indexOf("  const codexPlusBackendSettingMap"));
  const result = new Function("conversationViewDefaultWidth", "window", `${defaults};return defaultCodexPlusSettings();`)(800, {});
  assert.equal(result.historyTurnRecovery, false);
  assert.match(style, /historyTurnRecovery: "codexAppHistoryTurnRecovery"/);
  assert.match(style.slice(style.indexOf("if (codexPlusBackendSettings.enhancementsEnabled === false)")), /historyTurnRecovery: false/);
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  assert.match(app, /checked=\{form\.codexAppHistoryTurnRecovery\} disabled=\{!masterEnabled\} onChange=\{\(value\) => setEnhanceFlag\("codexAppHistoryTurnRecovery", value\)\}/);
  const backend = readFileSync(path.join(fragments, "40-backend-settings.js"), "utf8");
  assert.match(backend, /data-codex-plus-setting="historyTurnRecovery"/);
  assert.match(backend, /runScanStep\(syncCodexPlusHistoryTurnRecovery\)/);
});

test("discovers only the uniquely exported raw manager family, without executing source", () => {
  const h = harness();
  assert.equal(h.api.signal({ managerToken: h.signal }, nativeSource), h.signal);
  const renamed = nativeSource.replaceAll("rawManagers", "$family").replaceAll("managerToken", "renamed");
  assert.equal(h.api.signal({ renamed: h.signal }, renamed), h.signal);
  assert.equal(h.api.signal({ managerToken: h.signal }, `${nativeSource};${nativeSource}`), null);
  assert.equal(h.api.signal({}, nativeSource), null);
  assert.equal(h.api.signal({ managerToken: { ...h.signal, kind: "readable-family" } }, nativeSource), null);
  assert.equal(h.api.signal({ managerToken: h.signal }, "export{rawManagers as managerToken};"), null);
  assert.equal(h.scopeReads, 0);
});

test("disabled recovery performs no discovery, scope reads, or history loads", async () => {
  const h = harness({ enabled: false });
  await h.tick();
  assert.equal(h.fetches, 0);
  assert.equal(h.scopeReads, 0);
  assert.equal(h.calls.length, 0);
  assert.equal(h.timers.size, 0);
});

test("restores three interior empty turns through the native loader, serially and once each", async () => {
  const h = harness({ data: conversation([[turn(1, true), turn(2), turn(3), turn(4), turn(5, true)]]) });
  for (let i = 0; i < 5; i++) await h.tick();
  assert.equal(h.calls.length, 3);
  assert.deepEqual(h.calls.map((args) => args.slice(0, 3)), [2, 3, 4].map((n) => [threadId, uuid(n), false]));
  for (const args of h.calls) assert.deepEqual(JSON.parse(JSON.stringify(args[3])), { priority: "interactive", source: "thread_hydration" });
  assert.equal(h.fetches, 1);
  assert.equal(h.notices.length, 0);
});

test("does not bridge different islands or load leading/trailing pagination boundaries", async () => {
  const h = harness({ data: conversation([[turn(1), turn(2, true), turn(3)], [turn(4), turn(5, true), turn(6)]]) });
  await h.tick();
  assert.equal(h.calls.length, 0);
});

test("loaded input, intentionally evicted items, reconnects, and proven empty turns are not gaps", async () => {
  for (const change of [
    (t: any) => { t.params.input.push({ type: "text", text: "fixture" }); },
    (t: any) => { t.itemsPagination.evicted = true; },
    (t: any) => { t.itemsPagination.reconnect = {}; },
    (t: any) => { t.itemsPagination.hasLoadedOldest = true; },
  ]) {
    const h = harness(); change(h.data.turnHistory.history.entitiesByKey[uuid(2)]);
    await h.tick();
    assert.equal(h.calls.length, 0);
  }
});

test("yields to generation, submissions, requests, resume, and any native history load", async () => {
  for (const change of [
    (c: any) => { c.resumeState = "resuming"; },
    (c: any) => { c.threadRuntimeStatus.type = "active"; },
    (c: any) => { c.unconfirmedTurnSubmissions = [{ terminal: false }]; },
    (c: any) => { c.requests = [{}]; },
    (c: any) => { c.turnsPagination.isLoadingOlder = true; },
    (c: any) => { c.turnHistory.history.entitiesByKey[uuid(1)].itemsPagination.isLoadingOlder = true; },
    (c: any) => { c.turnHistory.history.entitiesByKey[uuid(3)].status = "inProgress"; },
  ]) {
    const h = harness(); change(h.data);
    await h.tick(); assert.equal(h.calls.length, 0);
  }
});

test("only canonical, internally consistent histories are inspected", async () => {
  for (const change of [
    (c: any) => { c.turnHistory.kind = "legacy"; },
    (c: any) => { delete c.turnHistory.history.entitiesByKey[uuid(2)]; },
    (c: any) => { c.turnHistory.history.islands[0].entries.push({ value: uuid(2) }); },
    (c: any) => { delete c.turnHistory.history.entitiesByKey[uuid(2)].params.input; },
  ]) {
    const h = harness(); change(h.data);
    await h.tick(); assert.equal(h.calls.length, 0);
  }
});

test("refuses to instantiate an unbound manager family or use a mismatched host", async () => {
  const h = harness();
  h.scope.node.familyBindings.clear();
  await h.tick();
  assert.equal(h.scopeReads, 0);
  assert.equal(h.calls.length, 0);
  h.scope.node.familyBindings.set(h.signal, new Map([["local", h.manager]]));
  h.manager.hostId = "remote";
  await h.tick();
  assert.equal(h.calls.length, 0);
});

test("ignores remote, hidden and auxiliary windows", async () => {
  for (const change of [
    (h: ReturnType<typeof harness>) => h.navigate(threadId, "remote"),
    (h: ReturnType<typeof harness>) => { h.document.hidden = true; },
    (h: ReturnType<typeof harness>) => { h.window.location.search = "?initialRoute=thread"; },
  ]) {
    const h = harness(); change(h);
    await h.tick(); assert.equal(h.fetches, 0); assert.equal(h.calls.length, 0);
  }
});

test("memory-router pages work without a URL thread ID, but a stale selected row cannot load another composer", async () => {
  const h = harness();
  h.marker.getAttribute = () => `local:${uuid(888)}`;
  await h.tick();
  assert.equal(h.calls.length, 0);
  h.marker.getAttribute = () => `local:${threadId}`;
  await h.tick();
  assert.equal(h.calls.length, 1);
});

test("switching to another conversation during discovery does not load the previous one", async () => {
  const h = harness();
  const pending = h.tick();
  h.navigate(uuid(888));
  await pending;
  assert.equal(h.calls.length, 0);
});

test("a rejection or no-progress resolve retries after cooldown then stops with one warning", async () => {
  for (const reject of [false, true]) {
    const h = harness();
    h.setLoader(async () => { if (reject) throw new Error("private fixture error must not be displayed"); });
    await h.tick();
    await h.tick(); assert.equal(h.calls.length, 1);
    h.elapse(30000); await h.tick();
    for (let i = 0; i < 5; i++) { h.elapse(30000); await h.tick(); }
    assert.equal(h.calls.length, 2);
    assert.equal(h.notices.length, 1);
    assert.doesNotMatch(h.notices[0], /private fixture/);
    h.enable(false); h.enable(true); await h.tick();
    assert.equal(h.calls.length, 3);
  }
});

test("unknown native API has bounded discovery and heartbeat cannot restart its retry loop", async () => {
  const h = harness(); h.setDiscovery("export{};");
  for (let i = 0; i < 8; i++) { await h.tick(); h.elapse(30000); h.api.sync(); }
  assert.equal(h.fetches, 3);
  assert.equal(h.notices.length, 1);
  assert.equal(h.calls.length, 0);
  assert.equal(h.api.state.timer, null);
});

test("navigation, disable and reinjection never add a concurrent native loader", async () => {
  const h = harness();
  let resolve!: () => void;
  h.setLoader(() => new Promise<void>((done) => { resolve = done; }));
  const first = h.tick();
  await new Promise(setImmediate);
  assert.equal(h.calls.length, 1);
  const old = h.api;
  h.inject();
  assert.equal(old.state.disposed, true);
  await h.tick();
  assert.equal(h.calls.length, 1);
  h.enable(false);
  resolve();
  await first;
  assert.equal(h.api.ledger.flight, null);
  assert.equal(h.api.state.timer, null);
  assert.equal(h.calls.length, 1);
});

test("reinjecting the renderer preserves exhausted attempts", async () => {
  const h = harness(); h.setLoader(async () => {});
  await h.tick(); h.elapse(30000); await h.tick();
  h.inject(); h.elapse(30000); await h.tick();
  assert.equal(h.calls.length, 2);
});

test("switching A to B to A preserves exhausted attempts", async () => {
  const h = harness(); h.setLoader(async () => {});
  const original = h.data;
  await h.tick(); h.elapse(30000); await h.tick();
  h.navigate(uuid(888));
  h.data = conversation([[turn(88, true)]]);
  await h.tick();
  h.navigate(threadId); h.data = original;
  h.elapse(30000); await h.tick();
  assert.equal(h.calls.length, 2);
  assert.equal(h.notices.length, 1);
});

test("heartbeat during a slow scheduled load cannot multiply polling timers", async () => {
  const h = harness();
  let resolve!: () => void;
  h.setLoader(() => new Promise<void>((done) => { resolve = done; }));
  const pending = h.fireNextTimer();
  await new Promise(setImmediate);
  assert.equal(h.calls.length, 1);
  for (let i = 0; i < 5; i++) { h.api.sync(); h.elapse(5000); }
  assert.equal(h.timers.size, 0);
  h.data.turnHistory.history.entitiesByKey[uuid(2)].itemsPagination.hasLoadedOldest = true;
  resolve(); await pending;
  assert.equal(h.timers.size, 1);
  h.api.sync(); h.api.sync();
  assert.equal(h.timers.size, 1);
  await h.fireNextTimer();
  assert.equal(h.timers.size, 1);
  h.enable(false);
  assert.equal(h.timers.size, 0);
});

test("bounded single pages continue past agent-only items until the user input is recovered", async () => {
  const h = harness();
  let pages = 0;
  h.setLoader(async (_thread, id, all) => {
    assert.equal(all, false);
    pages++;
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "agentMessage" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - pages };
    if (pages === 3) (entry.params.input as any[]).push({ type: "text", text: "fixture" });
  });
  for (let i = 0; i < 8; i++) await h.tick();
  assert.equal(h.calls.length, 3);
  assert.equal(h.notices.length, 0);
});

test("large incomplete turns stop at four single pages instead of chasing the entire history", async () => {
  const h = harness();
  let pages = 0;
  h.setLoader(async (_thread, id, all) => {
    assert.equal(all, false);
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "agentMessage" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - ++pages };
  });
  for (let i = 0; i < 12; i++) await h.tick();
  assert.equal(h.calls.length, 4);
  assert.equal(h.notices.length, 1);
});

test("scope and conversation are re-read instead of retaining an old manager snapshot", async () => {
  const h = harness(); await h.tick();
  h.data = conversation([[turn(11, true), turn(12), turn(13, true)]]);
  await h.tick();
  assert.equal(h.calls.length, 2);
  assert.equal(h.calls[1][1], uuid(12));
  assert.ok(h.scopeReads >= 2);
});

test("automatic work is bounded to at most 32 registered interior turns", async () => {
  const empty = Array.from({ length: 40 }, (_, i) => turn(i + 2));
  const h = harness({ data: conversation([[turn(1, true), ...empty, turn(99, true)]]) });
  for (let i = 0; i < 45; i++) await h.tick();
  assert.equal(h.calls.length, 32);
  assert.equal(h.api.ledger.attempts.size, 32);
  assert.equal(h.notices.length, 1);
});
