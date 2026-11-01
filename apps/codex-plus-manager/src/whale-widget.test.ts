import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
const packageFile = (name: string) => readFileSync(new URL(`../../../assets/extensions/agent-pet/${name}`, import.meta.url), "utf8");
/** 大肥鱼在 Agent 桌宠拓展包里：适配层 + 挂件，只用公开 ctx 接口（config、call、on）与 codexPlus.currentSession。 */
const source = packageFile("whale-compat.js") + "\n" + packageFile("whale.js");
const ya1 = "data:audio/mpeg;base64," + readFileSync(new URL("../../../assets/inject/upstream/whale-widget/Ya1.mp3", import.meta.url)).toString("base64");
type Listener = (event: any) => void;
function fixture({ enabled = true, legacy = null as any, savedConfig = true, preloadMedia = true } = {}) {
  let mediaRequests = 0;
  let now = 100000, serial = 0, sessionId = "session-one", sizeSaves = 0;
  // 拓展配置：kind 为 whaleWidget 时挂载大肥鱼；余额设置变了就重建。
  let config: Record<string, unknown> = { kind: enabled ? "whaleWidget" : "image" };
  const timers = new Map<number, () => void>(), storage = new Map<string, string>(), requests: any[] = [], dispatcherListeners = new Map<string, Listener[]>();
  if (legacy) storage.set("codexPlus.whaleWidget.v1", JSON.stringify(legacy));
  class Element {
    tagName: string; children: any[] = []; parentNode: any = null; attrs: any = {}; value = ""; textContent = ""; className = "";
    listeners = new Map<string, Set<Listener>>(); style: any = { setProperty() {} };
    constructor(tag: string) { this.tagName = tag.toUpperCase(); }
    appendChild(node: any) { node.parentNode?.removeChild(node); node.parentNode = this; this.children.push(node); return node; }
    removeChild(node: any) { this.children = this.children.filter((n) => n !== node); node.parentNode = null; return node; }
    setAttribute(name: string, value: string) { this.attrs[name] = value; }
    getAttribute(name: string) { return this.attrs[name]; }
    remove() { this.parentNode?.removeChild(this); }
    addEventListener(name: string, listener: Listener) { const set = this.listeners.get(name) || new Set(); set.add(listener); this.listeners.set(name, set); }
    removeEventListener(name: string, listener: Listener) { this.listeners.get(name)?.delete(listener); }
    emit(name: string, event: any = {}) { for (const listener of this.listeners.get(name) || []) listener({ type: name, ...event }); }
    dispatchEvent(event: any) { this.emit(event.type, event); }
  }
  const body = new Element("body"), head = new Element("head"), document = Object.assign(new Element("document"), { body, head, hidden: false, createElement: (tag: string) => new Element(tag), createElementNS: (_ns: string, tag: string) => new Element(tag) });
  const audioContexts: any[] = [], observers: any[] = [];
  class Context { closed = false; suspended = false; constructor() { audioContexts.push(this); } close() { this.closed = true; return Promise.resolve(); } suspend() { this.suspended = true; return Promise.resolve(); } }
  class Observer { disconnected = false; constructor(_callback: Listener) { observers.push(this); } observe() {} disconnect() { this.disconnected = true; } }
  const window: any = Object.assign(new Element("window"), { innerWidth: 1024, innerHeight: 768, location: { origin: "https://codex.test" }, fetch, AudioContext: Context, MutationObserver: Observer, Image: class extends Element { constructor() { super("img"); } }, Audio: class extends Element { constructor() { super("audio"); } } });
  // 拓展 ctx 的最小替身：config 读当前配置、call 记下请求、on 订阅 codex/<method> 事件。
  const ctx = {
    get config() { return config; },
    call: (path: string, payload: any) => {
      // 内嵌素材经 /whale/media 取一次，不算进数据请求。
      if (path === "/whale/media") return mediaRequests++, Promise.resolve({ status: "ok", image: "data:image/png;base64,AAAA", assets: { "Ya1.mp3": ya1 } });
      requests.push({ path, payload }); return responder(path, payload);
    },
    on: (event: string, callback: Listener) => { const method = event.replace(/^codex\//, ""); const list = dispatcherListeners.get(method) || []; list.push(callback); dispatcherListeners.set(method, list); return () => dispatcherListeners.set(method, (dispatcherListeners.get(method) || []).filter((item) => item !== callback)); },
  };
  const amounts = { inputTokens: 100, cachedInputTokens: 40, outputTokens: 20, totalTokens: 120 };
  const history: any = { status: "ok", complete: true, periods: Object.fromEntries(["today", "week", "month", "all"].map((key) => [key, { usage: amounts }])), models: [{ model: "gpt-test" }], codex: { ok: true, sessions: 1, days7: [{ date: "2026-10-08", tokens: 120 }] }, today: { modelTotal: null, models: [{ model: "gpt-test", cost: null }] }, days7: [], all: { days: [], events: [] }, records: [], rateLimits: [] };
  const session: any = { status: "ok", sessionId, model: "gpt-test", total: amounts, today: amounts, lastTurn: { id: "turn-one", status: "running", usage: amounts }, rateLimits: [] };
  const full: any = { "/dsh-whale/size.json": { ok: true, hasSavedConfig: savedConfig, sound: false, scale: 1.5 }, "/dsh-whale/api-models.json": { ok: true, codexStatsOn: true, models: [{ id: "codex", codex: {} }], relayProfiles: [{ id: "profile-one", name: "Test profile" }] }, "/dsh-whale/usage-records.json": { ok: true, today: { total: 8, currency: "USD", source: "balance-corrected" }, days7: [], all: { days: [], events: [] } }, "/dsh-whale/roles.json": { ok: true, roles: [{ id: "default", name: "小鲸鱼" }] }, "/dsh-whale/bubble.json": { ok: true, config: { items: [] } }, "/dsh-whale/usage-settings.json": { ok: true, settings: {} } };
  let responder: any = async (path: string, payload: any) => {
    if (path === "/whale/session") return structuredClone(session);
    if (path === "/whale/history") return structuredClone(history);
    if (payload.path === "/dsh-whale/size.json" && payload.method === "PUT") { sizeSaves++; full[payload.path] = { ...payload.body, ok: true, hasSavedConfig: true }; }
    if (payload.path === "/dsh-whale/roles.json" && payload.method === "POST") full[payload.path].roles.push({ id: "imported", name: payload.body.name, url: "/dsh-whale/role-image.png?id=imported" });
    return { status: 200, body: structuredClone(full[payload.path] || { ok: true }) };
  };
  const engine = (...env: any[]) => {
    const [win, doc, _fetch, _storage, timeout, _clear, interval, _clearInterval, raf, _cancel, Mutation, _Resize, Audio] = env;
    const root = doc.createElement("div"); win.__codexPlusWhaleHost.attach(root); doc.body.appendChild(root); doc.head.appendChild(doc.createElement("style"));
    win.addEventListener("pointerdown", () => {}); doc.addEventListener("keydown", () => {});
    timeout(() => {}, 100); interval(() => {}, 1000); raf(() => {});
    new Mutation(() => {}).observe(doc.body); new Audio();
    win.__codexPlusWhaleHost.bind({ root: doc.body.children.at(-1), pollWaitState() {}, pollLastTurn() {}, refresh() {} });
  };
  class ClockDate extends Date { static now() { return now; } }
  const context = createContext({ window, document, console, Response, Event, Uint8Array, URL, atob, fetch, Date: ClockDate, globalThis: undefined, ctx,
    codexPlus: { currentSession: () => ({ id: sessionId, title: "", hostId: "local", remote: false }) }, codexPlusWhaleEngine: engine,
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value), removeItem: (key: string) => storage.delete(key) },
    setTimeout: (callback: () => void) => { const id = ++serial; timers.set(id, callback); return id; }, clearTimeout: (id: number) => timers.delete(id), requestAnimationFrame: (callback: () => void) => { const id = ++serial; timers.set(id, callback); return id; }, cancelAnimationFrame: (id: number) => timers.delete(id),
  });
  // 重新注入 = 拓展卸下再装上：先撤掉上一个挂件。直接建的运行时（create）用挂件同一个数据平台，素材预先就位。
  const inject = () => runInContext(`(()=>{window.fixture?.widget.dispose();${source}\n${preloadMedia ? `codexPlusWhaleMedia.assets={"Ya1.mp3":${JSON.stringify(ya1)}};` : ""}const widget=createCodexPlusWhaleWidget(ctx);window.fixture={widget,sync:()=>widget.sync(),get state(){return{runtime:widget.runtime,notice:widget.notice};},create:()=>createCodexPlusWhaleFullRuntime(widget.platform)};})()`, context);
  inject();
  const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
  return { window, document, body, head, timers, storage, requests, audioContexts, observers, history, session, full, context, settle, inject, get state() { return window.fixture.state; }, get runtime() { return window.fixture.state.runtime; }, create() { return window.fixture.create(); }, sync() { window.fixture.sync(); }, enable(value: boolean) { config = { ...config, kind: value ? "whaleWidget" : "image" }; window.fixture.sync(); }, configure(next: Record<string, unknown>) { config = next; window.fixture.sync(); }, balance(patch: Record<string, unknown>) { config = { ...config, ...patch }; window.fixture.sync(); }, thread(value: string) { sessionId = value; window.fixture.sync(); }, emit(method: string, params: any, id?: string) { for (const callback of dispatcherListeners.get(method) || []) callback(id ? { id, params } : params); }, get sizeSaves() { return sizeSaves; }, get mediaRequests() { return mediaRequests; }, responder(fn: any) { responder = fn; }, tick(ms: number) { now += ms; }, dispatcherListeners };
}
test("widget mounts only when the extension config selects the whale", async () => {
  // 没配置类型、选了图片桌宠：都不挂载、不发请求、不起定时器。
  for (const config of [{}, { kind: "image" }, { kind: "dragon" }]) {
    const f = fixture({ enabled: false }); f.configure(config); await f.settle();
    assert.equal(f.requests.length, 0, JSON.stringify(config)); assert.equal(f.timers.size, 0); assert.equal(f.body.children.length, 0);
  }
  const f = fixture({ enabled: false });
  f.configure({ kind: "whaleWidget" }); await f.settle();
  assert.equal(f.body.children.length, 1);
  // 切回图片桌宠：大肥鱼完整卸载（两者互斥）。
  f.configure({ kind: "image" });
  assert.equal(f.body.children.length, 0); assert.equal(f.head.children.length, 0); assert.equal(f.timers.size, 0);
});

