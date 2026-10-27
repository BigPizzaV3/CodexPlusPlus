import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";

const root = path.resolve(import.meta.dirname, "../../..");
const fragments = path.join(root, "assets/inject/renderer-inject");
const source = readFileSync(path.join(fragments, "85-history-turn-recovery.js"), "utf8");
const nativeSource = "var defaultManager=derive(AppScope,({get:g})=>{let m=g(rawManagers,localHost);if(m==null)throw Error(`Missing default AppServerManager`);return m});export{rawManagers as managerToken};";
const uuid = (n: number) => `10000000-0000-0000-0000-${String(n).padStart(12, "0")}`;
const threadId = uuid(999);

function turn(n: number, loaded = false) {
  return {
    turnId: uuid(n), status: "completed", items: loaded ? [{ type: "agentMessage" }] : [],
    durationMs: 0 as number | null, turnStartedAtMs: 1000 as number | null,
    finalAssistantStartedAtMs: 1000 as number | null,
    params: { threadId, input: [] },
    itemsPagination: { hasLoadedOldest: loaded, isLoadingOlder: false, olderCursor: null },
  };
}

function conversation(groups = [[turn(1, true), turn(2), turn(3, true)]]) {
  return {
    resumeState: "resumed", requests: [], threadRuntimeStatus: { type: "idle" },
    turnsPagination: { isLoadingOlder: false },
    turnHistory: {
      kind: "canonical",
      history: {
        generation: 1,
        entitiesByKey: Object.fromEntries(groups.flat().map((entry) => [entry.turnId, entry])),
        islands: groups.map((turns) => ({ entries: turns.map((t) => ({ value: t.turnId })) })),
      },
    },
  };
}

function harness(data = conversation()) {
  let currentId = threadId, host = "local", now = 100000;
  let discoverySource = nativeSource, fetches = 0, scopeReads = 0, nextTimer = 0;
  let pageTouches = 0, conversationReads = 0;
  const settings = { enhancementsEnabled: true, codexAppHistoryTurnRecovery: true as unknown };
  const timers = new Map<number, () => unknown>();
  const calls: unknown[][] = [];
  const metadataCalls: unknown[][] = [];
  let role = "owner", broadcasts = 0, updates = 0;
  let metadataLoad: (params: any) => Promise<any> = async () => ({
    data: Object.values(data.turnHistory.history.entitiesByKey).map((entry) => ({
      id: entry.turnId, status: entry.status, durationMs: 3210, startedAt: 50, completedAt: 54, items: [],
    })),
    nextCursor: null,
  });
  const signal = { kind: "signal-family", scope: { __scopeBrand: "AppScope" }, resolve() {}, read() {} };
  let load: (...args: unknown[]) => Promise<void> = async (_thread, id) => {
    const entry = data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "userMessage" });
    entry.itemsPagination.hasLoadedOldest = true;
  };
  const manager = {
    hostId: "local", disposed: false, getHostId: () => "local",
    getStreamRole: () => ({ role }),
    getConversation: (id: string) => { conversationReads++; return id === currentId ? data : null; },
    sendRequest: async (...args: unknown[]) => {
      metadataCalls.push(args);
      assert.equal(args[0], "thread/turns/list");
      return metadataLoad(args[1]);
    },
    updateConversationState: (_id: string, mutate: (draft: typeof data) => void, broadcastPatches: boolean) => {
      assert.equal(broadcastPatches, false); updates++; mutate(data);
    },
    broadcastConversationSnapshot: () => { broadcasts++; },
    pagination: { itemPagination: { loadTurnItems: (...args: unknown[]) => { calls.push(args); return load(...args); } } },
  };
  const scope = {
    scope: signal.scope,
    node: { familyBindings: new Map([[signal, new Map([["local", manager]])]]) },
    get(key: unknown, h: string) {
      assert.equal(key, signal); assert.equal(h, "local"); scopeReads++;
      return manager;
    },
  };
  const marker = {
    getAttribute: () => `local:${currentId}`,
    __reactFiber$fixture: { memoizedState: { memoizedState: { current: scope }, next: null }, return: null },
  };
  const document = {
    hidden: false,
    querySelector: (selector: string) => selector.includes("data-above-composer-conversation-id") ? marker : null,
    getElementById: () => null,
  };
  const window = {
    location: { search: "" },
    setTimeout(fn: () => unknown) { const id = ++nextTimer; timers.set(id, fn); return id; },
    clearTimeout(id: number) { timers.delete(id); },
  };
  const context = vm.createContext({
    window, document, Map, Set, URL, AbortController, Date: { now: () => now },
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: settings,
    currentSessionRef: () => { pageTouches++; return { session_id: currentId, host_id: host }; },
    normalizedCodexThreadUuid: (id: unknown) => typeof id === "string" && /^[a-f\d]{8}(?:-[a-f\d]{4}){3}-[a-f\d]{12}$/i.test(id) ? id : "",
    codexAppAssetUrl: () => "app://-/assets/app-initial-fixture.js",
    codexAppAssetUrlFromScriptText: async () => "",
    loadOptionalCodexAppModule: async () => ({ managerToken: signal }),
    fetch: async () => { fetches++; return { ok: true, text: async () => discoverySource }; },
  });
  function inject() {
    vm.runInContext(`(() => { ${source}
      globalThis.testApi = {
        run: runHistoryTurnRecovery, signal: historyTurnRecoverySignal,
        islands: historyTurnRecoveryIslands, busy: historyTurnRecoveryBusy,
        state: historyTurnRecoveryState, ledger: historyTurnRecoveryLedger,
      };
    })();`, context);
  }
  inject();
  return {
    context, signal, scope, manager, marker, document, window, calls, metadataCalls, timers,
    get api() { return context.testApi; },
    get fetches() { return fetches; }, get scopeReads() { return scopeReads; },
    get pageTouches() { return pageTouches; }, get conversationReads() { return conversationReads; },
    get broadcasts() { return broadcasts; }, get updates() { return updates; },
    get data() { return data; }, set data(value) { data = value; },
    navigate(id: string, h = "local") { currentId = id; host = h; },
    setLoader(fn: typeof load) { load = fn; },
    setMetadataLoader(fn: typeof metadataLoad) { metadataLoad = fn; },
    setRole(value: string) { role = value; },
    setEnabled(value: unknown) { settings.codexAppHistoryTurnRecovery = value; },
    setMasterEnabled(value: boolean) { settings.enhancementsEnabled = value; },
    setSettingsLoaded(value: boolean) { context.codexPlusBackendSettingsLoaded = value; },
    setDiscovery(value: string) { discoverySource = value; },
    elapse(ms: number) { now += ms; },
    inject,
    run(id = uuid(1000)) { return context.testApi.run(id); },
  };
}

