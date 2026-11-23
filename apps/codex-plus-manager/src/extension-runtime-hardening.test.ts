import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

// 拓展运行时的几处加固：没有归属脚本的失败进诊断日志、改写后的帧发不出去时改发原帧、
// 中间件连续超时熔断、apply 失败后依赖没变不重试。
// 这里把注册中心、拓展内核、RPC 拦截三个分片的真实源码拼在一起执行，拓展经真实的 ctx 接口挂中间件。
const read = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const registrySource = read("01-registry.js");
const kernelSource = read("91-extension-runtime.js");
const rpcSource = read("91-extension-rpc.js");

type Frame = any[];
const middlewareTimeoutMs = 5000;
const breakerCooldownMs = 60_000;

/** 让已经排上的 promise 回调跑完（setImmediate 不在假定时器的接管范围里）。 */
const flush = async () => {
  for (let round = 0; round < 3; round += 1) await new Promise((resolve) => setImmediate(resolve));
};

/**
 * 默认把三个分片拼在一起执行。带原生环节（上下文刷新、供应商预检）的用例要给一个 app-server 客户端，
 * 而内核分片自带的客户端查找依赖真实的 Codex 页面，所以那几例（native）不带内核分片，
 * 中间件链由 chain 充当，其余仍是注册中心与 RPC 拦截的真实源码。
 */
function harness(options: { native?: { context?: boolean; providerSync?: boolean; chain: (value: any) => any } } = {}) {
  const sent: Frame[] = [];
  const attempts: Frame[] = [];
  const diagnostics: Array<{ path: string; payload: any }> = [];
  const state = { refuse: (_frame: Frame) => false };
  class MessagePort {
    postMessage(frame: Frame) {
      attempts.push(frame);
      if (state.refuse(frame)) throw new Error("port refused the frame");
      // 真实的 postMessage 会结构化克隆：帧里带函数时抛 DataCloneError，什么也不发。
      structuredClone(frame);
      sent.push(frame);
    }
  }
  const scriptKey = "user:demo";
  const window: any = {
    MessagePort,
    __codexPlusUserScripts: { currentKey: scriptKey, scripts: { [scriptKey]: { manifest: null } }, registerCleanup: () => {} },
    __codexSessionDeleteBridge: (path: string, payload: any) => {
      diagnostics.push({ path, payload });
      return Promise.resolve({ status: "ok" });
    },
  };
  const native = options.native;
  const scope: Record<string, unknown> = native ? {
    window,
    showToast: () => {},
    codexPlusExtensionListeners: new Map(),
    codexPlusExtensionInterceptors: new Map([["codex/request", [{}]]]),
    codexPlusRunWaterfall: (_name: string, value: any) => native.chain(value),
    codexPerModelContextEnabled: () => !!native.context,
    codexPlusAppServerClient: async () => ({ sendRequest: async () => ({}) }),
    refreshCodexThreadModelBeforeTurn: async () => {},
    prepareCodexThreadProviderResume: native.providerSync ? async () => null : undefined,
    codexPlusBackendSettings: { providerSyncEnabled: !!native.providerSync },
  } : {
    window,
    document: { documentElement: { lang: "zh-CN" } },
    localStorage: { getItem: () => null, setItem() {}, removeItem() {}, key: () => null, length: 0 },
    showToast: () => {},
    codexPlusCurrentExtensionScriptKey: (itemOptions?: { scriptKey?: string }) =>
      itemOptions?.scriptKey || window.__codexPlusUserScripts.currentKey || "",
    codexPlusRegisterExtensionItem: () => () => {},
    codexPlusRegisterExtensionPage: () => () => {},
    codexPlusRegisterExtensionSidePanel: () => () => {},
    codexPlusRegisterExtensionSlot: () => () => {},
    codexPlusExtensionCall: () => Promise.resolve({ status: "ok" }),
  };
  const factory = new Function(...Object.keys(scope), native
    ? `${registrySource}\n${rpcSource}\nreturn {};`
    : `${registrySource}\n${kernelSource}\n${rpcSource}
return { create: codexPlusCreateExtension, services: codexPlusServices,
  requestMiddleware: codexPlusApplyExtensionRequestMiddleware };`);
  const install = () => factory(...Object.values(scope)) as {
    create: (...args: any[]) => any; services: Map<string, unknown>;
    requestMiddleware: (method: string, params: unknown) => any;
  };
  const api = install();
  const port = new MessagePort();
  /** 进了诊断日志的内核失败（事件 extension.kernel_failure）的 message。 */
  const kernelFailures = () => diagnostics
    .filter((entry) => entry.path === "/diagnostics/log" && entry.payload.event === "extension.kernel_failure")
    .map((entry) => String(entry.payload.detail.message));
  return { ...api, install, window, port, sent, attempts, diagnostics, state, kernelFailures, scriptKey };
}