test("complete upstream engine is preserved with verified Codex adapter anchors", () => {
  const raw = readFileSync(new URL("../../../assets/inject/upstream/whale-widget/full-widget.js", import.meta.url), "utf8");
  assert.equal(createHash("sha256").update(raw).digest("hex"), "391806b8d4fd7c77a1711e02f58360e641f97b97bfb9085049031365007301b0");
  const full = readFileSync(new URL("../../../assets/inject/upstream/whale-widget/full-widget-codex.js", import.meta.url), "utf8");
  assert.doesNotThrow(() => new Function(full));
  for (const entry of ["openSoundSettingsPanel", "bubbleModuleWizard", "bubblePickChoiceStep", "confirmAudioCrop", "confirmCrop", "openSnapModal", "openResManager"]) assert.match(full, new RegExp("function " + entry));
});

test("repeated scans mount one complete engine; disable frees every native resource", async () => {
  const f = fixture(); f.sync(); await f.settle();
  for (let i = 0; i < 20; i++) f.sync();
  assert.equal(f.body.children.length, 1); assert.equal(f.head.children.length, 1);
  assert.equal(f.requests.filter((r) => r.payload?.path === "/dsh-whale/size.json").length, 1);
  assert.equal(f.window.__dshWhaleInit, undefined);
  assert.equal(f.runtime.state.nodes.size, 2);
  f.enable(false);
  assert.equal(f.body.children.length, 0); assert.equal(f.head.children.length, 0); assert.equal(f.timers.size, 0);
  assert.ok(f.audioContexts.every((audio) => audio.closed)); assert.ok(f.observers.every((observer) => observer.disconnected));
  assert.ok([...f.document.listeners.values(), ...f.window.listeners.values()].every((listeners) => listeners.size === 0));
});

