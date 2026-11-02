import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// 内置拓展挂在拓展内核上：扫描步骤由拓展登记、卸下即撤。
// 用量状态栏、请求状态条的数据服务改由官方拓展包提供，见 status-bar-packages.test.ts。
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
        id: definition.id,
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
        // 公开的 ctx.onScan：登记到扫描步骤登记处（owner 为拓展 id），卸载时撤销。
        onScan(handler: () => void, options: { phase?: string } = {}) {
          return track((known.registerScanStep as any)(definition.id, handler, options.phase));
        },
      };
      definition.apply(ctx);
      const handle = { id: definition.id, dispose: () => disposers.reverse().forEach((dispose) => dispose()) };
      handles.push(handle);
      return handle;
    },
    refreshDreamSkin: () => calls.push("dream:refresh"),
    syncCodexPlusCapacityAutoRetry: () => calls.push("capacity:sync"),
    syncWeixinDesktopThreads: async () => { calls.push("weixin:sync"); },
    codexPlusSettings: () => ({ conversationView: false }),
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
  const api = new Function("scope", `with (scope) { ${source}\n return { sync: syncCodexPlusBuiltinExtensions, runSteps: runCodexPlusScanSteps, steps: codexPlusScanSteps, register: registerCodexPlusScanStep }; }`)(scope);
  known.registerScanStep = api.register;
  /** 登记了扫描步骤的拓展 → 它们的 phase。 */
  const stepOwners = () => {
    const owners = new Map<string, string[]>();
    for (const entry of api.steps().values()) owners.set(entry.owner, [...(owners.get(entry.owner) || []), entry.phase]);
    return owners;
  };
  const emit = (event: string, ...args: unknown[]) => [...(listeners.get(event) || [])].forEach((handler) => handler(...args));
  const chain = (name: string, value: any) => {
    const items = interceptors.get(name) || [];
    const step = (index: number, current: any): any => (index >= items.length ? current : items[index](current, (next = current) => step(index + 1, next)));
    return step(0, value);
  };
  return { api, calls, services, known, listeners, interceptors, domListeners, emit, chain, stepOwners };
}

test("内置拓展装上时登记扫描步骤；停用后撤掉", () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const { api, calls, known, stepOwners } = harness();
  api.sync();
  for (const id of ["dream-skin", "side-chat-save", "dictation", "plugin-market-unlock", "session-delete", "thread-id-badge", "conversation-view"]) {
    assert.ok(stepOwners().has(`builtin:${id}`), `${id} 经 ctx.onScan 登记扫描步骤`);
  }
  assert.deepEqual(stepOwners().get("builtin:turn-rewind"), ["light"], "轮次回退跟着轻量扫描");
  assert.deepEqual(stepOwners().get("builtin:thread-scroll-restore")?.sort(), ["deferred", "light"], "滚动钩子跟轻量扫描、位置同步跟完整扫描");
  calls.length = 0;
  api.runSteps("light");
  assert.ok(!calls.includes("dream:refresh"), "轻量扫描只跑 light 步骤");
  calls.length = 0;
  api.runSteps();
  assert.ok(calls.includes("dream:refresh"), "扫描循环经登记的步骤刷新梦皮");
  (known.window as any).__codexPlusBuiltinExtensionState = { disabled: ["dream-skin"], replacedBy: {} };
  calls.length = 0;
  api.sync();
  assert.ok(!stepOwners().has("builtin:dream-skin"), "停用后扫描不再点名它");
  calls.length = 0;
  api.runSteps();
  assert.ok(!calls.includes("dream:refresh"));
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
  assert.deepEqual(domListeners.map((entry) => entry.type), ["keydown", "resize", "paste"], "听写快捷键、对话宽度跟着窗口缩放、粘贴修复的监听都随拓展挂上");
  (known.window as any).__codexPlusBuiltinExtensionState = { disabled: [], replacedBy: { "side-chat-save": "user:my-side", "paste-fix": "user:my-paste" } };
  api.sync();
  assert.equal(interceptors.get("codex/exchange")?.length, 0, "被顶替后中间件撤掉");
  assert.deepEqual(domListeners.map((entry) => entry.type), ["keydown", "resize"], "被顶替后粘贴监听摘掉");
});

test("同页重复注入时先卸下上一份注入包装上的内置拓展", () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const first = harness();
  first.api.sync();
  assert.equal(first.domListeners.length, 3);
  const second = harness({ __codexPlusBuiltinExtensionHandles: (first.known.window as any).__codexPlusBuiltinExtensionHandles });
  assert.equal(first.domListeners.length, 0, "旧内核上的监听被摘掉");
  second.api.sync();
  assert.equal(second.domListeners.length, 3);
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

test("改了设置（settings/changed）时跟着设置走的内置拓展立即对齐；心跳只叫醒对齐便宜的那几项", async () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const { api, calls, known, emit } = harness();
  known.syncStepwisePanel = () => calls.push("stepwise:sync");
  known.syncCodexPlusTurnRewind = () => calls.push("rewind:sync");
  (known.window as any).__codexStepwisePanel = { loadSettings: async () => { calls.push("stepwise:load"); } };
  api.sync();
  calls.length = 0;
  emit("settings/loaded", { reason: "heartbeat" });
  assert.ok(calls.includes("capacity:sync") && calls.includes("dream:refresh"), "心跳叫醒对齐便宜的功能");
  assert.ok(!calls.includes("stepwise:sync") && !calls.includes("rewind:sync"), "心跳不叫醒只跟改动走的功能");
  calls.length = 0;
  emit("settings/loaded", { reason: "startup" });
  for (const call of ["stepwise:sync", "rewind:sync", "capacity:sync"]) assert.ok(calls.includes(call), call);
  calls.length = 0;
  emit("settings/changed", { key: "codexAppTurnRewind", value: true, persisted: false });
  for (const call of ["rewind:sync", "capacity:sync"]) assert.ok(calls.includes(call), call);
  assert.ok(!calls.includes("stepwise:load"), "还没写进后端时不让悬浮球重读设置");
  calls.length = 0;
  emit("settings/changed", { key: "codexAppAnswerOutlineEnabled", value: true, persisted: true });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls.filter((call) => call === "stepwise:load").length, 1, "回答大纲的开关只由它自己的内置拓展处理");
});

test("卸下时功能自己的撤销步骤读到的已经是停用", () => {
  (globalThis as any).__codexPlusScanSteps = undefined;
  (globalThis as any).__codexPlusBuiltinInactive = undefined;
  const { api, known } = harness();
  const seen: boolean[] = [];
  known.installCodexPlusDictation = () => seen.push(!!(globalThis as any).__codexPlusBuiltinInactive?.has("dictation"));
  api.sync();
  assert.deepEqual(seen, [false], "装上时按启用对齐");
  (known.window as any).__codexPlusBuiltinExtensionState = { disabled: ["dictation"], replacedBy: {} };
  api.sync();
  assert.deepEqual(seen, [false, true], "卸下时再对齐一次，功能按停用撤下入口");
});