const request = (method: string, params: Record<string, unknown> = {}): Frame =>
  ["push", ["pipeline", -19, ["sendRequest"], [method, params]]];
const subscribeLocal = (): Frame => ["push", ["pipeline", -19, ["subscribe"], [{
  type: "notification", key: { hostId: "local" }, methods: [["turn/completed"]], listener: ["export", -3],
}]]];
const turnFrame = (): Frame => ["resolve", 7, {
  executionHostId: "local", conversationId: "thread",
  resume: { conversationId: "thread", collaborationMode: { settings: { model: "supplier/model" } } },
  start: { request: { threadId: "thread", clientUserMessageId: "message", input: [[{ type: "text", text: "fixture" }]],
    collaborationMode: { settings: { model: "supplier/model" } } } },
}];
const hasFunction = (value: unknown): boolean => typeof value === "function"
  || (!!value && typeof value === "object" && Object.values(value as object).some(hasFunction));

describe("apply 失败后依赖没变不重试", () => {
  it("apply 里先 provide 再抛错只执行一次；依赖重新提供后再试一次", () => {
    const k = harness();
    let applied = 0;
    const fiber = k.create({
      inject: ["x"],
      apply(ctx: any) {
        applied += 1;
        ctx.provide("y", applied);
        throw new Error("apply 坏了");
      },
    });
    assert.equal(fiber.state, "pending", "依赖没到时挂起");
    const provider = k.create((ctx: any) => { ctx.provide("x", { name: "一号" }); });
    assert.equal(applied, 1, "provide 与回滚触发的复查不能再次 apply");
    assert.equal(fiber.state, "failed");
    assert.equal(k.services.has("y"), false, "失败的 apply 提供的服务已回滚");
    assert.match(k.window.__codexPlusUserScripts.scripts[k.scriptKey].error, /apply 坏了/, "失败记在该脚本名下");
    // 与它无关的服务来去都不触发重试。
    const other = k.create((ctx: any) => { ctx.provide("z", 1); });
    other.dispose();
    assert.equal(applied, 1);
    provider.dispose();
    assert.equal(applied, 1, "依赖消失时不重试");
    k.create((ctx: any) => { ctx.provide("x", { name: "二号" }); });
    assert.equal(applied, 2, "依赖回来后只再试一次");
    assert.equal(fiber.state, "failed");
  });

  it("依赖换了之后重试成功就恢复正常，之后照旧随提供方卸载、重新应用", () => {
    const k = harness();
    const seen: string[] = [];
    const fiber = k.create({
      inject: ["x"],
      apply(ctx: any) {
        if (ctx.x.broken) throw new Error("依赖不可用");
        seen.push(`apply:${ctx.x.name}`);
        ctx.effect(() => () => seen.push("dispose"));
      },
    });
    const broken = k.create((ctx: any) => { ctx.provide("x", { broken: true }); });
    assert.equal(fiber.state, "failed");
    broken.dispose();
    const first = k.create((ctx: any) => { ctx.provide("x", { name: "一号" }); });
    assert.equal(fiber.state, "active");
    first.dispose();
    assert.equal(fiber.state, "pending");
    k.create((ctx: any) => { ctx.provide("x", { name: "二号" }); });
    assert.deepEqual(seen, ["apply:一号", "dispose", "apply:二号"]);
  });
});