test("balance setting change and reinjection clean old scope and preserve opt-in state", async () => {
  const f = fixture(); f.sync(); await f.settle(); const old = f.runtime;
  f.balance({ codexAppWhaleBalanceProtocol: "off" }); await f.settle(); assert.equal(old.state.disposed, true); assert.equal(f.body.children.length, 1);
  f.inject(); f.sync(); await f.settle(); assert.equal(f.body.children.length, 1); assert.equal(f.head.children.length, 1);
});

test("hidden windows suspend timers, observers, and audio; bridge polling is gated", async () => {
  const f = fixture(); f.sync(); await f.settle(); const runtime = f.runtime;
  f.document.hidden = true; f.document.emit("visibilitychange");
  assert.equal(f.timers.size, 0); assert.ok(f.audioContexts.every((audio) => audio.suspended));
  const count = f.requests.length; await assert.rejects(runtime.fetch("/dsh-whale/balance.json")); assert.equal(f.requests.length, count);
  f.document.hidden = false; f.document.emit("visibilitychange"); assert.ok(f.timers.size > 0);
});

test("actual builtin audio bytes use local assets; custom groups never fall back to duck", async () => {
  const f = fixture(); const runtime = f.create();
  const response = await runtime.fetch("/dsh-whale/audio-fragment.wav?id=ya1");
  const original = readFileSync(new URL("../../../assets/inject/upstream/whale-widget/Ya1.mp3", import.meta.url));
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), original); assert.equal(f.requests.length, 0);
  await runtime.fetch("/dsh-whale/sound/press.mp3?set=custom-group");
  assert.equal(f.requests.at(-1).payload.path, "/dsh-whale/sound/press.mp3");
  await assert.rejects(runtime.fetch("https://external.example/key")); runtime.dispose();
});

