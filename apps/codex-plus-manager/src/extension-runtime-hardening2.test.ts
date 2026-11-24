import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

// 拓展运行时的第二批加固：中间件调了 next 没 return 时沿用 next 的结果、内置服务的订阅随实例卸载撤销、
// 多实例下按名字起默认 id 的注册项带实例后缀、codexPlus.call 的合并缓存（独立结果、各自超时、过期淘汰）、
// 同页重复注入后把用户脚本搬到新内核上。
// 这里把注册中心、拓展内核、对外接口层、三个视图分片的真实源码拼在一起执行（92-extension-host.js 只取
// registerCommand 那一个函数，整片依赖真实的 Codex 页面），拓展经真实的 ctx 接口操作。
const read = (path: string) => readFileSync(new URL(`../../../assets/inject/${path}`, import.meta.url), "utf8");
const shard = (name: string) => read(`renderer-inject/${name}`);
const registrySource = shard("01-registry.js");
const kernelSource = shard("91-extension-runtime.js");
const apiSource = shard("91-extension-api.js");
const extrasSource = shard("92-extension-extras.js");
const viewsSource = shard("92-extension-views.js");
const startupSource = shard("99-startup.js");
const userScriptsRuntimeSource = read("user-scripts-runtime.js");
const commandSource = (() => {
  const host = shard("92-extension-host.js");
  const start = host.indexOf("  function codexPlusRegisterExtensionCommand(");
  const end = host.indexOf("\n  /** 当前打开的 Codex 会话 id", start);
  assert.ok(start >= 0 && end > start, "找不到 registerCommand 的实现");
  return host.slice(start, end);
})();

const defaultCallTimeoutMs = 26000;
const scriptKey = "user:demo";

/** 让已经排上的 promise 回调跑完（setImmediate 不在假定时器的接管范围里）。 */
const flush = async () => {
  for (let round = 0; round < 3; round += 1) await new Promise((resolve) => setImmediate(resolve));
};

type BridgeCall = { path: string; payload: any };

/**
 * generation：这份注入是同一个文档里的第几次（00-prelude 写进 window.__codexPlusRuntimeGeneration 的值）。
 * install() 把分片再执行一遍，等同于同页重新注入：拿到一套新闭包，window 还是同一个。
 */
