import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// 内置拓展挂在拓展内核上：扫描步骤由拓展登记、卸下即撤；打字特效的全局监听随拓展装卸；内置数据作为服务提供。
const read = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const builtins = read("96-builtin-extensions.js");
const scan = read("95-conversation-view.js");
const sideChat = read("85-side-chat-save.js");
const paste = read("96-paste-fix.js");
const extract = (source: string, name: string) => {
  const body = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name);
  return body;
};

function harness(windowState: Record<string, unknown> = {}) {
  const calls: string[] = [];
  // 拓展内核的事件、中间件、DOM 监听：按内核语义登记，卸载时撤销。
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const interceptors = new Map<string, Array<(value: any, next: (value?: any) => any) => any>>();
  const domListeners: Array<{ target: unknown; type: string }> = [];
  const services = new Map<string, { value: any; owner: string }>();
  const handles: Array<{ id: string; dispose(): void }> = [];
  const known: Record<string, unknown> = {
    window: { __codexPlusBuiltinExtensionState: { disabled: [], replacedBy: {} }, ...windowState },
    document: { id: "document" },
    codexPlusServices: services,
    codexPlusProvideService: (name: string, value: unknown, owner: string) => services.set(name, { value, owner }),
    codexPlusMarkExtensionFailure: (key: string, message: string) => calls.push(`fail:${key}:${message}`),
    codexPlusCreateExtension(definition: any) {
      const disposers: Array<() => void> = [];
      const track = (dispose: () => void) => { disposers.push(dispose); return dispose; };
      const ctx = {
        effect: (fn: () => () => void) => { const dispose = fn(); if (typeof dispose === "function") disposers.push(dispose); return dispose; },
        on(event: string, handler: (...args: any[]) => void) {
          if (!listeners.has(event)) listeners.set(event, new Set());
          listeners.get(event)!.add(handler);
          return track(() => listeners.get(event)!.delete(handler));
        },
        intercept(name: string, handler: (value: any, next: (value?: any) => any) => any) {
          interceptors.set(name, [...(interceptors.get(name) || []), handler]);
          return track(() => interceptors.set(name, interceptors.get(name)!.filter((item) => item !== handler)));
        },
        listen(target: unknown, type: string) {
          const entry = { target, type };
          domListeners.push(entry);
          return track(() => domListeners.splice(domListeners.indexOf(entry), 1));
        },
        registerSlot: () => () => {},
        setTimeout: () => () => {},
      };
      definition.apply(ctx);
      const handle = { id: definition.id, dispose: () => disposers.reverse().forEach((dispose) => dispose()) };
      handles.push(handle);
      return handle;
    },
    installCodexPlusTypingEffects: () => calls.push("typing:install"),
    uninstallCodexPlusTypingEffects: () => calls.push("typing:uninstall"),
    refreshDreamSkin: () => calls.push("dream:refresh"),
    syncCodexPlusCapacityAutoRetry: () => calls.push("capacity:sync"),
    syncWeixinDesktopThreads: async () => { calls.push("weixin:sync"); },
    codexPlusSettings: () => ({ conversationView: false }),
    codexPlusUsageState: { cache: new Map([["t1", [{ total_tokens: 10 }]]]) },
    codexPlusUsageSnapshot: (events: unknown[] | undefined) => (events ? { events: events.length } : null),
    codexPlusProxyStatusState: { requests: [{ id: "r1" }] },
    codexPlusProxyStatusPostWithTimeout: async () => ({ status: "ok", requests: [{ id: "r2" }] }),
    codexPlusCurrentSessionId: () => "t1",
  };
  // 其余被引用的功能函数一律当作无副作用的桩。
  const scope = new Proxy(known, {
    has: (_target, key) => typeof key === "string" && !["globalThis", "Map", "Set", "Object", "Array", "String", "Error", "Promise"].includes(key),
    get: (target, key) => (key in target ? target[key as string] : key === Symbol.unscopables ? undefined : () => undefined),
  });
  const source = [
    extract(scan, "codexPlusScanSteps"), extract(scan, "registerCodexPlusScanStep"), extract(scan, "runCodexPlusScanSteps"), extract(scan, "runScanStep"),
    extract(sideChat, "codexPlusSideChatSaveExtension"), extract(paste, "codexPlusPasteFixExtension"),
    builtins,
  ].join("\n");
  const api = new Function("scope", `with (scope) { ${source}\n return { sync: syncCodexPlusBuiltinExtensions, runSteps: runCodexPlusScanSteps, steps: codexPlusScanSteps }; }`)(scope);
  const emit = (event: string, ...args: unknown[]) => [...(listeners.get(event) || [])].forEach((handler) => handler(...args));
  const chain = (name: string, value: any) => {
    const items = interceptors.get(name) || [];
    const step = (index: number, current: any): any => (index >= items.length ? current : items[index](current, (next = current) => step(index + 1, next)));
    return step(0, value);
  };
  return { api, calls, services, known, listeners, interceptors, domListeners, emit, chain };
}