test("embedded audio remains playable when native data fetch is blocked by the host CSP", async () => {
  const f = fixture(); let networkCalls = 0;
  f.window.fetch = async () => { networkCalls++; throw new Error("connect-src blocks data URLs"); };
  const runtime = f.create();
  const original = readFileSync(new URL("../../../assets/inject/upstream/whale-widget/Ya1.mp3", import.meta.url));
  const response = await runtime.fetch("/dsh-whale/audio-fragment.wav?id=ya1");
  assert.equal(response.headers.get("Content-Type"), "audio/mpeg");
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), original);
  const direct = await runtime.fetch(ya1);
  assert.deepEqual(Buffer.from(await direct.arrayBuffer()), original);
  assert.equal(networkCalls, 0); assert.equal(f.requests.length, 0);
  runtime.dispose();
});

test("request method, query and body round trip through the public route with the extension's balance settings", async () => {
  const f = fixture(); f.balance({ codexAppWhaleBalanceProtocol: "custom", codexAppWhaleBalancePath: "/user/balance" }); const runtime = f.create();
  await runtime.fetch("/dsh-whale/audio.json?set=one", { method: "POST", body: JSON.stringify({ action: "save-group", name: "Test", press: "ya1", release: "" }) });
  const request = f.requests.find((item) => item.payload?.path === "/dsh-whale/audio.json");
  assert.equal(request.path, "/whale/full");
  assert.deepEqual(JSON.parse(JSON.stringify(request.payload)), { path: "/dsh-whale/audio.json", method: "POST", query: { set: "one" }, body: { action: "save-group", name: "Test", press: "ya1", release: "" }, balance: { protocol: "custom", path: "/user/balance", field: "", currency: "USD", scale: 1 } }); runtime.dispose();
});

test("fast native turns notify once and thread changes align historical completion", async () => {
  const f = fixture(); const runtime = f.create(); runtime.subscribe();
  f.emit("turn/started", { threadId: "session-one", turn: { id: "fast", status: "running" } });
  f.emit("turn/completed", { threadId: "session-one", turn: { id: "fast", status: "completed" } });
  assert.equal(runtime.state.seq, 1);
  f.emit("turn/completed", { threadId: "session-one", turn: { id: "fast", status: "completed" } }); assert.equal(runtime.state.seq, 1);
  f.emit("turn/started", { threadId: "other-thread", turn: { id: "unrelated" } }); assert.equal(runtime.state.turn.id, "fast"); runtime.dispose();
});

test("approval and input sound events clear when actual request is resolved", () => {
  const f = fixture(); const runtime = f.create(); runtime.subscribe();
  runtime.nativeEnvelope({ type: "mcp-request", message: { method: "item/tool/requestUserInput", id: "request-1", params: { threadId: "session-one", itemId: "question-one" } } });
  assert.equal(runtime.state.pendingWait.kind, "question");
  runtime.nativeEnvelope({ method: "serverRequest/resolved", params: { threadId: "other-thread", requestId: "request-1" } }); assert.ok(runtime.state.pendingWait);
  runtime.nativeEnvelope({ method: "serverRequest/resolved", params: { threadId: "session-one", requestId: "request-1" } }); assert.equal(runtime.state.pendingWait, null);
  f.emit("item/commandExecution/requestApproval", { threadId: "session-one", itemId: "command-one" }, "approve-one");
  runtime.nativeEnvelope({ message: { id: "approve-one", result: { decision: "accept" } } }); assert.equal(runtime.state.pendingWait, null); runtime.dispose();
});