function harness(options: { manifest?: unknown; bridge?: (path: string, payload: any) => unknown } = {}) {
  const bridgeCalls: BridgeCall[] = [];
  const infos: string[] = [];
  const state = { bridge: options.bridge ?? (() => ({ status: "ok" })) };
  const store = new Map<string, string>();
  const window: any = {
    __codexPlusRuntimeGeneration: 1,
    __codexPlusUserScripts: { currentKey: null, scripts: {} },
    __codexSessionDeleteBridge: (path: string, payload: any) => {
      bridgeCalls.push({ path, payload });
      return Promise.resolve(state.bridge(path, payload));
    },
    setTimeout: (...args: [any, any]) => setTimeout(...args),
    clearTimeout: (id: any) => clearTimeout(id),
    setInterval: (...args: [any, any]) => setInterval(...args),
    clearInterval: (id: any) => clearInterval(id),
  };
  const styles = new Map<string, any>();
  const scope: Record<string, unknown> = {
    window,
    document: {
      hidden: false,
      documentElement: { lang: "zh-CN", setAttribute() {}, removeAttribute() {} },
      head: { append: (node: any) => styles.set(node.id, node) },
      getElementById: (id: string) => styles.get(id) ?? null,
      createElement: () => {
        const node: any = { id: "", textContent: "", remove: () => styles.delete(node.id) };
        return node;
      },
      querySelector: () => null,
      querySelectorAll: () => [],
      addEventListener() {},
      removeEventListener() {},
    },
    localStorage: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, String(value)); },
      removeItem: (key: string) => { store.delete(key); },
      key: () => null,
      length: 0,
    },
    navigator: { platform: "MacIntel", userAgent: "" },
    console: { info: (message: string) => infos.push(String(message)), warn() {}, error() {}, log() {} },
    showToast: () => {},
    // 91-extension-api.js 顶层常量引用的类名、选择器（别的分片定义）。
    codexPlusPageClass: "codex-plus-page",
    codexPlusRailSelector: "[data-rail]",
    codexPlusRailDestinationSelector: "[data-rail-destination]",
    actionGroupClass: "codex-action-group",
    moreMenuClass: "codex-more-menu",
    // 各界面宿主的重画入口（别的分片定义）：这里没有界面可画。
    refreshCodexPlusPageNav: () => {},
    refreshCodexPlusRailNavigation: () => {},
    refreshCodexPlusComposerActions: () => {},
    refreshCodexPlusMessageSlots: () => {},
    refreshCodexPlusSidePanels: () => {},
    refreshCodexPlusUiSlots: () => {},
    installCodexPlusCommandHandlers: () => {},
    markCodexPlusExtensionNode: () => {},
    codexPlusCurrentSessionId: () => "",
    codexPlusRegisterExtensionSidePanel: () => () => {},
    codexPlusRegisterExtensionSlot: () => () => {},
  };
  const factory = new Function(...Object.keys(scope), `${registrySource}
${kernelSource}
${apiSource}
${commandSource}
${extrasSource}
${viewsSource}
return { create: codexPlusCreateExtension, services: codexPlusServices, registry: codexPlusRegistry,
  listeners: codexPlusExtensionListeners, emit: codexPlusEmitExtensionEvent, waterfall: codexPlusRunWaterfall,
  installBuiltins: installCodexPlusBuiltinServices, call: codexPlusExtensionCall, callCache: codexPlusExtensionCallCache,
  rehome: () => codexPlusRehomeUserScriptsAfterReinjection(0) };`);
  const install = () => factory(...Object.values(scope)) as {
    create: (...args: any[]) => any; services: Map<string, { value: any; owner: string }>;
    registry: Record<string, Map<string, any>>; listeners: Map<string, Set<unknown>>;
    emit: (event: string, args?: unknown[]) => void; waterfall: (name: string, value: unknown) => any;
    installBuiltins: () => void; call: (route: string, payload?: unknown, options?: { timeout?: number }) => Promise<any>;
    callCache: Map<string, unknown>; rehome: () => boolean;
  };
  /** 模拟 wrap_script：脚本初始化期间 currentKey 是它自己，状态记录里带清单。 */
  const runScript = (kernel: { create: (...args: any[]) => any }, definition: unknown, key = scriptKey) => {
    const runtime = window.__codexPlusUserScripts;
    runtime.scripts[key] = { key, manifest: options.manifest ?? null, status: "loading", error: "" };
    runtime.currentKey = key;
    try {
      const handle = kernel.create(definition);
      runtime.scripts[key].status = "loaded";
      return handle;
    } catch (error) {
      runtime.scripts[key].status = "failed";
      runtime.scripts[key].error = String((error as Error)?.message || error);
      return null;
    } finally {
      runtime.currentKey = null;
    }
  };
  const record = (key = scriptKey) => window.__codexPlusUserScripts.scripts[key];
  const calls = (path: string) => bridgeCalls.filter((call) => call.path === path);
  /** 进了诊断日志的内核失败（事件 extension.kernel_failure）的 message。 */
  const kernelFailures = () => calls("/diagnostics/log")
    .filter((call) => call.payload.event === "extension.kernel_failure")
    .map((call) => String(call.payload.detail.message));
  const api = install();
  return { ...api, install, window, state, bridgeCalls, calls, kernelFailures, infos, runScript, record };
}