test("内置拓展装上时登记扫描步骤、挂打字特效监听；停用后撤掉", () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const { api, calls, known } = harness();
  api.sync();
  assert.ok(calls.includes("typing:install"));
  assert.ok(api.steps().has("dream-skin") && api.steps().has("side-chat-save") && api.steps().has("agent-pet"));
  assert.ok(api.steps().has("dictation") && api.steps().has("plugin-market-unlock"));
  assert.equal(api.steps().get("turn-rewind").phase, "light", "轮次回退跟着轻量扫描");
  calls.length = 0;
  api.runSteps("light");
  assert.ok(!calls.includes("dream:refresh"), "轻量扫描只跑 light 步骤");
  calls.length = 0;
  api.runSteps();
  assert.ok(calls.includes("dream:refresh"), "扫描循环经登记的步骤刷新梦皮");
  (known.window as any).__codexPlusBuiltinExtensionState = { disabled: ["dream-skin", "typing-effects"], replacedBy: {} };
  calls.length = 0;
  api.sync();
  assert.ok(!api.steps().has("dream-skin"), "停用后扫描不再点名它");
  assert.ok(calls.includes("typing:uninstall"), "停用后摘掉打字特效的全局监听");
  calls.length = 0;
  api.runSteps();
  assert.ok(!calls.includes("dream:refresh"));
});

test("内置数据作为服务提供，界面那一层被顶替也能拿到", async () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  const { api, services } = harness();
  api.sync();
  assert.equal(services.get("codexpp.usage")?.owner, "builtin");
  assert.deepEqual(services.get("codexpp.usage")?.value.cached(), { events: 1 }, "默认取当前会话");
  assert.equal(services.get("codexpp.usage")?.value.cached("nope"), null);
  assert.deepEqual(services.get("codexpp.proxyStatus")?.value.requests(), [{ id: "r1" }]);
  assert.deepEqual(await services.get("codexpp.proxyStatus")?.value.fetch(), [{ id: "r2" }]);
});

test("读到设置后由内置拓展经 settings/loaded 自己刷新；停用后不再收到", () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const { api, calls, known, emit } = harness();
  api.sync();
  calls.length = 0;
  emit("settings/loaded", { reason: "heartbeat" });
  for (const call of ["dream:refresh", "capacity:sync", "weixin:sync"]) assert.ok(calls.includes(call), call);
  (known.window as any).__codexPlusBuiltinExtensionState = { disabled: ["weixin-desktop-sync", "capacity-auto-retry"], replacedBy: {} };
  api.sync();
  assert.ok(calls.includes("capacity:sync"), "卸下时同步一次，让定时器按停用清掉");
  calls.length = 0;
  emit("settings/loaded", { reason: "heartbeat" });
  assert.ok(calls.includes("dream:refresh"));
  assert.ok(!calls.includes("weixin:sync") && !calls.includes("capacity:sync"), "停用的内置拓展不再收到设置事件");
});

test("侧聊保存挂在公开的 codex/exchange 中间件上，粘贴修复的监听随拓展装卸", async () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const { api, calls, known, interceptors, domListeners, chain } = harness();
  known.sendCodexSideChatNativeRequest = async (client: any, method: string, _params: unknown, send: () => Promise<string>) => {
    calls.push(`side:${client.hostId}:${method}`);
    return send();
  };
  api.sync();
  assert.equal(interceptors.get("codex/exchange")?.length, 1);
  const exchange = chain("codex/exchange", { method: "thread/fork", params: {}, hostId: "local", send: async () => "sent" });
  assert.equal(await exchange.send(), "sent");
  assert.ok(calls.includes("side:local:thread/fork"));
  assert.deepEqual(domListeners.map((entry) => entry.type), ["paste"]);
  (known.window as any).__codexPlusBuiltinExtensionState = { disabled: [], replacedBy: { "side-chat-save": "user:my-side", "paste-fix": "user:my-paste" } };
  api.sync();
  assert.equal(interceptors.get("codex/exchange")?.length, 0, "被顶替后中间件撤掉");
  assert.equal(domListeners.length, 0, "被顶替后粘贴监听摘掉");
});

test("同页重复注入时先卸下上一份注入包装上的内置拓展", () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const first = harness();
  first.api.sync();
  assert.equal(first.domListeners.length, 1);
  const second = harness({ __codexPlusBuiltinExtensionHandles: (first.known.window as any).__codexPlusBuiltinExtensionHandles });
  assert.equal(first.domListeners.length, 0, "旧内核上的粘贴监听被摘掉");
  second.api.sync();
  assert.equal(second.domListeners.length, 1);
});

test("粘贴修复读到设置前按启动时注入的开关，读到之后按设置项，改设置不必重启", () => {
  const inserted: string[] = [];
  const context: Record<string, any> = {
    window: { __CODEX_PLUS_PASTE_FIX__: { enabled: true } },
    document: { execCommand: (_command: string, _ui: boolean, text: string) => { inserted.push(text); return true; } },
    console: { log() {}, warn() {} },
    codexPlusBackendSettingsLoaded: false,
    settings: { pasteFix: false },
  };
  context.codexPlusSettings = () => context.settings;
  const api = new Function("scope", `with (scope) { ${paste}\n return { handler: codexPlusPasteFixHandler }; }`)(context);
  const paste1 = (text: string) => {
    const event = { prevented: false, clipboardData: { getData: () => text }, preventDefault() { this.prevented = true; }, stopImmediatePropagation() {} };
    api.handler(event);
    return event.prevented;
  };
  assert.equal(paste1("word text"), true, "设置还没读到：按启动时注入的开关");
  context.codexPlusBackendSettingsLoaded = true;
  assert.equal(paste1("word text"), false, "读到设置后按设置项（关）");
  context.settings = { pasteFix: true };
  assert.equal(paste1("word text"), true);
  assert.equal(paste1("x".repeat(20001)), false, "长文本交回 Codex 转附件（#1931）");
  assert.deepEqual(inserted, ["word text", "word text"]);
});