test("configured prices retain matching patterns and unknown prices remain unknown", async () => {
  const f = fixture(); const runtime = f.create();
  runtime.state.models = [{ id: "provider", matchIds: ["gpt"], price: { hit: "1", miss: "2", out: "3", cur: "USD", rate: "7" } }];
  runtime.state.session = f.session; await runtime.history();
  assert.deepEqual(JSON.parse(JSON.stringify(f.requests.at(-1).payload.prices)), [{ model: "gpt", currency: "CNY", input: 14, cachedInput: 7, output: 21 }]);
  runtime.state.models[0].price.hit = ""; assert.equal(runtime.prices().length, 0);
  assert.equal(runtime.host.completionCost(null), "用量暂不可用");
  runtime.host.complete({ amount: null, usage: { totalTokens: 120 } }, () => {}); assert.equal(runtime.host.completionCost(null), "120 tokens"); runtime.dispose();
});

test("dollar quotes require an explicit exchange rate and disabled statistics read no logs", async () => {
  const f = fixture(); const runtime = f.create();
  runtime.state.models = [{ id: "provider", matchIds: ["gpt"], price: { hit: 1, miss: 2, out: 3, cur: "USD" } }];
  assert.equal(runtime.prices().length, 0);
  runtime.state.codexStatsOn = false; runtime.subscribe();
  f.emit("turn/started", { threadId: "session-one", turn: { id: "native-only" } });
  f.emit("turn/completed", { threadId: "session-one", turn: { id: "native-only", status: "completed" } });
  await runtime.session(); await runtime.history();
  assert.equal(f.requests.length, 0); assert.equal(runtime.state.seq, 1); runtime.dispose();
});

test("automatic quota and token modules preserve unknown values while indexing", () => {
  const full = readFileSync(new URL("../../../assets/inject/upstream/whale-widget/full-widget-codex.js", import.meta.url), "utf8");
  const context = createContext({ apiQuotaOf: () => ({ mode: "auto", total: 100, autoUsed: null }) });
  runInContext(full.slice(full.indexOf("function apiQuotaInfo("), full.indexOf("function apiQuotaResetText(")), context);
  assert.equal(context.apiQuotaInfo("test").used, null); assert.equal(context.apiQuotaInfo("test").left, null);
  assert.equal(context.apiQuotaPctText("test"), "—"); assert.equal(context.apiQuotaTotalText("test"), "100");
});

test("local cost estimates preserve account ledger and corrected currency", async () => {
  const f = fixture(); const runtime = f.create(); f.history.today.modelTotal = 3;
  const response = await runtime.fetch("/dsh-whale/usage-records.json"), data = await response.json();
  assert.equal(data.today.total, 8); assert.equal(data.today.currency, "USD"); assert.equal(data.today.source, "balance-corrected"); assert.equal(data.today.modelTotal, 3); runtime.dispose();
});

test("partial scans are visibly marked and expose only actual token totals", async () => {
  const f = fixture(); const runtime = f.create(); f.history.status = "partial"; f.history.complete = false;
  const data = await (await runtime.fetch("/dsh-whale/api-models.json")).json();
  assert.equal(data.models[0].codex.complete, false); assert.ok(data.models[0].codex.deferred > 0);
  assert.equal(runtime.host.tokens("today"), "120"); runtime.dispose();
});

test("legacy uploaded role and explicit mute migrate once without replacing saved full settings", async () => {
  const f = fixture({ legacy: { image: "data:image/png;base64,aGVsbG8=", size: 100, x: 800, y: 600, sound: false, phrase: "陪你写代码", thresholds: { USD: { low: 5 } } }, savedConfig: false });
  f.sync(); await f.settle(); assert.equal(f.storage.get("codexPlus.whale.full.v1.dshw-role"), "imported");
  assert.equal(f.full["/dsh-whale/size.json"].sound, false); assert.equal(f.sizeSaves, 1);
  f.enable(false); f.enable(true); await f.settle(); assert.equal(f.sizeSaves, 1);
  const existing = fixture({ legacy: { sound: true, image: "data:image/png;base64,aGVsbG8=" }, savedConfig: true }); existing.sync(); await existing.settle(); assert.equal(existing.sizeSaves, 0);
});