test("registration is inert with no heartbeat, polling or independent repair trigger", () => {
  const h = harness();
  assert.equal(h.fetches, 0); assert.equal(h.scopeReads, 0);
  assert.equal(h.calls.length, 0); assert.equal(h.timers.size, 0);
  assert.doesNotMatch(source, /historyTurnRecoveryPollMs|scheduleHistoryTurnRecovery|showToast/);
  const backendSync = readFileSync(path.join(fragments, "40-backend-settings.js"), "utf8");
  assert.doesNotMatch(backendSync, /syncCodexPlusHistoryTurnRecovery|runHistoryTurnRecovery/);
});

test("hidden windows and unavailable native APIs return content-free diagnostic reasons", async () => {
  const hidden = harness(); hidden.document.hidden = true;
  const skip = await hidden.run();
  assert.equal(skip.status, "skipped"); assert.equal(skip.reason, "window_hidden");
  assert.equal(hidden.fetches, 0); assert.equal(hidden.scopeReads, 0);
  const unavailable = harness(); unavailable.setDiscovery("private fixture text");
  const result = await unavailable.run();
  assert.equal(result.status, "unavailable"); assert.equal(result.reason, "native_api_unavailable");
  assert.doesNotMatch(JSON.stringify(result), /private fixture text/);
  assert.equal(unavailable.calls.length, 0); assert.equal(unavailable.metadataCalls.length, 0);
  const completed = await harness().run();
  assert.equal(completed.status, "completed"); assert.equal(completed.reason, null);
});

test("the default-off recovery switch is exposed in both settings UIs and uses the backend mapping", () => {
  const app = readFileSync(path.join(root, "apps/codex-plus-manager/src/App.tsx"), "utf8");
  const styles = readFileSync(path.join(fragments, "10-style.js"), "utf8");
  const backend = readFileSync(path.join(fragments, "40-backend-settings.js"), "utf8");
  const catalog = JSON.parse(readFileSync(path.join(root, "crates/codex-plus-core/assets/enhancement-settings.json"), "utf8"));
  const entries = catalog.items.filter((item: { key: string }) => item.key === "codexAppHistoryTurnRecovery");
  assert.equal(entries.length, 1);
  assert.equal(entries[0].group, "chat");
  assert.equal(entries[0].control, "bool");
  assert.deepEqual(entries[0].surfaces, ["manager", "codex"]);
  assert.notEqual(entries[0].masterGated, false);
  assert.equal(entries[0].title, "历史轮次补载（实验）");
  assert.match(app, /codexAppHistoryTurnRecovery: boolean/);
  assert.match(app, /codexAppHistoryTurnRecovery: false/);
  assert.match(app, /managerGroupItems\(group\.id, enhancementPlatform\)\.map\(renderSetting\)/);
  assert.match(styles, /historyTurnRecovery: false/);
  assert.match(styles, /historyTurnRecovery: "codexAppHistoryTurnRecovery"/);
  assert.match(backend, /renderCodexPlusSettingsCatalogSections\(\)/);
  assert.match(backend, /Object\.keys\(codexPlusBackendSettingMap\)/);
});