describe("中间件连续超时熔断", () => {
  it("到阈值后队列里剩下的帧立即放行，冷却期内同步放行，冷却结束恢复拦截", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const env = harness();
    let hang = true;
    let calls = 0;
    env.create((ctx: any) => ctx.intercept("codex/request", (value: any, next: any) => {
      calls += 1;
      return hang ? new Promise(() => {}) : next({ ...value, params: { ...value.params, touched: true } });
    }));
    const frames = [1, 2, 3, 4, 5].map((index) => request("hooks/list", { index }));
    for (const frame of frames) env.port.postMessage(frame);
    await flush();
    assert.equal(env.sent.length, 0, "头一帧在等中间件，后面的排队");
    for (const expected of [1, 2]) {
      t.mock.timers.tick(middlewareTimeoutMs);
      await flush();
      assert.equal(env.sent.length, expected, "熔断前每帧各等满一次超时");
    }
    t.mock.timers.tick(middlewareTimeoutMs);
    await flush();
    assert.equal(calls, 3, "第三次超时后不再进中间件");
    assert.equal(env.sent.length, 5, "队列里剩下的帧不用再各等一次超时");
    frames.forEach((frame, index) => assert.equal(env.sent[index], frame, "原帧按原顺序发出"));
    // 冷却期内：只为拓展拦的帧同步放行。
    const during = request("hooks/list", { index: 6 });
    env.port.postMessage(during);
    assert.equal(env.sent.at(-1), during);
    assert.equal(calls, 3);
    assert.equal(env.kernelFailures().filter((message) => message.includes("持续超时")).length, 1, "熔断只记一条");
    assert.equal(env.kernelFailures().length, 2, "熔断前的单次超时合并成一条，放行的帧不再逐个记");
    // 冷却结束：重新走中间件。
    t.mock.timers.tick(breakerCooldownMs);
    hang = false;
    env.port.postMessage(request("hooks/list", { index: 7 }));
    await flush();
    assert.equal(calls, 4);
    assert.equal(env.sent.at(-1)![1][3][1].touched, true, "冷却后改写恢复生效");
  });

  it("没超时的计划把连续计数清零；冷却后头一个又超时就立刻重新熔断", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const env = harness();
    const script: string[] = [];
    let calls = 0;
    env.create((ctx: any) => ctx.intercept("codex/request", (value: any, next: any) => {
      calls += 1;
      const mode = script.shift();
      if (mode === "hang") return new Promise(() => {});
      if (mode === "throw") throw new Error("中间件抛错");
      return next(value);
    }));
    const run = async (mode: string) => {
      script.push(mode);
      env.port.postMessage(request("hooks/list", { mode }));
      await flush();
      if (mode === "hang") {
        t.mock.timers.tick(middlewareTimeoutMs);
        await flush();
      }
    };
    /** 熔断中的帧同步发出；没熔断时要先过中间件（异步）。 */
    const open = async () => {
      const probe = request("hooks/list", { probe: true });
      env.port.postMessage(probe);
      const passedThrough = env.sent.at(-1) === probe;
      await flush();
      return passedThrough;
    };
    for (const mode of ["hang", "hang", "pass", "hang", "hang", "throw", "hang", "hang"]) await run(mode);
    assert.equal(calls, 8);
    assert.equal(await open(), false, "中间穿插了正常完成和出错放行，始终没有连续三次");
    for (const mode of ["hang", "hang", "hang"]) await run(mode);
    assert.equal(await open(), true, "连续三次超时后熔断");
    t.mock.timers.tick(breakerCooldownMs);
    await run("hang");
    assert.equal(await open(), true, "冷却后的试探又超时：不必再等满三次");
    assert.equal(env.sent.length, env.attempts.length, "每一帧都发出去了，没有丢帧");
  });

  it("同页重新注入后熔断状态从头开始", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
    const env = harness();
    const patched = env.window.MessagePort.prototype.postMessage;
    let calls = 0;
    const hangForever = (ctx: any) => ctx.intercept("codex/request", () => { calls += 1; return new Promise(() => {}); });
    env.create(hangForever);
    for (const index of [1, 2, 3]) env.port.postMessage(request("hooks/list", { index }));
    for (let round = 0; round < 3; round += 1) {
      await flush();
      t.mock.timers.tick(middlewareTimeoutMs);
    }
    await flush();
    assert.equal(env.sent.length, 3);
    const during = request("hooks/list", { index: 4 });
    env.port.postMessage(during);
    assert.equal(env.sent.at(-1), during, "已熔断");
    // 重新注入：补丁函数还是上一代的，判断帧的换成新一代，熔断状态是新的。
    const next = env.install();
    assert.equal(env.window.MessagePort.prototype.postMessage, patched);
    next.create(hangForever);
    const after = request("hooks/list", { index: 5 });
    env.port.postMessage(after);
    await flush();
    assert.equal(calls, 4, "重新注入后又开始走中间件");
    assert.notEqual(env.sent.at(-1), after);
    t.mock.timers.tick(middlewareTimeoutMs);
    await flush();
    assert.equal(env.sent.at(-1), after);
  });
});