describe("中间件调了 next() 没 return 时沿用 next 的结果", () => {
  const upstream = (ctx: any) => ctx.intercept("demo", (value: any, next: any) => next({ ...value, upstream: true }), { order: 1 });
  const downstream = (ctx: any) => ctx.intercept("demo", (value: any, next: any) => next({ ...value, downstream: true }), { order: 3 });

  it("同步中间件：上游的改写和下游的结果都留着", () => {
    const k = harness();
    k.runScript(k, (ctx: any) => {
      upstream(ctx);
      ctx.intercept("demo", (value: any, next: any) => { next({ ...value, middle: true }); }, { order: 2 });
      downstream(ctx);
    });
    assert.deepEqual(k.waterfall("demo", { a: 1 }), { a: 1, upstream: true, middle: true, downstream: true });
  });

  it("异步中间件（await 之后才调 next）：结果是 Promise，解析出下游的结果", async () => {
    const k = harness();
    k.runScript(k, (ctx: any) => {
      upstream(ctx);
      ctx.intercept("demo", async (value: any, next: any) => {
        await Promise.resolve();
        next({ ...value, middle: true });
      }, { order: 2 });
      downstream(ctx);
    });
    const result = k.waterfall("demo", { a: 1 });
    assert.equal(typeof result?.then, "function");
    assert.deepEqual(await result, { a: 1, upstream: true, middle: true, downstream: true });
  });

  it("同步中间件没 return，下游是异步的：拿到下游的 Promise", async () => {
    const k = harness();
    k.runScript(k, (ctx: any) => {
      ctx.intercept("demo", (value: any, next: any) => { next(value); }, { order: 1 });
      ctx.intercept("demo", async (value: any, next: any) => next({ ...value, late: true }), { order: 2 });
    });
    assert.deepEqual(await k.waterfall("demo", { a: 1 }), { a: 1, late: true });
  });

  it("没调 next 的仍是短路：返回什么就是什么，下游不执行", async () => {
    const k = harness();
    let reached = 0;
    k.runScript(k, (ctx: any) => {
      ctx.intercept("sync", () => {}, { order: 1 });
      ctx.intercept("sync", (value: any, next: any) => { reached += 1; return next(value); }, { order: 2 });
      ctx.intercept("async", async () => {}, { order: 1 });
      ctx.intercept("async", (value: any, next: any) => { reached += 1; return next(value); }, { order: 2 });
      ctx.intercept("stop", () => "到此为止", { order: 1 });
      ctx.intercept("stop", (value: any, next: any) => { reached += 1; return next(value); }, { order: 2 });
    });
    assert.equal(k.waterfall("sync", { a: 1 }), undefined);
    assert.equal(await k.waterfall("async", { a: 1 }), undefined);
    assert.equal(k.waterfall("stop", { a: 1 }), "到此为止");
    assert.equal(reached, 0);
  });

  it("显式返回了值的不变：调过 next 也以自己的返回值为准", async () => {
    const k = harness();
    k.runScript(k, (ctx: any) => {
      ctx.intercept("sync", (value: any, next: any) => { next(value); return { own: true }; }, { order: 1 });
      ctx.intercept("sync", (value: any, next: any) => next({ ...value, downstream: true }), { order: 2 });
      ctx.intercept("null", (value: any, next: any) => { next(value); return null; });
      ctx.intercept("async", async (value: any, next: any) => { await next(value); return { own: true }; });
    });
    assert.deepEqual(k.waterfall("sync", { a: 1 }), { own: true });
    assert.equal(k.waterfall("null", { a: 1 }), null, "null 是显式返回的值，不当成没 return");
    assert.deepEqual(await k.waterfall("async", { a: 1 }), { own: true });
  });

  it("排在内置中间件后面的第三方没 return，内置的包装不丢（codex/exchange 的用法）", async () => {
    const k = harness();
    const order: string[] = [];
    // 内置功能（侧聊保存）把 send 包一层；第三方排在它后面、只看一眼就 next。
    k.create((ctx: any) => ctx.intercept("codex/exchange", (exchange: any, next: any) => next({
      ...exchange,
      send: async () => { order.push("builtin:before"); const result = await exchange.send(); order.push("builtin:after"); return result; },
    })), { scriptKey: "builtin:side-chat-save" });
    k.runScript(k, (ctx: any) => ctx.intercept("codex/exchange", (exchange: any, next: any) => { next(exchange); }, { order: 10 }));
    const exchange = await k.waterfall("codex/exchange", { method: "turn/start", send: async () => { order.push("send"); return "ok"; } });
    assert.equal(typeof exchange?.send, "function", "链的结果不能是 undefined");
    assert.equal(await exchange.send(), "ok");
    assert.deepEqual(order, ["builtin:before", "send", "builtin:after"]);
  });
});