test("the in-app switch maps saved settings and persists only its backend partial update", async () => {
  const style = readFileSync(path.join(fragments, "10-style.js"), "utf8");
  const writes: Array<[string, unknown]> = [];
  const stored = new Map<string, string>();
  const context = vm.createContext({
    window: {}, conversationViewDefaultWidth: 900, codexPlusSettingsKey: "settings",
    codexPlusBackendSettings: {} as Record<string, unknown>, codexPlusBackendSettingsLoaded: true,
    localStorage: { getItem: (key: string) => stored.get(key) || null, setItem: (key: string, value: string) => stored.set(key, value) },
    setBackendSetting: async (key: string, value: unknown) => { writes.push([key, value]); },
    loadBackendSettings: async () => {}, syncStepwisePanel: () => {},
  });
  const start = style.indexOf("  function defaultCodexPlusSettings()");
  const end = style.indexOf("  // Dream skin runtime", start);
  const setter = style.indexOf("  function setCodexPlusSetting(");
  const setterEnd = style.indexOf("  function syncStepwisePanel(", setter);
  assert.ok(start >= 0 && end > start && setter >= 0 && setterEnd > setter);
  vm.runInContext(`${style.slice(start, end)}\n${style.slice(setter, setterEnd)}`, context);
  assert.equal(context.codexPlusSettings().historyTurnRecovery, false);
  context.codexPlusBackendSettings = { enhancementsEnabled: true, codexAppHistoryTurnRecovery: true };
  assert.equal(context.codexPlusSettings().historyTurnRecovery, true);
  context.codexPlusBackendSettings.enhancementsEnabled = false;
  assert.equal(context.codexPlusSettings().historyTurnRecovery, false);
  context.codexPlusBackendSettings.enhancementsEnabled = true;
  for (const enabled of [false, true]) context.setCodexPlusSetting("historyTurnRecovery", enabled);
  await Promise.resolve();
  assert.deepEqual(writes, [["codexAppHistoryTurnRecovery", false], ["codexAppHistoryTurnRecovery", true]]);
  assert.equal(stored.size, 0);
});

test("disabled, unloaded or non-boolean opt-in cannot inspect the page or consume a request", async () => {
  for (const change of [
    (h: ReturnType<typeof harness>) => h.setEnabled(undefined),
    (h: ReturnType<typeof harness>) => h.setEnabled(false),
    (h: ReturnType<typeof harness>) => h.setEnabled("true"),
    (h: ReturnType<typeof harness>) => h.setEnabled(1),
    (h: ReturnType<typeof harness>) => h.setSettingsLoaded(false),
    (h: ReturnType<typeof harness>) => h.setMasterEnabled(false),
  ]) {
    const h = harness(); change(h);
    h.context.codexPlusSettings = () => ({ historyTurnRecovery: true });
    const original = structuredClone(h.data);
    assert.equal((await h.run()).status, "skipped");
    assert.equal(h.pageTouches, 0); assert.equal(h.fetches, 0); assert.equal(h.scopeReads, 0);
    assert.equal(h.conversationReads, 0); assert.equal(h.calls.length, 0); assert.equal(h.metadataCalls.length, 0);
    assert.equal(h.updates, 0); assert.equal(h.broadcasts, 0); assert.equal(h.timers.size, 0);
    assert.equal(h.api.ledger.requests.size, 0); assert.equal(h.api.ledger.flight, null);
    assert.deepEqual(h.data, original);
    h.setEnabled(true); h.setSettingsLoaded(true); h.setMasterEnabled(true);
    assert.equal((await h.run()).status, "completed");
  }
});

