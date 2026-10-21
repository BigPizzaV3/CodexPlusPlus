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
        entitiesByKey: Object.fromEntries(groups.flat().map((entry) => [entry.turnId, entry])),
        islands: groups.map((turns) => ({ entries: turns.map((t) => ({ value: t.turnId })) })),
      },
    },
  };
}

function harness(data = conversation()) {
  let currentId = threadId, host = "local", now = 100000;
  let discoverySource = nativeSource, fetches = 0, scopeReads = 0, nextTimer = 0;
  const timers = new Map<number, () => unknown>();
  const calls: unknown[][] = [];
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
    currentSessionRef: () => ({ session_id: currentId, host_id: host }),
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
    context, signal, scope, manager, marker, document, window, calls, timers,
    get api() { return context.testApi; },
    get fetches() { return fetches; }, get scopeReads() { return scopeReads; },
    get data() { return data; }, set data(value) { data = value; },
    navigate(id: string, h = "local") { currentId = id; host = h; },
    setLoader(fn: typeof load) { load = fn; },
    setDiscovery(value: string) { discoverySource = value; },
    elapse(ms: number) { now += ms; },
    inject,
    run(id = uuid(1000)) { return context.testApi.run(id); },
  };
}

test("registration is inert with no independent settings, UI, heartbeat or polling", () => {
  const h = harness();
  assert.equal(h.fetches, 0); assert.equal(h.scopeReads, 0);
  assert.equal(h.calls.length, 0); assert.equal(h.timers.size, 0);
  assert.doesNotMatch(source, /historyTurnRecoveryPollMs|scheduleHistoryTurnRecovery|showToast|codexPlusSettings/);
  for (const file of [
    "apps/codex-plus-manager/src/App.tsx", "crates/codex-plus-core/src/settings.rs",
    "assets/inject/renderer-inject/10-style.js", "assets/inject/renderer-inject/40-backend-settings.js",
  ]) assert.doesNotMatch(readFileSync(path.join(root, file), "utf8"), /codexAppHistoryTurnRecovery|syncCodexPlusHistoryTurnRecovery|历史空轮次补载（实验）/);
});

test("only existing successful repair calls notify frontend; report refresh stays read-only", () => {
  const manager = readFileSync(path.join(root, "apps/codex-plus-manager/src-tauri/src/commands.rs"), "utf8");
  const command = manager.slice(manager.indexOf("pub async fn repair_session_index()"), manager.indexOf("pub async fn load_session_index_repair_report()"));
  assert.match(command, /Ok\(report\) => \{[\s\S]*after_index_repair\([\s\S]*report\.aborted_reason\.is_some\(\)/);
  const reportRead = manager.slice(manager.indexOf("pub async fn load_session_index_repair_report()"), manager.indexOf("pub async fn preview_session_index_cleanup()"));
  assert.doesNotMatch(reportRead, /after_index_repair/);
  const launcher = readFileSync(path.join(root, "apps/codex-plus-launcher/src/main.rs"), "utf8");
  assert.match(launcher, /Ok\(Some\(report\)\) => \{[\s\S]*after_index_repair\(/);
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

test("does not bridge canonical islands or leading and trailing pagination boundaries", async () => {
  const h = harness(conversation([[turn(1), turn(2, true), turn(3)], [turn(4), turn(5, true), turn(6)]]));
  await h.run(); assert.equal(h.calls.length, 0);
});

test("loaded input, evicted items, reconnects and proven empty turns are not gaps", async () => {
  for (const change of [
    (t: any) => { t.params.input.push({ type: "text", text: "fixture" }); },
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

test("single pages continue past agent-only content until user input is recovered", async () => {
  const h = harness();
  let pages = 0;
  h.setLoader(async (_thread, id, all) => {
    assert.equal(all, false);
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "agentMessage" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - ++pages };
    if (pages === 3) (entry.params.input as any[]).push({ type: "text", text: "fixture" });
  });
  assert.equal((await h.run()).status, "completed"); assert.equal(h.calls.length, 3);
});

test("large incomplete turns stop at four single pages", async () => {
  const h = harness();
  h.setLoader(async (_thread, id) => {
    const entry = h.data.turnHistory.history.entitiesByKey[id as string];
    entry.items.push({ type: "agentMessage" });
    (entry.itemsPagination as any).olderCursor = { ordinal: 1000 - h.calls.length };
  });
  assert.equal((await h.run()).status, "partial"); assert.equal(h.calls.length, 4);
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