describe("内置服务的订阅随拓展实例卸载撤销", () => {
  const historyBridge = (events: any[]) => (path: string, payload: any) => {
    if (path !== "/extensions/history") return { status: "ok" };
    if (payload.method === "listSessions") return { status: "ok", result: [{ id: "s-1", updatedAt: 1 }] };
    if (payload.method === "listEvents") return { status: "ok", result: events.slice(-1) };
    if (payload.method === "readEventsSince") return { status: "ok", result: events.filter((event) => event.seq >= payload.params.seq) };
    return { status: "ok", result: null };
  };

  it("observeAll：没包进 ctx.effect 也随卸载撤销，会话轮询停下", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const k = harness({ bridge: historyBridge([]) });
    k.installBuiltins();
    const seen: unknown[] = [];
    const fiber = k.runScript(k, { inject: ["sessionQuery"], apply(ctx: any) { ctx.sessionQuery.observeAll((event: unknown) => seen.push(event)); } });
    assert.equal(k.listeners.get("session/event")?.size, 1);
    k.emit("session/event", [{ seq: 1 }, { sessionId: "s-1" }]);
    assert.deepEqual(seen, [{ seq: 1 }]);
    // 轮询在跑：1.5 秒一轮，会话列表每 4 轮重新拉一次。
    const poll = async (rounds: number) => {
      for (let round = 0; round < rounds; round += 1) {
        t.mock.timers.tick(1500);
        await flush();
      }
      return k.calls("/extensions/history").length;
    };
    t.mock.timers.tick(0);
    await flush();
    const polled = k.calls("/extensions/history").length;
    assert.ok(polled > 0);
    assert.ok(await poll(8) > polled, "有人订阅时轮询持续");
    // 热重载：用户脚本运行时调登记的清理函数。
    fiber.dispose();
    assert.equal(k.listeners.has("session/event"), false, "监听已撤销");
    k.emit("session/event", [{ seq: 2 }, { sessionId: "s-1" }]);
    assert.deepEqual(seen, [{ seq: 1 }], "旧 handler 不再被调");
    const afterDispose = await poll(1);
    assert.equal(await poll(16), afterDispose, "没人订阅后轮询停止，不再走桥");
  });

  it("observeSession：经 ctx.get 拿到的服务同样随卸载停止轮询", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const events = [{ seq: 0 }];
    const k = harness({ bridge: historyBridge(events) });
    k.installBuiltins();
    const seen: number[] = [];
    const ctx = k.runScript(k, (ctx: any) => { ctx.get("sessionQuery").observeSession("s-1", (event: any) => seen.push(event.seq), { intervalMs: 500 }); });
    await flush();
    events.push({ seq: 1 });
    t.mock.timers.tick(500);
    await flush();
    assert.deepEqual(seen, [1], "卸载前照常增量推送");
    ctx.dispose();
    events.push({ seq: 2 });
    const before = k.calls("/extensions/history").length;
    for (let round = 0; round < 5; round += 1) {
      t.mock.timers.tick(2000);
      await flush();
    }
    assert.deepEqual(seen, [1]);
    assert.equal(k.calls("/extensions/history").length, before, "卸载后不再轮询");
  });

  it("locale.register：经 ctx.get 登记的词条随卸载撤销", () => {
    const k = harness();
    k.installBuiltins();
    let t: (key: string) => string = () => "";
    const ctx = k.runScript(k, (ctx: any) => {
      const locale = ctx.get("locale");
      locale.register("demo", "zh", { hello: "你好" });
      locale.register("demo", { en: { bye: "bye" }, zh: { bye: "再见" } });
      t = locale.bind("demo");
      assert.equal(locale.current, "zh-CN", "getter 原样保留");
    });
    assert.equal(t("hello"), "你好");
    assert.equal(t("bye"), "再见");
    ctx.dispose();
    assert.equal(t("hello"), "hello", "词条已撤销，回到 key 本身");
    assert.equal(t("bye"), "bye");
  });

  it("返回的撤销函数可以提前调，重复调、卸载后再调都无害", () => {
    const k = harness();
    k.installBuiltins();
    let stop: () => void = () => {};
    let stopSession: () => void = () => {};
    const fiber = k.runScript(k, {
      inject: ["sessionQuery"],
      apply(ctx: any) {
        stop = ctx.sessionQuery.observeAll(() => {});
        ctx.sessionQuery.observeAll(() => {});
        stopSession = ctx.sessionQuery.observeSession("s-1", () => {});
      },
    });
    assert.equal(k.listeners.get("session/event")?.size, 2);
    stop();
    stop();
    assert.equal(k.listeners.get("session/event")?.size, 1, "提前撤销只撤自己那一个");
    stopSession();
    fiber.dispose();
    assert.equal(k.listeners.has("session/event"), false);
    assert.doesNotThrow(() => { stop(); stopSession(); });
    assert.equal(k.record().error, "", "撤销过程没有报错记到脚本名下");
  });

  it("每个实例各管各的：一个卸载不影响另一个；实例里反复读到的是同一个对象", () => {
    const k = harness();
    k.installBuiltins();
    const seen: string[] = [];
    let firstQuery: any;
    const first = k.runScript(k, { inject: ["sessionQuery"], apply(ctx: any) {
      firstQuery = ctx.sessionQuery;
      assert.equal(ctx.sessionQuery, firstQuery);
      assert.equal(ctx.get("sessionQuery"), firstQuery);
      ctx.sessionQuery.observeAll(() => seen.push("first"));
    } }, "user:first");
    k.runScript(k, { inject: ["sessionQuery"], apply(ctx: any) { ctx.sessionQuery.observeAll(() => seen.push("second")); } }, "user:second");
    assert.ok(Object.isFrozen(firstQuery));
    assert.deepEqual(Object.keys(firstQuery).sort(), Object.keys(k.services.get("sessionQuery")!.value).sort(), "方法一个不少");
    first.dispose();
    k.emit("session/event", [{ seq: 1 }, { sessionId: "s-1" }]);
    assert.deepEqual(seen, ["second"]);
  });

  it("其它拓展 provide 的服务原样交出去，不包装", () => {
    const k = harness();
    k.installBuiltins();
    const provided = { observeAll: () => () => {}, register: () => () => {} };
    k.runScript(k, (ctx: any) => { ctx.provide("thirdParty", provided); }, "user:provider");
    let got: unknown;
    let viaGet: unknown;
    k.runScript(k, { inject: ["thirdParty"], apply(ctx: any) { got = ctx.thirdParty; viaGet = ctx.get("thirdParty"); } });
    assert.equal(got, provided);
    assert.equal(viaGet, provided);
  });
});