describe("改写后的帧发不出去时改发原帧", () => {
  const uncloneable = (ctx: any) => ctx.intercept("codex/request", (value: any, next: any) => (
    ["bad/clone", "turn/start"].includes(value.method)
      ? next({ ...value, params: { ...value.params, callback() {} } })
      : next({ ...value, params: { ...value.params, touched: true } })));

  const withCallback = (value: any) => ({ ...value, params: { ...value.params, callback() {} } });

  it("push：原帧只发一次，后面的帧顺序不变", async () => {
    const env = harness();
    env.create(uncloneable);
    const bad = request("bad/clone", { a: 1 });
    const later = request("hooks/list", { b: 2 });
    const plain: Frame = ["push", ["pipeline", -19, ["markRead"], ["thread"]]];
    env.port.postMessage(bad);
    env.port.postMessage(later);
    env.port.postMessage(plain);
    await flush();
    assert.equal(env.sent.length, 3);
    assert.equal(env.sent[0], bad, "改写后的帧克隆失败，原帧原样发出");
    assert.equal(env.sent[1][1][3][1].touched, true, "后面的帧照常改写");
    assert.equal(env.sent[2], plain);
    assert.equal(env.attempts.filter((frame) => frame[1]?.[3]?.[0] === "bad/clone").length, 2, "一次改写帧（失败）加一次原帧");
    assert.ok(env.kernelFailures().some((message) => message.includes("已改发原帧")));
  });

  it("没改写的帧发送失败时不重复发送", async () => {
    const env = harness();
    env.create((ctx: any) => ctx.intercept("codex/request", (value: any, next: any) => next(value)));
    const frame = request("hooks/list", {});
    env.state.refuse = (candidate) => candidate === frame;
    env.port.postMessage(frame);
    await flush();
    assert.equal(env.attempts.length, 1);
    assert.equal(env.sent.length, 0);
    assert.ok(env.kernelFailures().some((message) => message.includes("rpc 帧发送失败")));
  });

  it("resolve（走排队循环）：改写后的轮次参数克隆失败，原帧发出一次", async () => {
    const env = harness();
    env.create(uncloneable);
    const turn = turnFrame();
    env.port.postMessage(turn);
    await flush();
    assert.deepEqual(env.sent, [turn]);
    assert.equal(env.sent[0], turn);
    assert.equal(env.attempts.length, 2);
  });

  it("resolve（带上下文刷新、单独等待的那条路径）：同样改发原帧", async () => {
    const env = harness({ native: { context: true, chain: withCallback } });
    const turn = turnFrame();
    env.port.postMessage(turn);
    await flush();
    assert.equal(env.sent.length, 1);
    assert.equal(env.sent[0], turn);
    assert.equal(env.attempts.length, 2);
  });

  it("拓展拒绝本轮时，拒绝帧发不出去也不改发原帧", async () => {
    const env = harness();
    env.create((ctx: any) => ctx.on("agent/pre-step", () => ({ kind: "reject", reason: "被拓展拦下" })));
    env.state.refuse = (frame) => frame[0] === "reject";
    env.port.postMessage(turnFrame());
    await flush();
    assert.equal(env.attempts.length, 1);
    assert.equal(env.attempts[0][0], "reject");
    assert.equal(env.sent.length, 0, "本该拦下的消息不能因为发送失败反而发出去");
  });

  it("供应商预检的闸门：留在闸门后面的参数克隆不了时，按原帧重发并用原参数放行", async () => {
    const env = harness({ native: { providerSync: true, chain: withCallback } });
    env.port.postMessage(subscribeLocal());
    const params = { threadId: "thread" };
    env.port.postMessage(request("thread/resume", params));
    await flush();
    assert.equal(env.sent.some(hasFunction), false);
    const pushes = env.sent.filter((frame) => frame[0] === "push" && frame[1][3][0] === "thread/resume");
    assert.equal(pushes.length, 1, "这次请求只占一个序号");
    const gate = pushes[0][1][3][1];
    assert.equal(gate[0], "promise");
    assert.deepEqual(env.sent.at(-1), ["resolve", gate[1], params], "预检完用原参数放行，请求不会一直挂着");
    assert.ok(env.kernelFailures().some((message) => message.includes("已改发原帧")));
  });
});