test("only existing successful repair calls notify frontend; report refresh stays read-only", () => {
  const manager = readFileSync(path.join(root, "apps/codex-plus-manager/src-tauri/src/commands.rs"), "utf8");
  const command = manager.slice(manager.indexOf("pub async fn repair_session_index()"), manager.indexOf("pub async fn load_session_index_repair_report()"));
  assert.match(command, /Ok\(report\) => \{[\s\S]*after_index_repair\([\s\S]*report\.aborted_reason\.is_some\(\)/);
  assert.doesNotMatch(command, /after_index_repair\([^;]+\.await/s);
  const reportRead = manager.slice(manager.indexOf("pub async fn load_session_index_repair_report()"), manager.indexOf("pub async fn preview_session_index_cleanup()"));
  assert.doesNotMatch(reportRead, /after_index_repair/);
  assert.match(reportRead, /history_turn_recovery::load_report\([\s\S]*report\.checked_at_ms/);
  assert.match(command, /after_index_repair\([\s\S]*report\.checked_at_ms/);
  assert.match(command, /payload\["frontendRecovery"\]/);
  const launcher = readFileSync(path.join(root, "apps/codex-plus-launcher/src/main.rs"), "utf8");
  assert.match(launcher, /Ok\(Some\(report\)\) => \{[\s\S]*after_index_repair\(/);
  assert.doesNotMatch(launcher, /after_index_repair\([^;]+\.await/s);
  assert.match(launcher, /Some\(handle\.debug_port\)/);
  assert.match(launcher, /std::time::Duration::from_secs\(30 \* 60\)/);
});

test("discovers only the uniquely exported raw family without executing source", () => {
  const h = harness();
  assert.equal(h.api.signal({ managerToken: h.signal }, nativeSource), h.signal);
  const renamed = nativeSource.replaceAll("rawManagers", "$family").replaceAll("managerToken", "renamed");
  assert.equal(h.api.signal({ renamed: h.signal }, renamed), h.signal);
  assert.equal(h.api.signal({ managerToken: h.signal }, `${nativeSource};${nativeSource}`), null);
  assert.equal(h.api.signal({}, nativeSource), null);
  assert.equal(h.api.signal({ managerToken: { ...h.signal, kind: "readable-family" } }, nativeSource), null);
  assert.equal(h.scopeReads, 0);
});

test("zero or multiple family guards and ambiguous exports are unavailable with no native reads or writes", async () => {
  for (const discovery of [
    "export{};",
    `${nativeSource};${nativeSource}`,
    nativeSource.replace("rawManagers as managerToken", "rawManagers as managerToken,rawManagers as anotherToken"),
  ]) {
    const h = harness(); h.setDiscovery(discovery);
    const original = structuredClone(h.data);
    const report = await h.run();
    assert.equal(report.status, "unavailable");
    assert.equal(report.loadedPages, 0); assert.equal(report.metadataPages, 0);
    assert.equal(h.scopeReads, 0); assert.equal(h.conversationReads, 0);
    assert.equal(h.calls.length, 0); assert.equal(h.metadataCalls.length, 0);
    assert.equal(h.updates, 0); assert.equal(h.broadcasts, 0);
    assert.deepEqual(h.data, original);
    assert.equal(h.api.ledger.flight, null); assert.equal(h.timers.size, 0);
  }
});

test("switching recovery off during discovery prevents native manager and conversation access", async () => {
  const h = harness();
  const pending = h.run(); h.setEnabled(false); await pending;
  assert.equal(h.scopeReads, 0); assert.equal(h.conversationReads, 0);
  assert.equal(h.calls.length, 0); assert.equal(h.metadataCalls.length, 0);
  assert.equal(h.updates, 0); assert.equal(h.broadcasts, 0);
  assert.equal(h.timers.size, 0);
});

test("invalid and duplicate request IDs cannot discover or repeat work, even after reinjection", async () => {
  const h = harness();
  await h.run("invalid");
  assert.equal(h.fetches, 0);
  await h.run();
  h.data = conversation();
  await h.run(); h.inject(); await h.run();
  assert.equal(h.calls.length, 1); assert.equal(h.timers.size, 0);
});

test("one existing repair request restores three interior turns serially", async () => {
  const h = harness(conversation([[turn(1, true), turn(2), turn(3), turn(4), turn(5, true)]]));
  const result = await h.run();
  assert.equal(result.status, "completed"); assert.equal(result.loadedPages, 3);
  assert.deepEqual(h.calls.map((args) => args.slice(0, 3)), [2, 3, 4].map((n) => [threadId, uuid(n), false]));
  for (const args of h.calls) assert.deepEqual(JSON.parse(JSON.stringify(args[3])), { priority: "interactive", source: "thread_hydration" });
  assert.equal(h.fetches, 1); assert.equal(h.timers.size, 0);
});

test("recovers registered boundary turns without fetching undiscovered turn pages", async () => {
  const h = harness(conversation([[turn(1), turn(2, true), turn(3)], [turn(4), turn(5, true), turn(6)]]));
  await h.run();
  assert.deepEqual(h.calls.map((args) => args[1]), [1, 3, 4, 6].map(uuid));
  assert.equal(h.data.turnHistory.history.islands.length, 2);
  assert.equal(Object.keys(h.data.turnHistory.history.entitiesByKey).length, 6);
});

test("evicted items, reconnects and proven complete empty turns are not gaps", async () => {
  for (const change of [
    (t: any) => { t.itemsPagination.evicted = true; },
    (t: any) => { t.itemsPagination.reconnect = {}; },
    (t: any) => { t.itemsPagination.hasLoadedOldest = true; },
  ]) {
    const h = harness(); change(h.data.turnHistory.history.entitiesByKey[uuid(2)]);
    await h.run(); assert.equal(h.calls.length, 0);
  }
});

test("yields to generation, submissions, requests, resume and native pagination", async () => {
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
    await h.run(); assert.equal(h.calls.length, 0); assert.equal(h.timers.size, 0);
  }
});

test("unknown or inconsistent history structures fail closed", async () => {
  for (const change of [
    (c: any) => { c.turnHistory.kind = "legacy"; },
    (c: any) => { delete c.turnHistory.history.generation; },
    (c: any) => { delete c.turnHistory.history.entitiesByKey[uuid(2)]; },
    (c: any) => { c.turnHistory.history.islands[0].entries.push({ value: uuid(2) }); },
    (c: any) => { delete c.turnHistory.history.entitiesByKey[uuid(2)].params.input; },
    (c: any) => { delete c.turnHistory.history.entitiesByKey[uuid(2)].itemsPagination; },
  ]) {
    const h = harness(); change(h.data);
    await h.run(); assert.equal(h.calls.length, 0);
  }
});

test("never instantiates an unbound manager family or uses a mismatched host", async () => {
  const h = harness();
  h.scope.node.familyBindings.clear();
  await h.run(); assert.equal(h.scopeReads, 0);
  h.scope.node.familyBindings.set(h.signal, new Map([["local", h.manager]]));
  h.manager.hostId = "remote";
  await h.run(uuid(1001)); assert.equal(h.calls.length, 0);
});

test("ignores remote, hidden, auxiliary and mismatched-composer windows", async () => {
  for (const change of [
    (h: ReturnType<typeof harness>) => h.navigate(threadId, "remote"),
    (h: ReturnType<typeof harness>) => { h.document.hidden = true; },
    (h: ReturnType<typeof harness>) => { h.window.location.search = "?initialRoute=thread"; },
    (h: ReturnType<typeof harness>) => { h.marker.getAttribute = () => `local:${uuid(888)}`; },
  ]) {
    const h = harness(); change(h);
    await h.run(); assert.equal(h.fetches, 0); assert.equal(h.calls.length, 0);
  }
});

test("navigation during discovery cancels without queuing work for another conversation", async () => {
  const h = harness();
  const pending = h.run(); h.navigate(uuid(888)); await pending;
  assert.equal(h.calls.length, 0); assert.equal(h.timers.size, 0);
});

test("rejection or no progress stops without retry; a new explicit repair may retry", async () => {
  for (const reject of [false, true]) {
    const h = harness();
    h.setLoader(async () => { if (reject) throw new Error("private fixture"); });
    const result = await h.run();
    assert.equal(result.status, "partial"); assert.equal(h.calls.length, 1);
    assert.equal(h.timers.size, 0);
    h.elapse(30000); await h.run(); assert.equal(h.calls.length, 1);
    await h.run(uuid(1001)); assert.equal(h.calls.length, 2);
    assert.doesNotMatch(JSON.stringify(result), /private fixture/);
  }
});

test("unsupported API discovery is once per request and never creates a retry timer", async () => {
  const h = harness(); h.setDiscovery("export{};");
  assert.equal((await h.run()).status, "unavailable");
  await h.run(); h.inject();
  assert.equal(h.fetches, 1); assert.equal(h.timers.size, 0);
  await h.run(uuid(1001)); assert.equal(h.fetches, 2);
});

test("overlapping requests and reinjection share one in-flight native loader", async () => {
  const h = harness();
  let resolve!: () => void;
  h.setLoader(() => new Promise<void>((done) => { resolve = done; }));
  const first = h.run(); await new Promise(setImmediate);
  assert.equal(h.calls.length, 1);
  assert.equal((await h.run(uuid(1001))).status, "busy");
  const old = h.api; h.inject(); assert.equal(old.state.disposed, true);
  assert.equal((await h.run(uuid(1002))).status, "busy");
  resolve(); await first;
  assert.equal(h.api.ledger.flight, null); assert.equal(h.calls.length, 1);
  assert.equal(h.timers.size, 0);
});

test("generation and navigation guards are rechecked before subsequent pages and turns", async () => {
  for (const change of [
    (h: ReturnType<typeof harness>) => { h.data.turnHistory.history.generation++; },
    (h: ReturnType<typeof harness>) => { h.data.threadRuntimeStatus.type = "active"; },
    (h: ReturnType<typeof harness>) => { (h.data.requests as any[]).push({}); },
    (h: ReturnType<typeof harness>) => { h.data.turnsPagination.isLoadingOlder = true; },
    (h: ReturnType<typeof harness>) => { h.document.hidden = true; },
    (h: ReturnType<typeof harness>) => { h.navigate(uuid(888)); },
  ]) {
    const h = harness(conversation([[turn(1, true), turn(2), turn(3), turn(4, true)]]));
    h.setLoader(async (_thread, id) => {
      h.data.turnHistory.history.entitiesByKey[id as string].itemsPagination.hasLoadedOldest = true;
      change(h);
    });
    await h.run(); assert.equal(h.calls.length, 1);
  }
});

test("single pages continue until native pagination confirms the oldest page", async () => {
  const h = harness();
  let pages = 0;
  h.setLoader(async (_thread, id, all) => {
    assert.equal(all, false);
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "agentMessage" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - ++pages };
    if (pages === 3) entry.itemsPagination.hasLoadedOldest = true;
  });
  assert.equal((await h.run()).status, "completed"); assert.equal(h.calls.length, 3);
});

test("large incomplete turns share a bounded 128 single page budget", async () => {
  const h = harness();
  h.setLoader(async (_thread, id) => {
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "agentMessage" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - h.calls.length };
  });
  assert.equal((await h.run()).status, "partial"); assert.equal(h.calls.length, 128);
});

test("one repair is bounded to 32 turns and cannot start calls past its deadline", async () => {
  const empty = Array.from({ length: 40 }, (_, i) => turn(i + 2));
  const h = harness(conversation([[turn(1, true), ...empty, turn(99, true)]]));
  assert.equal((await h.run()).status, "partial"); assert.equal(h.calls.length, 32);
  const slow = harness(conversation([[turn(1, true), turn(2), turn(3), turn(4), turn(5, true)]]));
  slow.setLoader(async (_thread, id) => {
    slow.data.turnHistory.history.entitiesByKey[id as string].itemsPagination.hasLoadedOldest = true;
    slow.elapse(6000);
  });
  await slow.run(); assert.equal(slow.calls.length, 2);
});

test("deduplication is bounded and navigation alone never requests recovery", async () => {
  const h = harness();
  for (let i = 1000; i < 1070; i++) await h.run(uuid(i));
  assert.equal(h.api.ledger.requests.size, 64);
  h.data = conversation(); h.navigate(uuid(888)); h.inject();
  assert.equal(h.calls.length, 1); assert.equal(h.timers.size, 0);
});

test("user input is not proof that the final report page has been loaded", async () => {
  const h = harness();
  let pages = 0;
  h.setLoader(async (_thread, id) => {
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    pages++;
    entry.items.push({ type: pages === 6 ? "agentMessage" : "commandExecution" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - pages };
    if (pages === 1) (entry.params.input as any[]).push({ type: "text", text: "fixture" });
    if (pages === 6) entry.itemsPagination.hasLoadedOldest = true;
  });
  const result = await h.run();
  assert.equal(result.status, "completed");
  assert.equal(result.loadedPages, 6);
  assert.equal(h.data.turnHistory.history.entitiesByKey[uuid(2)].itemsPagination.hasLoadedOldest, true);
});

test("consecutive nonempty incomplete turns are recovered without content anchors", async () => {
  const groups = [[turn(1), turn(2)], [turn(3), turn(4)]];
  for (const entry of groups.flat()) {
    entry.items.push({ type: "commandExecution" });
    (entry.params.input as any[]).push({ type: "text", text: "fixture" });
  }
  const h = harness(conversation(groups));
  const result = await h.run();
  assert.equal(result.status, "completed");
  assert.equal(result.loadedPages, 4);
  assert.deepEqual(new Set(h.calls.map((args) => args[1])), new Set(groups.flat().map((entry) => entry.turnId)));
  assert.ok(groups.flat().every((entry) => entry.itemsPagination.hasLoadedOldest));
});

test("a new repair continues a nonempty partial turn instead of silently skipping it", async () => {
  const h = harness();
  let calls = 0;
  h.setLoader(async (_thread, id) => {
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "commandExecution" });
    (entry.params.input as any[]).push({ type: "text", text: "fixture" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - ++calls };
    if (calls === 1) h.elapse(12000);
    else entry.itemsPagination.hasLoadedOldest = true;
  });
  assert.equal((await h.run()).status, "partial");
  assert.equal((await h.run(uuid(1001))).status, "completed");
  assert.equal(h.calls.length, 2);
});

test("a cursor cloned without progress cannot consume the entire page budget", async () => {
  const h = harness();
  const entry = h.data.turnHistory.history.entitiesByKey[uuid(2)];
  (entry.itemsPagination as any).olderCursor = { ordinal: 1000 };
  h.setLoader(async () => {
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 };
  });
  assert.equal((await h.run()).status, "partial");
  assert.equal(h.calls.length, 1);
});

test("exactly filling the page budget may still report complete", async () => {
  const h = harness();
  h.setLoader(async (_thread, id) => {
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "commandExecution" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - h.calls.length };
    if (h.calls.length === 128) entry.itemsPagination.hasLoadedOldest = true;
  });
  assert.equal((await h.run()).status, "completed");
  assert.equal(h.calls.length, 128);
});

function missingMetadata(h: ReturnType<typeof harness>, n = 2) {
  const entry = h.data.turnHistory.history.entitiesByKey[uuid(n)];
  entry.durationMs = entry.turnStartedAtMs = entry.finalAssistantStartedAtMs = null;
  return entry;
}

const metadataRow = (n = 2) => ({
  id: uuid(n), status: "completed", durationMs: 3210, startedAt: 50, completedAt: 54, items: [],
});

test("metadata-only reads restore native duration and timestamps without rebuilding history", async () => {
  const h = harness(conversation([[turn(1, true), turn(2, true)], [turn(3, true)]]));
  const entry = missingMetadata(h);
  const { items, params, itemsPagination } = entry;
  const { islands, generation, entitiesByKey } = h.data.turnHistory.history;
  const result = await h.run();
  assert.equal(result.status, "completed");
  assert.equal(result.metadataPages, 1); assert.equal(result.loadedPages, 0);
  assert.equal(entry.durationMs, 3210);
  assert.equal(entry.turnStartedAtMs, 50000); assert.equal(entry.finalAssistantStartedAtMs, 54000);
  assert.equal(entry.items, items); assert.equal(entry.params, params); assert.equal(entry.itemsPagination, itemsPagination);
  assert.equal(h.data.turnHistory.history.islands, islands);
  assert.equal(h.data.turnHistory.history.entitiesByKey, entitiesByKey);
  assert.equal(h.data.turnHistory.history.generation, generation);
  assert.equal(h.broadcasts, 1);
  assert.equal(h.calls.length, 0);
  assert.deepEqual(h.metadataCalls[0].slice(0, 1), ["thread/turns/list"]);
  assert.deepEqual(JSON.parse(JSON.stringify(h.metadataCalls[0][1])), {
    threadId, cursor: null, limit: 32, sortDirection: "desc", itemsView: "notLoaded",
  });
});

test("metadata scanning follows bounded opaque cursors to older registered turns", async () => {
  const h = harness();
  missingMetadata(h);
  h.setMetadataLoader(async ({ cursor }) => cursor == null
    ? { data: [metadataRow(888)], nextCursor: "older-page" }
    : { data: [metadataRow()], nextCursor: null });
  const result = await h.run();
  assert.equal(result.status, "completed");
  assert.equal(result.metadataPages, 2); assert.equal(result.loadedPages, 1);
  assert.deepEqual(h.metadataCalls.map((call: any) => call[1].cursor), [null, "older-page"]);
  assert.equal(h.data.turnHistory.history.entitiesByKey[uuid(888)], undefined);
});

test("metadata never overwrites existing values, including a genuine zero", async () => {
  const h = harness();
  const entry = h.data.turnHistory.history.entitiesByKey[uuid(2)];
  entry.finalAssistantStartedAtMs = null;
  assert.equal((await h.run()).status, "completed");
  assert.equal(entry.durationMs, 0); assert.equal(entry.turnStartedAtMs, 1000);
  assert.equal(entry.finalAssistantStartedAtMs, 54000);
});

test("malformed or unproven server metadata is never fabricated", async () => {
  for (const change of [
    (row: any) => { row.status = "inProgress"; },
    (row: any) => { row.durationMs = -1; },
    (row: any) => { row.durationMs = null; },
    (row: any) => { row.startedAt = "50"; },
    (row: any) => { row.completedAt = Infinity; },
  ]) {
    const h = harness(); const entry = missingMetadata(h);
    const row = metadataRow(); change(row);
    h.setMetadataLoader(async () => ({ data: [row], nextCursor: null }));
    assert.equal((await h.run()).status, "partial");
    assert.ok([entry.durationMs, entry.turnStartedAtMs, entry.finalAssistantStartedAtMs].some((value) => value == null));
    assert.equal(h.calls.length, 1);
  }
});

test("metadata ownership, generation, navigation and receipt guards are rechecked after reads", async () => {
  for (const change of [
    (h: ReturnType<typeof harness>) => h.setEnabled(false),
    (h: ReturnType<typeof harness>) => h.setSettingsLoaded(false),
    (h: ReturnType<typeof harness>) => { h.data.turnHistory.history.generation++; },
    (h: ReturnType<typeof harness>) => { h.data.threadRuntimeStatus.type = "active"; },
    (h: ReturnType<typeof harness>) => { (h.data as any).unconfirmedTurnSubmissions = [{ terminal: true }]; },
    (h: ReturnType<typeof harness>) => { h.setRole("follower"); },
    (h: ReturnType<typeof harness>) => { h.navigate(uuid(888)); },
    (h: ReturnType<typeof harness>) => { h.document.hidden = true; },
    (h: ReturnType<typeof harness>) => { h.inject(); },
    (h: ReturnType<typeof harness>) => { h.manager.disposed = true; },
    (h: ReturnType<typeof harness>) => { h.scope.get = () => ({ ...h.manager }); },
  ]) {
    const h = harness(conversation([[turn(1, true), turn(2, true), turn(3, true)]]));
    const entry = missingMetadata(h);
    h.setMetadataLoader(async () => { change(h); return { data: [metadataRow()], nextCursor: null }; });
    await h.run();
    assert.equal(entry.durationMs, null); assert.equal(entry.turnStartedAtMs, null);
    assert.equal(h.updates, 0); assert.equal(h.calls.length, 0); assert.equal(h.broadcasts, 0);
  }
});

test("metadata setter repeats guards before touching a draft", async () => {
  for (const change of ["receipt", "retire", "disable"]) {
    const h = harness(); const entry = missingMetadata(h);
    h.manager.updateConversationState = (_id, mutate) => {
      if (change === "retire") h.manager.disposed = true;
      else if (change === "disable") h.setEnabled(false);
      else (h.data as any).unconfirmedTurnSubmissions = [{ terminal: true }];
      mutate(h.data);
    };
    await h.run();
    assert.equal(entry.durationMs, null); assert.equal(h.broadcasts, 0);
  }
});

test("switching recovery off while an item load is pending stops subsequent reads", async () => {
  const h = harness();
  h.setLoader(async () => { h.setEnabled(false); });
  await h.run();
  assert.equal(h.calls.length, 1);
  assert.equal(h.metadataCalls.length, 0); assert.equal(h.updates, 0); assert.equal(h.broadcasts, 0);
});

test("an already retired manager cannot read or mutate history", async () => {
  const h = harness(); missingMetadata(h);
  h.manager.disposed = true;
  await h.run();
  assert.equal(h.metadataCalls.length, 0); assert.equal(h.calls.length, 0);
  assert.equal(h.updates, 0); assert.equal(h.broadcasts, 0);
});

test("metadata updates survive immutable native snapshots and synchronous subscriptions", async () => {
  const h = harness(conversation([[turn(1, true), turn(2, true), turn(3, true)]])); missingMetadata(h);
  const original = h.data;
  h.manager.updateConversationState = (_id, mutate) => {
    const draft = structuredClone(h.data);
    mutate(draft);
    h.data = draft;
    h.data.threadRuntimeStatus.type = "active";
  };
  const result = await h.run();
  assert.equal(result.status, "partial");
  assert.equal(original.turnHistory.history.entitiesByKey[uuid(2)].durationMs, null);
  assert.equal(h.data.turnHistory.history.entitiesByKey[uuid(2)].durationMs, 3210);
  assert.equal(h.data.turnHistory.history.generation, original.turnHistory.history.generation);
  assert.equal(h.broadcasts, 0); assert.equal(h.calls.length, 0);
});

test("synchronous retirement after metadata commit cannot broadcast a stale snapshot", async () => {
  const h = harness(conversation([[turn(1, true), turn(2, true), turn(3, true)]])); missingMetadata(h);
  h.manager.updateConversationState = (_id, mutate) => {
    const draft = structuredClone(h.data);
    mutate(draft);
    h.data = draft;
    h.manager.disposed = true;
  };
  assert.equal((await h.run()).status, "partial");
  assert.equal(h.data.turnHistory.history.entitiesByKey[uuid(2)].durationMs, 3210);
  assert.equal(h.broadcasts, 0); assert.equal(h.calls.length, 0);
});

test("follower and terminal submission receipts cannot trigger metadata writes", async () => {
  for (const role of ["owner", "follower", "unknown"]) {
    const h = harness(); missingMetadata(h); h.setRole(role);
    if (role === "owner") (h.data as any).unconfirmedTurnSubmissions = [{ terminal: true }];
    const result = await h.run();
    assert.notEqual(result.status, "completed");
    assert.equal(h.metadataCalls.length, 0); assert.equal(h.updates, 0);
  }
});

test("metadata errors and unsupported setters preserve safe native item recovery", async () => {
  for (const mode of ["reject", "missing-api"]) {
    const h = harness(); missingMetadata(h);
    if (mode === "missing-api") (h.manager as any).updateConversationState = undefined;
    else h.setMetadataLoader(async () => { throw new Error("private fixture"); });
    const result = await h.run();
    assert.equal(result.status, "partial"); assert.equal(result.loadedPages, 1);
    assert.doesNotMatch(JSON.stringify(result), /private fixture/);
    assert.equal(h.timers.size, 0);
  }
});

test("metadata pages share the deadline and total page budget with item recovery", async () => {
  const h = harness(); missingMetadata(h);
  h.setMetadataLoader(async () => ({ data: [], nextCursor: `page-${h.metadataCalls.length}` }));
  h.setLoader(async (_thread, id) => {
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "commandExecution" });
    (entry.itemsPagination as any).olderCursor = `item-page-${h.calls.length}`;
    if (h.calls.length === 96) entry.itemsPagination.hasLoadedOldest = true;
  });
  const result = await h.run();
  assert.equal(result.status, "partial");
  assert.equal(result.metadataPages, 32); assert.equal(result.loadedPages, 96);
  const metadataOnly = harness(conversation([[turn(1, true), turn(2, true), turn(3, true)]]));
  missingMetadata(metadataOnly);
  metadataOnly.setMetadataLoader(async () => ({ data: [], nextCursor: `page-${metadataOnly.metadataCalls.length}` }));
  assert.equal((await metadataOnly.run()).metadataPages, 64);
  const slow = harness(); missingMetadata(slow);
  slow.setMetadataLoader(async () => { slow.elapse(12000); return { data: [], nextCursor: "older-page" }; });
  assert.equal((await slow.run()).status, "partial");
  assert.equal(slow.metadataCalls.length, 1); assert.equal(slow.calls.length, 1);
});

test("a repeated metadata cursor cannot loop or prevent native item recovery", async () => {
  const h = harness(); missingMetadata(h);
  h.setMetadataLoader(async () => ({ data: [], nextCursor: "repeat" }));
  assert.equal((await h.run()).status, "partial");
  assert.equal(h.metadataCalls.length, 2); assert.equal(h.calls.length, 1);
});

test("partial native work is not mistaken for the final answer and file change records", async () => {
  const h = harness();
  const entry = h.data.turnHistory.history.entitiesByKey[uuid(2)];
  entry.items.push({ type: "commandExecution" });
  (entry.params.input as any[]).push({ type: "text", text: "fixture input" });
  const final = { type: "agentMessage", id: "final", phase: "final_answer", text: "fixture report" };
  const files = { type: "fileChange", id: "files", status: "completed", changes: [{ path: "fixture.ts", kind: { type: "add" }, diff: "+fixture" }] };
  h.setLoader(async () => {
    if (h.calls.length === 1) (entry.items as any[]).push(files);
    else {
      (entry.items as any[]).push(final);
      entry.itemsPagination.hasLoadedOldest = true;
    }
    (entry.itemsPagination as any).olderCursor = `page-${h.calls.length}`;
  });
  assert.equal((await h.run()).status, "completed");
  assert.equal(h.calls.length, 2);
  assert.ok(entry.items.includes(final)); assert.ok(entry.items.includes(files));
  assert.equal((entry.items as any[]).find((item) => item.phase === "final_answer"), final);
});