describe("多实例（清单 instances）下注册项的 id", () => {
  const instances = { id: "multi", instances: [{ id: "work", name: "工作" }, { id: "home", name: "家" }] };
  const apply = (seen: string[]) => (ctx: any) => {
    ctx.registerCommand({ name: "hello", run() {} });
    ctx.registerTheme({ id: "sepia", css: "body{}" });
    ctx.registerSettingsSection({ id: "usage", title: "用量", render() {} });
    ctx.registerSettingsSection({ title: "别的", render() {} }, { id: "other" });
    ctx.registerSettingsPanel({ render() {} });
    seen.push(ctx.instance?.id ?? "single");
  };
  const ids = (map: Map<string, unknown>) => [...map.keys()];

  it("单实例拓展的 id 和以前完全一样", (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const k = harness({ manifest: { id: "solo" } });
    const seen: string[] = [];
    k.runScript(k, apply(seen));
    assert.deepEqual(seen, ["single"]);
    assert.deepEqual(ids(k.registry.commands), ["user:demo:command:cmd-hello"]);
    assert.deepEqual(ids(k.registry.themes), ["user:demo:theme:theme-sepia"]);
    assert.deepEqual(ids(k.registry.settingsSections), ["user:demo:settingsSection:usage", "user:demo:settingsSection:other"]);
    assert.deepEqual(ids(k.registry.settingsPanels), ["user:demo:settingsPanel"]);
    assert.equal(k.record().status, "loaded");
  });

  it("多实例：每个实例都建起来，默认 id 带实例后缀，规则同 options.id（<id>@<实例 id>）", (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const k = harness({ manifest: instances });
    const seen: string[] = [];
    k.runScript(k, apply(seen));
    assert.equal(k.record().error, "", "没有「id 已被占用」");
    assert.deepEqual(seen, ["work", "home"], "第二个实例照常创建");
    assert.deepEqual(ids(k.registry.commands), ["user:demo:command:cmd-hello@work", "user:demo:command:cmd-hello@home"]);
    assert.deepEqual(ids(k.registry.themes), ["user:demo:theme:theme-sepia@work", "user:demo:theme:theme-sepia@home"]);
    assert.deepEqual(ids(k.registry.settingsSections), [
      "user:demo:settingsSection:usage@work", "user:demo:settingsSection:other@work",
      "user:demo:settingsSection:usage@home", "user:demo:settingsSection:other@home",
    ]);
    assert.equal(k.registry.settingsSections.get("user:demo:settingsSection:usage@home").sectionKey, "usage", "挑图标用的 sectionKey 不变");
    assert.equal(k.registry.themes.get("user:demo:theme:theme-sepia@home").themeId, "sepia", "主题 id 本身不变");
  });

  it("设置面板每个拓展一个：先注册的实例生效，其余忽略并留一句提示；该实例卸载时面板撤销", (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const k = harness({ manifest: instances });
    const renders: string[] = [];
    const contexts: any[] = [];
    k.runScript(k, (ctx: any) => {
      contexts.push(ctx);
      ctx.registerSettingsPanel({ render: () => renders.push(ctx.instance.id) });
    });
    assert.equal(contexts.length, 2);
    assert.deepEqual(ids(k.registry.settingsPanels), ["user:demo:settingsPanel"], "id 与单实例一样，详情页按脚本 key 找得到");
    k.registry.settingsPanels.get("user:demo:settingsPanel").render();
    assert.deepEqual(renders, ["work"]);
    assert.equal(k.infos.length, 1);
    assert.match(k.infos[0], /实例 home 的 registerSettingsPanel 已忽略/);
    assert.equal(k.record().error, "", "提示不记成脚本的错误");
    // 被忽略的实例卸载不动面板；注册了面板的那个实例卸载时面板撤销。
    contexts[1].dispose();
    assert.equal(k.registry.settingsPanels.size, 1);
    contexts[0].dispose();
    assert.equal(k.registry.settingsPanels.size, 0);
  });

  it("同一个实例重复注册设置面板照旧报错（单实例、多实例都是）", (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const twice = (ctx: any) => {
      ctx.registerSettingsPanel({ render() {} });
      ctx.registerSettingsPanel({ render() {} });
    };
    const solo = harness({ manifest: { id: "solo" } });
    solo.runScript(solo, twice);
    assert.match(solo.record().error, /拓展项 id 已被占用/);
    const multi = harness({ manifest: instances });
    multi.runScript(multi, twice);
    assert.match(multi.record().error, /拓展项 id 已被占用/);
  });
});