describe("没有归属脚本的失败进诊断日志", () => {
  function registry(extra: { sendDiagnostic?: (...args: any[]) => any; bridge?: (...args: any[]) => any } = {}) {
    const calls: Array<{ path: string; payload: any }> = [];
    const window: any = {
      __codexPlusUserScripts: { scripts: { "user:demo": {} } },
      __codexSessionDeleteBridge: extra.bridge ?? ((path: string, payload: any) => {
        calls.push({ path, payload });
        return Promise.resolve({ status: "ok" });
      }),
    };
    const mark = new Function("window", "document", "sendCodexPlusDiagnostic",
      `${registrySource}\nreturn codexPlusMarkExtensionFailure;`)(window, {}, extra.sendDiagnostic) as (key: string, message: string) => void;
    const logged = () => calls.filter((call) => call.payload.event === "extension.kernel_failure").map((call) => call.payload.detail);
    return { window, mark, calls, logged };
  }

  it("空 key 写进 /diagnostics/log；同一条 message 一个窗口只记一次，窗口有总量上限", (t) => {
    t.mock.timers.enable({ apis: ["Date"] });
    const r = registry();
    for (let index = 0; index < 5; index += 1) r.mark("", "rpc 帧发送失败：boom");
    assert.deepEqual(r.calls, [{ path: "/diagnostics/log",
      payload: { event: "extension.kernel_failure", detail: { message: "rpc 帧发送失败：boom", suppressed: 0 } } }]);
    for (let index = 0; index < 30; index += 1) r.mark("", `各不相同的失败 ${index}`);
    assert.equal(r.logged().length, 20, "一个窗口最多 20 条");
    assert.equal(r.logged()[1].suppressed, 4, "被限流掉的条数记在下一条日志里");
    assert.equal(r.window.__codexPlusExtensionFailures, undefined, "空 key 不进按脚本归属的失败列表");
    // 窗口过去后同一条 message 可以再记。
    t.mock.timers.tick(60_000);
    r.mark("", "rpc 帧发送失败：boom");
    assert.equal(r.logged().length, 21);
    assert.deepEqual(r.logged().at(-1), { message: "rpc 帧发送失败：boom", suppressed: 11 });
  });

  it("有 key 时行为不变：记在脚本名下，不写内核失败日志", () => {
    const r = registry();
    r.mark("user:demo", "onActivate: 坏了");
    assert.equal(r.logged().length, 0);
    assert.equal(r.window.__codexPlusUserScripts.scripts["user:demo"].error, "onActivate: 坏了");
    assert.equal(r.window.__codexPlusUserScripts.scripts["user:demo"].extensionError, "onActivate: 坏了");
    assert.deepEqual(r.window.__codexPlusExtensionFailures.map((item: any) => [item.script_key, item.message]),
      [["user:demo", "onActivate: 坏了"]]);
  });

  it("诊断通道自己出问题时不抛错", async () => {
    const throwing = registry({ bridge: () => { throw new Error("bridge down"); } });
    assert.doesNotThrow(() => throwing.mark("", "一"));
    const rejecting = registry({ bridge: () => Promise.reject(new Error("bridge rejected")) });
    assert.doesNotThrow(() => rejecting.mark("", "二"));
    const missing = registry();
    delete missing.window.__codexSessionDeleteBridge;
    assert.doesNotThrow(() => missing.mark("", "三"));
    const failing = registry({ sendDiagnostic: () => Promise.reject(new Error("diagnostic rejected")) });
    assert.doesNotThrow(() => failing.mark("", "四"));
    // 被拒绝的 promise 都已接住：留一轮事件循环，未处理的拒绝会让本用例失败。
    await flush();
  });

  it("完整产物里走统一的诊断上报函数", () => {
    const sent: any[] = [];
    const r = registry({ sendDiagnostic: (event: string, detail: any) => { sent.push({ event, detail }); } });
    r.mark("", "rpc 拦截失败");
    r.mark("", "rpc 拦截失败");
    assert.deepEqual(sent, [{ event: "extension.kernel_failure", detail: { message: "rpc 拦截失败", suppressed: 0 } }]);
    assert.equal(r.calls.length, 0);
  });

  it("旧分发路径上 codex/request 中间件返回 Promise：原样放行并留下记录", () => {
    const env = harness();
    env.create((ctx: any) => ctx.intercept("codex/request", async (value: any) => value));
    const params = { a: 1 };
    assert.equal(env.requestMiddleware("thread/list", params), params);
    assert.deepEqual(env.kernelFailures(), ["codex/request 中间件必须同步返回，本次请求已原样发出"]);
  });
});
