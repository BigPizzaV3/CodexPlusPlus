import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const fragment = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const source = fragment("97-whale-widget.js");
type Listener = (event: any) => void;

function fixture({ enabled = true, prefs = {}, failStorage = false } = {}) {
  const timers = new Map<number, { callback: () => void; at: number }>();
  const stored = new Map<string, string>([["codexPlus.whaleWidget.v1", JSON.stringify(prefs)]]);
  let timerId = 0, now = 100000, activeSession = "session-one", activeProfile = "profile-one", settingsEnabled = enabled;
  let quotaFailure = failStorage;
  const requests: Array<{ path: string; payload: any; resolve: (value: any) => void; reject: (error: Error) => void }> = [];
  class Element {
    tagName: string;
    children: Element[] = [];
    parentElement: Element | null = null;
    attrs: Record<string, string> = {};
    dataset: Record<string, string> = {};
    style: any = { setProperty(key: string, value: string) { this[key] = value; } };
    listeners = new Map<string, Set<Listener>>();
    hidden = false; textContent = ""; innerHTML = ""; className = ""; id = ""; type = ""; value = ""; checked = false;
    constructor(tag: string) { this.tagName = tag.toUpperCase(); }
    get isConnected(): boolean { return this === body || this.parentElement?.isConnected === true; }
    append(...children: Element[]) { for (const child of children) this.appendChild(child); }
    appendChild(child: Element) { child.parentElement = this; this.children.push(child); return child; }
    replaceChildren(...children: Element[]) { for (const child of this.children) child.parentElement = null; this.children = []; this.append(...children); }
    remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(child => child !== this); this.parentElement = null; }
    setAttribute(name: string, value: string) { this.attrs[name] = value; }
    addEventListener(name: string, listener: Listener) { if (!this.listeners.has(name)) this.listeners.set(name, new Set()); this.listeners.get(name)!.add(listener); }
    removeEventListener(name: string, listener: Listener) { this.listeners.get(name)?.delete(listener); }
    emit(name: string, event: any = {}) { for (const listener of this.listeners.get(name) || []) listener({ type: name, target: this, preventDefault() {}, ...event }); }
    setPointerCapture() {} focus() {}
  }
  const body = new Element("body");
  const document = Object.assign(new Element("document"), { body, hidden: false, createElement: (tag: string) => new Element(tag) });
  const window: any = Object.assign(new Element("window"), { innerWidth: 1024, innerHeight: 768, __CODEX_PLUS_WHALE_IMAGE__: "data:image/png;base64," + readFileSync(new URL("../../../assets/inject/upstream/whale-widget/DSniang1.png", import.meta.url)).toString("base64") });
  class ClockDate extends Date { static now() { return now; } }
  const context = createContext({
    window, document, Date: ClockDate, Uint8Array,
    localStorage: { getItem: (key: string) => stored.get(key) ?? null, setItem: (key: string, value: string) => { if (quotaFailure) throw new Error("quota"); stored.set(key, value); } },
    codexPlusBackendSettingsLoaded: true, codexPlusBackendSettings: {},
    codexPlusSettings: () => ({ whaleWidget: settingsEnabled }),
    currentSessionRef: () => ({ session_id: activeSession }),
    codexRemoteSessionActiveProfile: () => ({ id: activeProfile }),
    registerCodexPlusExtensionSelector: () => true,
    postJson: (path: string, payload: any) => new Promise((resolve, reject) => requests.push({ path, payload, resolve, reject })),
    setTimeout: (callback: () => void, delay: number) => { const id = ++timerId; timers.set(id, { callback, at: now + delay }); return id; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  const inject = () => runInContext(`(()=>{${source}\nglobalThis.whale={sync:syncCodexPlusWhaleWidget,state:codexPlusWhaleState,poll:codexPlusWhalePoll,save:codexPlusWhaleSavePrefs,normalize:codexPlusWhaleNormalizePrefs,alerts:codexPlusWhaleCheckAlerts,observe:codexPlusWhaleObserveTurn,upload:codexPlusWhaleUpload,settings:codexPlusWhaleSettings,amount:codexPlusWhaleAmount};})()`, context);
  inject();
  const api = () => context.whale as any;
  const settle = async () => { for (let index = 0; index < 6; index++) await Promise.resolve(); };
  return {
    api, inject, document, window, body, timers, stored, requests, settle, context,
    enable(value: boolean) { settingsEnabled = value; api().sync(); },
    switchSession(id: string) { activeSession = id; api().sync(); },
    switchProfile(id: string) { activeProfile = id; api().sync(); },
    tick(ms: number) { now += ms; const callbacks = [...timers].filter(([, item]) => item.at <= now); for (const [id, item] of callbacks) { timers.delete(id); item.callback(); } },
    setQuota(value: boolean) { quotaFailure = value; },
  };
}

const usage = { inputTokens: 100, cachedInputTokens: 40, outputTokens: 20, totalTokens: 120 };
const session = (status = "running", id = "turn-one") => ({ status: "ok", sessionId: "session-one", today: usage, total: usage, lastTurn: { id, status, usage }, rateLimits: [] });
const balance = (total = 10, observedToday: number | null = 2) => ({ status: "ok", provider: { id: "account-one", name: "Provider" }, balances: [{ currency: "USD", total, observedToday }], stale: false });

function settingsFixture() {
  const style = fragment("10-style.js");
  const writes: Array<[string, unknown]> = [];
  const context = createContext({
    window: {}, conversationViewDefaultWidth: 900, codexPlusSettingsKey: "settings", codexPlusBackendSettings: {},
    localStorage: { getItem: () => null }, setBackendSetting: async (key: string, value: unknown) => writes.push([key, value]),
    loadBackendSettings: async () => {}, syncStepwisePanel: () => {},
  });
  runInContext(style.slice(style.indexOf("  function defaultCodexPlusSettings()"), style.indexOf("  // Dream skin runtime")), context);
  runInContext(style.slice(style.indexOf("  function setCodexPlusSetting("), style.indexOf("  function syncStepwisePanel(")), context);
  return { context, writes };
}

test("widget is opt in and follows the enhancement master switch and backend saves", async () => {
  const { context, writes } = settingsFixture();
  assert.equal(context.codexPlusSettings().whaleWidget, false);
  context.codexPlusBackendSettings = { codexAppWhaleWidgetEnabled: true };
  assert.equal(context.codexPlusSettings().whaleWidget, true);
  context.codexPlusBackendSettings.enhancementsEnabled = false;
  assert.equal(context.codexPlusSettings().whaleWidget, false);
  context.setCodexPlusSetting("whaleWidget", true); await Promise.resolve();
  assert.deepEqual(writes, [["codexAppWhaleWidgetEnabled", true]]);
  const f = fixture({ enabled: false }); f.api().sync();
  assert.equal(f.body.children.length, 0); assert.equal(f.requests.length, 0); assert.equal(f.timers.size, 0);
});

test("slow polling has one owner and repeated scans do not duplicate requests", async () => {
  const f = fixture(); f.api().sync();
  assert.equal(f.requests.length, 2); assert.equal(f.timers.size, 3);
  for (let i = 0; i < 20; i++) f.api().sync();
  assert.equal(f.requests.length, 2); assert.equal(f.body.children.length, 1);
  f.requests[0].resolve(balance()); f.requests[1].resolve(session()); await f.settle();
  f.tick(10000); assert.equal(f.requests.length, 3); assert.equal(f.requests[2].path, "/whale/session");
  f.requests[2].resolve(session()); await f.settle();
  f.tick(50000); assert.equal(f.requests.filter(item => item.path === "/whale/balance").length, 2);
  assert.equal(f.timers.size, 3);
});

test("switching threads and disabling discard pending data and history completion", async () => {
  const f = fixture(); f.api().sync();
  const oldSession = f.requests[1];
  f.switchSession("session-two");
  oldSession.resolve(session("completed")); await f.settle();
  assert.equal(f.api().state.session, null); assert.equal(f.api().state.message, "");
  assert.equal(f.requests.at(-1)!.payload.session_id, "session-two");
  const pending = f.requests.at(-1)!; f.enable(false); pending.resolve(session()); await f.settle();
  assert.equal(f.body.children.length, 0); assert.equal(f.api().state.session, null); assert.equal(f.timers.size, 0);
  assert.equal(f.document.listeners.get("visibilitychange")?.size, 0);
});

test("profile changes discard old balance and clear old session state", async () => {
  const f = fixture(); f.api().sync();
  const old = f.requests[0]; f.switchProfile("profile-two"); old.resolve(balance()); await f.settle();
  assert.equal(f.api().state.balance, null); assert.equal(f.api().state.session, null);
  assert.equal(f.requests.filter(item => item.path === "/whale/balance").length, 2);
});

test("hidden pages stop polling and resume without replaying historical completion", async () => {
  const f = fixture(); f.api().sync();
  f.requests[1].resolve(session()); await f.settle();
  f.document.hidden = true; f.document.emit("visibilitychange");
  assert.equal(f.timers.size, 0); assert.equal(f.api().state.previousTurn, null);
  f.requests[0].resolve(balance()); await f.settle(); assert.equal(f.api().state.balance, null);
  f.document.hidden = false; f.document.emit("visibilitychange");
  f.requests.at(-1)!.resolve(session("completed")); await f.settle();
  assert.equal(f.api().state.message, ""); assert.equal(f.timers.size, 1);
});

test("reinjection cleans the old node, timer and listeners", () => {
  const f = fixture(); f.api().sync(); const old = f.api().state;
  f.inject(); f.api().sync();
  assert.equal(old.disposed, true); assert.equal(old.root, null);
  assert.equal(f.body.children.length, 1); assert.equal(f.timers.size, 3);
  assert.equal(f.document.listeners.get("visibilitychange")?.size, 1);
});

test("completion only follows an observed running to completed transition in the same turn", () => {
  const f = fixture(); f.api().sync();
  f.api().observe(session("completed")); assert.equal(f.api().state.message, "");
  f.api().observe(session("running")); f.api().observe(session("completed", "other-turn")); assert.equal(f.api().state.message, "");
  f.api().observe(session("running", "fresh-turn")); f.api().observe(session("completed", "fresh-turn"));
  assert.match(f.api().state.message, /任务完成/);
  f.api().state.message = ""; f.api().observe(session("completed", "fresh-turn")); assert.equal(f.api().state.message, "");
});

test("account thresholds notify once per day and ignore missing or stale balances", () => {
  const f = fixture({ prefs: { thresholds: { USD: { low: 5, budget: 3 } } } }); f.api().sync();
  f.api().alerts({ ...balance(2, 4), stale: true }); assert.equal(f.api().state.message, "");
  f.api().alerts(balance(null as unknown as number, null)); assert.equal(f.api().state.message, "");
  f.api().alerts(balance(2, 4)); assert.match(f.api().state.message, /余额低于/); assert.match(f.api().state.message, /达到预算/);
  f.api().state.message = ""; f.api().alerts(balance(1, 5)); assert.equal(f.api().state.message, "");
  f.inject(); f.api().sync(); f.api().alerts(balance(1, 5)); assert.equal(f.api().state.message, "");
  f.api().alerts({ ...balance(1, 5), provider: { id: "account-two" } }); assert.match(f.api().state.message, /余额低于/);
});

test("unknown values remain unknown and unsupported providers leave Codex usage usable", async () => {
  const f = fixture(); f.api().sync();
  f.requests[0].resolve({ status: "unsupported", message: "ignored implementation detail" });
  f.requests[1].resolve({ ...session(), today: null, total: null }); await f.settle();
  assert.match(f.api().state.elements.balance.textContent, /当前供应商未提供余额数据/);
  assert.doesNotMatch(f.api().state.elements.balance.textContent, /implementation/);
  assert.match(f.api().state.elements.session.textContent, /今日 — tokens · 累计 — tokens/);
  assert.equal(f.api().amount(null), "—"); assert.equal(f.api().amount(0), "0");
});

test("dragging clamps and snaps; taps remain clickable and keyboard movement is bounded", () => {
  const f = fixture(); f.api().sync(); const pet = f.api().state.elements.pet;
  pet.emit("pointerdown", { button: 0, pointerId: 1, clientX: 920, clientY: 680 });
  pet.emit("pointermove", { pointerId: 1, clientX: -1000, clientY: -1000 });
  pet.emit("pointerup", { pointerId: 1 });
  assert.equal(f.api().state.root.style.left, "8px"); assert.equal(f.api().state.root.style.top, "8px");
  pet.emit("click", { detail: 1 }); assert.equal(f.api().state.bubbleOpen, false);
  pet.emit("click", { detail: 0 }); assert.equal(f.api().state.bubbleOpen, true);
  f.api().state.root.emit("keydown", { target: pet, key: "ArrowLeft" }); assert.equal(f.api().state.root.style.left, "8px");
  f.api().state.root.emit("keydown", { target: pet, key: "Escape" }); assert.equal(f.api().state.bubbleOpen, false);
  assert.ok(f.stored.has("codexPlus.whaleWidget.v1"));
});

test("preferences reject remote and SVG images, clamp size and retain plain text", () => {
  const f = fixture();
  for (const image of ["https://example.com/whale.png", "data:image/svg+xml;base64,PHN2Zz4=", "data:image/png;base64,%%%"]) assert.equal(f.api().normalize({ image }).image, "");
  assert.equal(f.api().normalize({ size: 900 }).size, 371.5625);
  assert.equal(f.api().normalize({ phrase: "<script>alert(1)</script>" }).phrase, "<script>alert(1)</script>");
  f.api().sync(); f.api().state.prefs.phrase = "<b>plain text</b>"; f.api().save();
  assert.equal(f.api().state.elements.message.textContent, "<b>plain text</b>");
});

test("quota errors keep the widget interactive and visible; subsequent writes recover", () => {
  const f = fixture({ failStorage: true }); f.api().sync();
  assert.equal(f.api().save(), false); assert.match(f.api().state.elements.notice.textContent, /本地存储已满/);
  f.api().state.elements.pet.emit("click", { detail: 0 }); assert.equal(f.api().state.bubbleOpen, true);
  f.setQuota(false); assert.equal(f.api().save(), true); assert.equal(f.api().state.elements.notice.hidden, true);
});

test("uploads reject oversized, SVG and spoofed image files before decoding", async () => {
  const f = fixture(); f.api().sync();
  await f.api().upload({ type: "image/svg+xml", size: 20 }); assert.match(f.api().state.message, /仅支持/);
  await f.api().upload({ type: "image/png", size: 1024 * 1024 + 1 }); assert.match(f.api().state.message, /仅支持/);
  await f.api().upload({ type: "image/png", size: 12, slice: () => ({ arrayBuffer: async () => new TextEncoder().encode("<svg>bad</svg>").buffer }) });
  assert.match(f.api().state.message, /内容与格式不符/); assert.equal(f.api().state.prefs.image, "");
});


test("balance protocol edits invalidate old units immediately, and account scope separates alerts", async () => {
  const f = fixture({ prefs: { thresholds: { USD: { low: 5 } } } }); f.api().sync();
  const old = f.requests[0];
  f.context.codexPlusBackendSettings.codexAppWhaleBalanceScale = 1000;
  f.api().sync(); old.resolve(balance()); await f.settle();
  assert.equal(f.api().state.balance, null);
  assert.equal(f.requests.filter(item => item.path === "/whale/balance").length, 2);
  f.api().alerts({ ...balance(2), provider: { id: "same-profile", accountId: "scope-first" } });
  f.api().state.message = "";
  f.api().alerts({ ...balance(2), provider: { id: "same-profile", accountId: "scope-second" } });
  assert.match(f.api().state.message, /余额低于/);
});

test("a stuck bridge times out and a later poll recovers without applying its late response", async () => {
  const f = fixture(); f.api().sync(); const old = f.requests[1];
  f.tick(15000); await f.settle();
  assert.equal(f.api().state.session.status, "unavailable");
  f.tick(10000); const next = f.requests.at(-1)!; assert.equal(next.path, "/whale/session");
  next.resolve(session("running", "new-turn")); await f.settle();
  old.resolve(session("completed", "old-turn")); await f.settle();
  assert.equal(f.api().state.session.lastTurn.id, "new-turn");
  f.enable(false); assert.equal(f.timers.size, 0);
});

test("bubble stays inside a narrow viewport when the character is near its middle", () => {
  const f = fixture({ prefs: { x: 130, y: 120 } }); f.window.innerWidth = 400; f.window.innerHeight = 300; f.api().sync();
  const state = f.api().state;
  const left = parseFloat(state.root.style.left) + parseFloat(state.elements.bubble.style.left);
  assert.ok(left >= 8); assert.ok(left + 330 <= 392);
  assert.equal(state.root.dataset.vertical, "above");
});

test("valid local raster upload persists; a late decoder cannot replace a newer reset", async () => {
  const f = fixture(); f.api().sync();
  const pendingImages: any[] = [];
  const data = "data:image/png;base64,iVBORw0KGgo=";
  f.context.FileReader = class { result = data; onload?: () => void; readAsDataURL() { this.onload?.(); } };
  f.context.Image = class { naturalWidth = 24; naturalHeight = 24; onload?: () => void; set src(_value: string) { pendingImages.push(this); } };
  const file = { type: "image/png", size: 8, slice: () => ({ arrayBuffer: async () => new Uint8Array([137,80,78,71,13,10,26,10]).buffer }) };
  const first = f.api().upload(file); await f.settle(); pendingImages[0].onload(); await first;
  assert.equal(f.api().state.prefs.image, data);
  assert.equal(JSON.parse(f.stored.get("codexPlus.whaleWidget.v1")!).image, data);
  const second = f.api().upload(file); await f.settle(); f.api().state.imageRevision += 1; f.api().state.prefs.image = "";
  pendingImages[1].onload(); await second; assert.equal(f.api().state.prefs.image, "");
});


test("native turn events catch short tasks and unsubscribe without double completion", async () => {
  const f = fixture(); const listeners = new Map<string, Set<(value: any) => void>>();
  f.window.__codexPlusRemoteSessionRecoveryDispatcher = { subscribe(method: string, callback: (value: any) => void) {
    if (!listeners.has(method)) listeners.set(method, new Set()); listeners.get(method)!.add(callback);
    return () => listeners.get(method)!.delete(callback);
  } };
  const emit = (method: string, threadId = "session-one", id = "short-turn") => { for (const callback of listeners.get(method) || []) callback({ threadId, turn: { id } }); };
  f.api().sync(); f.api().sync(); assert.equal(listeners.get("turn/started")!.size, 1);
  f.requests[1].resolve(session("completed", "previous-turn")); await f.settle();
  emit("turn/started", "another-session"); emit("turn/completed", "another-session"); assert.equal(f.api().state.message, "");
  emit("turn/started"); emit("turn/completed"); assert.match(f.api().state.message, /任务完成/);
  f.api().state.message = "";
  f.api().observe(session("running", "short-turn")); f.api().observe(session("completed", "short-turn"));
  assert.equal(f.api().state.message, "");
  f.enable(false); assert.equal(listeners.get("turn/started")!.size, 0); assert.equal(listeners.get("turn/completed")!.size, 0);
});


test("failed balance refresh displays its cached value without notifying and labels expired quota snapshots", async () => {
  const f = fixture({ prefs: { thresholds: { USD: { low: 100 } } } }); f.api().sync();
  f.requests[0].resolve({ ...balance(2), status: "unavailable", stale: true, message: "余额记录保存失败，今日观测消费暂不可用。" });
  f.requests[1].resolve({ ...session(), rateLimits: [{ label: "5 小时", usedPercent: 24, resetAt: 1, observedAt: 0 }] });
  await f.settle();
  assert.match(f.api().state.elements.balance.textContent, /缓存，刷新失败/);
  assert.match(f.api().state.elements.balance.textContent, /余额 2 USD/);
  assert.match(f.api().state.elements.balance.textContent, /余额记录保存失败，今日观测消费暂不可用/);
  assert.equal(f.api().state.message, "");
  assert.match(f.api().state.elements.limits.textContent, /额度快照（已过期）/);
  assert.match(f.api().state.elements.limits.textContent, /记录于/);
});


test("the original character is embedded, restore returns to it and the source bubble geometry is retained", () => {
  const f = fixture(); f.api().sync();
  const state = f.api().state;
  assert.equal(state.elements.pet.children[0].src, f.window.__CODEX_PLUS_WHALE_IMAGE__);
  assert.equal(state.elements.pet.children[0].alt, "小鲸鱼娘");
  assert.match(state.elements.pop.innerHTML, /viewBox="0 0 1026 700"/);
  assert.match(state.elements.pop.innerHTML, /M 827 248 A 373 232/);
  assert.equal(parseFloat(state.elements.pop.style.width), state.prefs.size / 0.5945);
  state.prefs.image = "data:image/png;base64,iVBORw0KGgo=";
  f.api().settings();
  state.elements.settings.children.find((item: any) => item.textContent === "恢复默认角色").emit("click");
  assert.equal(state.elements.pet.children[0].src, f.window.__CODEX_PLUS_WHALE_IMAGE__);
});

test("press release and cancellation restore the original squash while menu and bubble stay separate", () => {
  const f = fixture(); f.api().sync(); const state = f.api().state, pet = state.elements.pet;
  pet.emit("pointerdown", {button: 0, pointerId: 1, clientX: 900, clientY: 680});
  assert.equal(state.root.dataset.pressed, "true");
  pet.emit("pointercancel", {pointerId: 1}); assert.equal(state.root.dataset.pressed, "false");
  pet.emit("click", {detail: 0}); assert.equal(state.elements.pop.hidden, false); assert.equal(state.elements.bubble.hidden, true);
  pet.emit("contextmenu"); assert.equal(state.elements.bubble.hidden, false); assert.equal(state.elements.pop.hidden, true);
  state.root.emit("keydown", {target: pet, key: "Escape"}); assert.equal(state.elements.bubble.hidden, true);
});

test("placeholder defaults migrate without replacing uploaded characters and large sprites fit a small viewport", () => {
  const f = fixture();
  assert.equal(f.api().normalize({version: 1, size: 88}).size, f.api().normalize({}).size);
  assert.equal(f.api().normalize({version: 1, size: 88, image: "data:image/png;base64,iVBORw0KGgo="}).size, 88);
  f.window.innerWidth = 240; f.window.innerHeight = 260; f.api().state.prefs.size = 371.5625; f.api().sync();
  const state = f.api().state;
  assert.ok(parseFloat(state.root.style.width) < 240);
  assert.ok(parseFloat(state.elements.pop.style.width) <= 224);
});