describe("codexPlus.call 的合并缓存", () => {
  const rows = () => ({ status: "ok", items: [{ id: 1 }, { id: 2 }, { id: 3 }] });
  /** 桥请求由用例手动放行。 */
  function deferredBridge() {
    const pending: Array<(value: unknown) => void> = [];
    return { pending, bridge: () => new Promise((resolve) => pending.push(resolve)) };
  }

  it("窗口内同参只发一次桥请求，过了窗口重新发", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const k = harness({ bridge: rows });
    await Promise.all([1, 2, 3].map(() => k.call("/request-log/recent", { limit: 5 })));
    t.mock.timers.tick(1999);
    await k.call("/request-log/recent", { limit: 5 });
    assert.equal(k.calls("/request-log/recent").length, 1);
    t.mock.timers.tick(1);
    await k.call("/request-log/recent", { limit: 5 });
    assert.equal(k.calls("/request-log/recent").length, 2);
  });

  it("每个调用方拿到独立的结果：一方原地改，别人和后来者读到的还是原样", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const k = harness({ bridge: rows });
    const [first, second] = await Promise.all([k.call("/request-log/recent", {}), k.call("/request-log/recent", {})]);
    assert.notEqual(first, second);
    assert.notEqual(first.items, second.items);
    first.items.reverse();
    first.items[0].id = 99;
    first.extra = true;
    assert.deepEqual(second, rows());
    // 结果已经回来之后、窗口内才来的调用方，拿到的也是没被动过的那份。
    assert.deepEqual(await k.call("/request-log/recent", {}), rows());
    assert.equal(k.calls("/request-log/recent").length, 1);
  });

  it("超时各算各的：先到者的短超时不套给后来的，底层请求仍只发一次", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const { pending, bridge } = deferredBridge();
    const k = harness({ bridge });
    const outcomes: string[] = [];
    const short = k.call("/request-log/recent", {}, { timeout: 100 }).then(() => outcomes.push("short:ok"), (error) => outcomes.push(`short:${error.message}`));
    const normal = k.call("/request-log/recent", {}).then((result) => outcomes.push(`normal:${result.items.length}`), (error) => outcomes.push(`normal:${error.message}`));
    assert.equal(k.calls("/request-log/recent").length, 1);
    t.mock.timers.tick(100);
    await flush();
    assert.deepEqual(outcomes, ["short:调用 /request-log/recent 超时"], "只有短超时的那一个放弃");
    pending[0](rows());
    await Promise.all([short, normal]);
    assert.deepEqual(outcomes, ["short:调用 /request-log/recent 超时", "normal:3"]);
    assert.equal(k.calls("/request-log/recent").length, 1);
  });

  it("后到者的短超时也只管自己：先到者按默认超时继续等", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const { pending, bridge } = deferredBridge();
    const k = harness({ bridge });
    const outcomes: string[] = [];
    const first = k.call("/model-catalog/public", {}).then((result) => outcomes.push(`first:${result.items.length}`), (error) => outcomes.push(`first:${error.message}`));
    const second = k.call("/model-catalog/public", {}, { timeout: 50 }).then(() => outcomes.push("second:ok"), () => outcomes.push("second:超时"));
    t.mock.timers.tick(50);
    await flush();
    assert.deepEqual(outcomes, ["second:超时"]);
    t.mock.timers.tick(defaultCallTimeoutMs - 51);
    await flush();
    assert.deepEqual(outcomes, ["second:超时"], "先到者还没到自己的超时");
    pending[0](rows());
    await Promise.all([first, second]);
    assert.deepEqual(outcomes, ["second:超时", "first:3"]);
  });

  it("有人等到超时后，后来的调用不再接着等那次卡住的请求", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const { pending, bridge } = deferredBridge();
    const k = harness({ bridge });
    const stuck = k.call("/model-catalog/public", {}, { timeout: 100 });
    t.mock.timers.tick(100);
    await assert.rejects(stuck, /超时/);
    const retry = k.call("/model-catalog/public", {});
    assert.equal(k.calls("/model-catalog/public").length, 2, "窗口还没过，但卡住的那次已不再复用");
    pending[1](rows());
    assert.deepEqual(await retry, rows());
  });

  it("失败不留在缓存里；失败的响应（status: failed）统一成 reject", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    let fail = true;
    const k = harness({ bridge: () => (fail ? { status: "failed", message: "后端报错" } : rows()) });
    await assert.rejects(Promise.all([k.call("/proxy-status", {}), k.call("/proxy-status", {})]), /后端报错/);
    await flush();
    assert.equal(k.callCache.size, 0);
    fail = false;
    assert.deepEqual(await k.call("/proxy-status", {}), rows());
    assert.equal(k.calls("/proxy-status").length, 2);
  });

  it("过期条目被淘汰：payload 带变化值时缓存有上限，窗口过后清空", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const k = harness({ bridge: rows });
    for (let index = 0; index < 200; index += 1) {
      await k.call("/thread-usage-live", { cursor: index });
      t.mock.timers.tick(10);
    }
    assert.equal(k.calls("/thread-usage-live").length, 200);
    assert.ok(k.callCache.size <= 64, `缓存条数 ${k.callCache.size} 超过上限`);
    assert.ok(k.callCache.size > 0);
    // 窗口内的那一批都还在复用。
    await k.call("/thread-usage-live", { cursor: 199 });
    assert.equal(k.calls("/thread-usage-live").length, 200);
    // 轮询停了：不再有新调用，条目到点自己清掉。
    t.mock.timers.tick(2000);
    assert.equal(k.callCache.size, 0);
  });

  it("同一窗口里指纹太多时先丢最早的，不影响已经拿到手的调用", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const k = harness({ bridge: rows });
    await Promise.all(Array.from({ length: 100 }, (_, index) => k.call("/model-catalog/public", { cursor: index })));
    assert.equal(k.callCache.size, 64);
    await k.call("/model-catalog/public", { cursor: 99 });
    assert.equal(k.calls("/model-catalog/public").length, 100, "最近的还在缓存里");
    await k.call("/model-catalog/public", { cursor: 0 });
    assert.equal(k.calls("/model-catalog/public").length, 101, "最早的被丢掉，再调就重新发");
  });

  it("没有合并窗口的路由不拷贝、不缓存，行为不变", async () => {
    const result = rows();
    const k = harness({ bridge: () => result });
    assert.equal(await k.call("/user-scripts/list", {}), result);
    await k.call("/user-scripts/list", {});
    assert.equal(k.calls("/user-scripts/list").length, 2);
    assert.equal(k.callCache.size, 0);
    await assert.rejects(k.call("/settings/set", {}), /未开放的路由/);
  });
});