test("disposal rejects late media and prevents async style reattachment", async () => {
  const f = fixture(); const runtime = f.create(); const doc = runtime.environment[1]; const node = doc.createElement("style"); runtime.dispose();
  doc.head.appendChild(node); assert.equal(f.head.children.length, 0); assert.throws(() => doc.createElement("div"));
});

test("initialization errors have visible feedback and bounded retry", async () => {
  const f = fixture(); f.responder(async () => { throw new Error("test bridge offline"); }); f.sync(); await f.settle();
  assert.ok(f.state.notice); assert.match(f.state.notice.textContent, /10 秒/);
  const count = f.requests.length; for (let i = 0; i < 100; i++) f.sync(); assert.equal(f.requests.length, count);
  f.tick(10000); f.responder(async () => ({ status: 200, body: f.full["/dsh-whale/size.json"] })); f.sync(); await f.settle();
  assert.ok(f.runtime.state.controls); assert.equal(f.state.notice, null); f.enable(false);
});

test("native short turns are observed while initial configuration is pending", async () => {
  const f = fixture(); let resolve!: (value: any) => void;
  f.responder(() => new Promise((r) => { resolve = r; })); f.sync();
  f.emit("turn/started", { threadId: "session-one", turn: { id: "during-init" } });
  f.emit("turn/completed", { threadId: "session-one", turn: { id: "during-init", status: "completed" } });
  assert.equal(f.runtime.state.seq, 1);
  resolve({ status: 200, body: f.full["/dsh-whale/size.json"] }); await f.settle();
  assert.equal(f.runtime.state.seq, 1); assert.ok(f.runtime.state.completed.has("during-init")); f.enable(false);
});

test("price edits invalidate old in-flight estimates before they can repaint", async () => {
  const f = fixture(); const runtime = f.create(); let resolveOld!: (value: any) => void, historyCalls = 0;
  runtime.state.models = [{ id: "model", matchIds: ["gpt"], price: { hit: 1, miss: 2, out: 3, cur: "CNY" } }];
  f.responder(async (path: string) => {
    if (path === "/whale/history") {
      if (++historyCalls === 1) return new Promise((resolve) => { resolveOld = resolve; });
      return { ...f.history, priceMarker: "new" };
    }
    return { status: 200, body: { ok: true, models: [{ id: "model", matchIds: ["gpt"], price: { hit: 7, miss: 8, out: 9, cur: "CNY" } }] } };
  });
  const old = runtime.history();
  await runtime.fetch("/dsh-whale/api-models.json", { method: "POST", body: JSON.stringify({ action: "save" }) });
  await runtime.history(); resolveOld({ ...f.history, priceMarker: "old" }); await old;
  assert.equal(runtime.state.history.priceMarker, "new"); runtime.dispose();
});

test("turning statistics off discards both pending log summaries", async () => {
  const f = fixture(); const runtime = f.create(); let resolveHistory!: (value: any) => void, resolveSession!: (value: any) => void;
  f.responder((path: string) => {
    if (path === "/whale/history") return new Promise((resolve) => { resolveHistory = resolve; });
    if (path === "/whale/session") return new Promise((resolve) => { resolveSession = resolve; });
    return Promise.resolve({ status: 200, body: { ok: true } });
  });
  const history = runtime.history(), session = runtime.session();
  await runtime.fetch("/dsh-whale/size.json", { method: "PUT", body: JSON.stringify({ codexStatsOn: false }) });
  resolveHistory(f.history); resolveSession(f.session); await Promise.all([history, session]);
  assert.equal(runtime.state.history, null); assert.equal(runtime.state.session, null); assert.equal(runtime.host.tokens("today"), "—"); runtime.dispose();
});

test("first mount fetches the embedded media once through /whale/media", async () => {
  const f = fixture({ preloadMedia: false }); f.sync(); await f.settle();
  assert.equal(f.mediaRequests, 1); assert.equal(f.body.children.length, 1);
  f.enable(false); f.enable(true); await f.settle();
  assert.equal(f.mediaRequests, 1, "素材只取一次");
});