describe("同页重复注入后把用户脚本搬到新内核上", () => {
  // 重载请求是用定时器延后发的（测试里延迟为 0），先让定时器走完再清空微任务。
  const settle = async () => {
    await new Promise((resolve) => setTimeout(resolve, 5));
    await flush();
  };
  /** 在同一个 window 上执行真实的用户脚本运行时（registerCleanup / prepareReload）。 */
  const installUserScriptsRuntime = (window: any) => new Function("window", userScriptsRuntimeSource)(window);
  const interceptor = (ctx: any) => ctx.intercept("codex/request", (value: any, next: any) => next({ ...value, touched: true }));

  it("首次注入不请求重载", async () => {
    const k = harness();
    installUserScriptsRuntime(k.window);
    k.runScript(k, interceptor);
    assert.equal(k.rehome(), false);
    await settle();
    assert.equal(k.calls("/user-scripts/reload").length, 0);
    assert.deepEqual(k.kernelFailures(), []);
  });

  it("重复注入但没有用户脚本：什么都不做", async () => {
    const k = harness();
    k.window.__codexPlusUserScripts.initialized = true;
    k.window.__codexPlusRuntimeGeneration = 2;
    const next = k.install();
    assert.equal(next.rehome(), false);
    await settle();
    assert.equal(k.calls("/user-scripts/reload").length, 0);
    assert.deepEqual(k.kernelFailures(), []);
  });

  it("重复注入且每个脚本都登记了清理函数：只请求一次重载，脚本在新内核上重跑，旧内核上的实例卸掉", async () => {
    const k = harness();
    installUserScriptsRuntime(k.window);
    let live = k as { create: (...args: any[]) => any };
    let reloads = 0;
    // 后端收到重载请求后在页面里执行重载包：先 prepareReload（真实实现），再把脚本重跑一遍。
    k.state.bridge = (path: string) => {
      if (path !== "/user-scripts/reload") return { status: "ok" };
      reloads += 1;
      const mode = k.window.__codexPlusUserScripts.prepareReload();
      if (mode === "scripts") k.runScript(live, interceptor);
      return { status: "ok", reload_mode: mode };
    };
    k.runScript(k, interceptor);
    k.window.__codexPlusUserScripts.initialized = true;
    assert.deepEqual(k.waterfall("codex/request", { method: "m" }), { method: "m", touched: true });

    // 同页重新注入：代际加一，新的一套闭包。
    k.window.__codexPlusRuntimeGeneration = 2;
    const next = k.install();
    live = next;
    assert.deepEqual(next.waterfall("codex/request", { method: "m" }), { method: "m" }, "重载前新内核上没有这个拓展的中间件");
    assert.equal(next.rehome(), true);
    assert.equal(k.calls("/user-scripts/reload").length, 0, "请求是异步发的，不占注入脚本的同步执行");
    await settle();
    assert.deepEqual(k.calls("/user-scripts/reload").map((call) => call.payload), [{}]);
    assert.equal(reloads, 1);
    assert.deepEqual(next.waterfall("codex/request", { method: "m" }), { method: "m", touched: true }, "脚本已在新内核上重跑");
    assert.deepEqual(k.waterfall("codex/request", { method: "m" }), { method: "m" }, "旧内核上的实例已卸载");
    // 不会循环：重载只跑用户脚本包，代际不变，也没有第二次请求。
    assert.equal(k.window.__codexPlusRuntimeGeneration, 2);
    await settle();
    assert.equal(k.calls("/user-scripts/reload").length, 1);
    assert.deepEqual(k.kernelFailures(), []);
  });

  it("有脚本没登记清理函数：不请求（重载会刷新整个页面），记一条诊断", async () => {
    const k = harness();
    installUserScriptsRuntime(k.window);
    k.runScript(k, interceptor);
    // 老式脚本：不经 codexPlus.extension，也没调 registerCleanup。
    k.window.__codexPlusUserScripts.scripts["user:legacy.js"] = { key: "user:legacy.js", status: "loaded" };
    k.window.__codexPlusUserScripts.initialized = true;
    k.window.__codexPlusRuntimeGeneration = 2;
    const next = k.install();
    assert.equal(next.rehome(), false);
    await settle();
    assert.equal(k.calls("/user-scripts/reload").length, 0);
    assert.equal(k.kernelFailures().length, 1);
    assert.match(k.kernelFailures()[0], /user:legacy\.js 没有登记清理函数/);
    // 这正是 prepareReload 会退回整页刷新的情形：判断条件和它一致。
    let pageReloads = 0;
    k.window.__codexPlusUserScriptsBootstrap = true;
    k.window.location = { reload: () => { pageReloads += 1; } };
    k.window.setTimeout = (callback: () => void) => { callback(); return 0; };
    assert.equal(k.window.__codexPlusUserScripts.prepareReload(), "page");
    assert.equal(pageReloads, 1);
  });

  it("登记的是 async 清理函数：prepareReload 等不了它、同样会整页刷新，也不请求", async () => {
    const k = harness();
    installUserScriptsRuntime(k.window);
    k.runScript(k, interceptor);
    k.record().cleanups.push(async () => {});
    k.window.__codexPlusRuntimeGeneration = 2;
    const next = k.install();
    assert.equal(next.rehome(), false);
    await settle();
    assert.equal(k.calls("/user-scripts/reload").length, 0);
    assert.equal(k.kernelFailures().length, 1);
  });

  it("整页刷新已经排上（refreshPending）：不请求", async () => {
    const k = harness();
    installUserScriptsRuntime(k.window);
    k.runScript(k, interceptor);
    k.window.__codexPlusUserScripts.initialized = true;
    k.window.__codexPlusUserScripts.refreshPending = true;
    k.window.__codexPlusRuntimeGeneration = 2;
    const next = k.install();
    assert.equal(next.rehome(), false);
    await settle();
    assert.equal(k.calls("/user-scripts/reload").length, 0);
  });

  it("重载请求失败只记诊断，不抛错", async () => {
    for (const bridge of [
      () => ({ status: "failed", message: "Codex 页面尚未连接" }),
      () => { throw new Error("bridge down"); },
      () => Promise.reject(new Error("bridge rejected")),
    ]) {
      const k = harness();
      installUserScriptsRuntime(k.window);
      k.runScript(k, interceptor);
      k.window.__codexPlusRuntimeGeneration = 2;
      const next = k.install();
      k.state.bridge = (path: string) => (path === "/user-scripts/reload" ? bridge() : { status: "ok" });
      assert.doesNotThrow(() => next.rehome());
      // 被拒绝的 promise 都已接住：留几轮事件循环，未处理的拒绝会让本用例失败。
      await settle();
      assert.equal(k.calls("/user-scripts/reload").length, 1);
      assert.equal(k.kernelFailures().length, 1);
      assert.match(k.kernelFailures()[0], /重复注入后重载用户脚本失败/);
    }
    const missing = harness();
    installUserScriptsRuntime(missing.window);
    missing.runScript(missing, interceptor);
    missing.window.__codexPlusRuntimeGeneration = 2;
    const next = missing.install();
    delete missing.window.__codexSessionDeleteBridge;
    assert.doesNotThrow(() => next.rehome());
    await settle();
  });

  it("99-startup 在挂好 window.codexPlus 之后调用它（源码断言）", () => {
    const mounted = startupSource.indexOf("window.codexPlus = buildCodexPlusExtensionApi();");
    const called = startupSource.indexOf("codexPlusRehomeUserScriptsAfterReinjection();");
    assert.ok(mounted >= 0 && called > mounted);
  });
});
