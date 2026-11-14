import assert from "node:assert/strict";
import { closeSync, existsSync, openSync, readFileSync, readSync } from "node:fs";
import { describe, it } from "node:test";
import { MessageChannel, MessagePort } from "node:worker_threads";

const asarPath = process.env.CODEX_PLUS_CAPNWEB_ASAR || "";
const fragment = readFileSync(new URL("../../../assets/inject/renderer-inject/91-extension-rpc.js", import.meta.url), "utf8");
const bundle = readFileSync(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
const rpcStart = bundle.indexOf(fragment);
assert.notEqual(rpcStart, -1, "the generated bundle must contain the installed RPC fragment");
const rpc = bundle.slice(rpcStart, rpcStart + fragment.length);

function installedCapnweb() {
  const fd = openSync(asarPath, "r");
  let source: string;
  try {
    const header = Buffer.alloc(16);
    readSync(fd, header, 0, header.length, 0);
    const json = Buffer.alloc(header.readUInt32LE(12));
    readSync(fd, json, 0, json.length, 16);
    let entry = JSON.parse(json.toString());
    for (const part of "webview/assets/async/885188.3857056929.js".split("/")) entry = entry.files[part];
    const bytes = Buffer.alloc(entry.size);
    readSync(fd, bytes, 0, bytes.length, 8 + header.readUInt32LE(4) + Number(entry.offset));
    source = bytes.toString();
  } finally {
    closeSync(fd);
  }
  const modules = new Function(`${source.replaceAll("export const ", "const ")}\nreturn __webpack_modules__`)();
  const cap: any = {};
  modules.Fkn8({}, cap, {
    d(target: any, getters: any, values: any) {
      Object.assign(target, values);
      for (const [key, get] of Object.entries(getters)) {
        Object.defineProperty(target, key, { get: get as () => any, enumerable: true });
      }
    },
  });
  return cap;
}

function runtime() {
  const cap = installedCapnweb();
  const { port1, port2 } = new MessageChannel();
  const proto = MessagePort.prototype;
  const original = proto.postMessage;
  const descriptor = Object.getOwnPropertyDescriptor(MessageEvent.prototype, "data")!;
  const frames: { side: string; frame: any }[] = [];
  const calls: any[] = [];
  const failures: string[] = [];
  const settings = { providerSyncEnabled: true };
  const state = { active: false, prepares: 0, started: 0, dispatches: 0, leases: 0,
    discoveries: 0, failAt: 0, waitAt: 0, wait: null as Promise<void> | null,
    contextEnabled: false, contextFailure: "", contextCalls: 0,
    replay: null as (() => Promise<void>) | null, candidates: [] as any[], params: [] as any[],
    candidate: (value: any, _intent: string) => value,
    middleware: async (value: any) => value };
  const listeners = new Map();
  const interceptors = new Map();
  proto.postMessage = function (frame: any, ...rest: any[]) {
    frames.push({ side: this === port1 ? "page" : "host", frame: structuredClone(frame) });
    return original.call(this, frame, ...rest as [any]);
  };
  class Lease extends cap.TF {
    ping() { return "lease"; }
    [Symbol.dispose]() { state.leases++; }
  }
  class Coordinator extends cap.TF {
    async sendMessage(params: any, prepare: any, added: any, current: any, admission: any) {
      state.dispatches++;
      state.params.push(params);
      await current?.();
      await admission?.();
      const candidate = await prepare(params, state.active ? "steer" : "start");
      state.candidates.push(candidate);
      try {
        await added?.("added");
        if (state.active) {
          const retained = prepare.dup();
          state.replay = async () => {
            try {
              const next = await retained(params, "start");
              state.candidates.push(next);
              try { state.started++; } finally { next[Symbol.dispose](); }
            } finally { retained[Symbol.dispose](); }
          };
          return "queued";
        }
        state.started++;
        return params.conversationId;
      } finally { candidate[Symbol.dispose](); }
    }
  }
  const coordinator = new Coordinator();
  class Manager extends cap.TF {
    subscribe() { return undefined; }
    getTurnCoordinator() { return coordinator; }
    async sendRequest(method: string, params: any) {
      calls.push({ method, params });
      if (method === "thread/read") return { thread: { status: { type: state.active ? "active" : "idle" } } };
      if (method === "config/read") return { config: { model_provider: "custom" } };
      return "pong";
    }
    resumeThread(params: any) { return params; }
  }
  cap.ab(port2, new Manager());
  const client = cap.ab(port1);
  const window: any = { MessagePort, MessageEvent };
  const align = async (params: any, hostId: string, sendRequest: any, host: any) => {
    assert.equal(hostId, "local");
    assert.equal(host, client);
    const index = ++state.prepares;
    calls.push({ prepare: index, params });
    await sendRequest("thread/read", { threadId: params.conversationId || params.threadId });
    await sendRequest("config/read", { cwd: null });
    if (index === state.waitAt) await state.wait;
    if (index === state.failAt) throw new TypeError("LOCAL_PROVIDER_PREPARE_FAILED");
    return null;
  };
  const install = (source = rpc) => new Function("window", "codexPlusBackendSettings",
    "prepareCodexThreadProviderResume", "codexPlusAppServerClient", "codexPlusExtensionListeners",
    "codexPlusExtensionInterceptors", "codexPlusMarkExtensionFailure", "showToast",
    "codexPlusRunWaterfall", "runCodexPlusExtensionCallback", "codexPerModelContextEnabled",
    "refreshCodexThreadModelBeforeTurn", source)(
    window, settings, align, async () => { state.discoveries++; return client; }, listeners, interceptors,
    (_key: string, message: string) => failures.push(message), () => {},
    (_name: string, value: any) => state.middleware(value),
    (_key: string, _name: string, callback: () => any) => ({ ok: true, value: callback() }),
    () => state.contextEnabled,
    async (host: any, sendRequest: any, method: string, params: any) => {
      assert.deepEqual(host, { hostId: "local" });
      assert.equal(method, "turn/start");
      state.contextCalls++;
      calls.push({ context: state.contextCalls, params });
      await sendRequest("context-check", { threadId: params.threadId });
      if (state.contextFailure) throw new Error(state.contextFailure);
    });
  const subscribe = (hostId = "local") => client.subscribe({ key: { hostId } });
  const send = (coordinator: any, choice: any = {}, embedded = false) => {
    const callbacks: string[] = [];
    const params = { conversationId: "thread", message: { input: [{ type: "text", text: "next" }],
      submissionOptions: { model: null, reasoningEffort: null, ...choice } }, acceptLocally: true };
    const prepare = async (_params: any, intent: string) => state.candidate({
      conversationId: "thread", executionHostId: "local", requiresIdle: true,
      resume: { conversationId: "thread", model: null, reasoningEffort: null, ...choice },
      steer: { input: [{ type: "text", text: "next" }], restoreMessage: { submissionOptions: choice } },
      ...(intent === "start" ? { start: { request: { threadId: "thread", input: [{ type: "text", text: "next" }],
        model: null, effort: null, ...choice } } } : {}),
      ...(embedded ? { lease: new Lease() } : {}),
    }, intent);
    const promise = coordinator.sendMessage(params, prepare,
      () => { callbacks.push("added"); }, () => { callbacks.push("current"); }, () => { callbacks.push("admission"); });
    return { promise, params, callbacks };
  };
  const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 15));
  const until = async (condition: () => boolean) => {
    for (let i = 0; i < 100 && !condition(); i++) await settle();
    assert.equal(condition(), true, "protocol operation must progress");
  };
  const close = () => {
    proto.postMessage = original;
    Object.defineProperty(MessageEvent.prototype, "data", descriptor);
    port1.close();
    port2.close();
  };
  return { cap, window, port1, client, state, settings, frames, calls, failures, install,
    listeners, interceptors, subscribe, send, settle, until, close };
}

describe("installed Capnweb prepared turn provider gate", { skip: !existsSync(asarPath), timeout: 10000 }, () => {
  it("runs provider preparation and context refresh in order before a turn", async () => {
    const env = runtime();
    try {
      env.state.contextEnabled = true;
      env.install();
      await env.subscribe();
      const result = env.send(await env.client.getTurnCoordinator(), { model: "chosen" }, true);
      assert.equal(await result.promise, "thread");
      assert.equal(env.state.prepares, 1);
      assert.equal(env.state.contextCalls, 1);
      assert.equal(env.state.started, 1);
      assert.deepEqual(env.calls.map((call) => call.prepare ? "provider"
        : call.context ? "context" : call.method),
      ["provider", "thread/read", "config/read", "context", "context-check"]);
      assert.equal(env.state.candidates[0].start.request.model, "chosen");
      await env.until(() => env.state.leases === 1);
      await env.until(() => env.window.__codexPlusRpcResumeGates.get(env.port1).pending.size === 0);
      assert.deepEqual(env.failures, []);
    } finally { env.close(); }
  });

  it("rejects either preflight failure, releases candidate exports and allows retry", async () => {
    for (const failing of ["provider", "context"]) {
      const env = runtime();
      try {
        env.state.contextEnabled = true;
        env.state.failAt = failing === "provider" ? 1 : 0;
        env.state.contextFailure = failing === "context" ? "CONTEXT_REFRESH_FAILED" : "";
        env.install();
        await env.subscribe();
        const coordinator = await env.client.getTurnCoordinator();
        await assert.rejects(Promise.resolve(env.send(coordinator, {}, true).promise),
          failing === "provider" ? /LOCAL_PROVIDER_PREPARE_FAILED/ : /CONTEXT_REFRESH_FAILED/);
        assert.equal(env.state.started, 0);
        assert.equal(env.state.contextCalls, failing === "provider" ? 0 : 1);
        await env.until(() => env.state.leases === 1);
        await env.until(() => env.window.__codexPlusRpcResumeGates.get(env.port1).pending.size === 0);
        env.state.failAt = 0;
        env.state.contextFailure = "";
        assert.equal(await env.send(coordinator, {}, true).promise, "thread");
        await env.until(() => env.state.leases === 2);
        assert.equal(env.state.started, 1);
        assert.equal(await env.client.sendRequest("ping", {}), "pong");
        assert.deepEqual(env.failures, []);
      } finally { env.close(); }
    }
  });

  it("preserves send arguments and candidate choices without rewriting coordinator frames", async () => {
    const env = runtime();
    try {
      env.install();
      await env.subscribe();
      const coordinator = await env.client.getTurnCoordinator();
      const result = env.send(coordinator, { model: "chosen", reasoningEffort: "high", modelProvider: "custom" });
      assert.equal(await result.promise, "thread");
      assert.deepEqual(env.state.params[0], result.params);
      assert.deepEqual(result.callbacks, ["current", "admission", "added"]);
      assert.equal(env.state.prepares, 1);
      assert.equal(env.state.started, 1);
      assert.equal(env.calls.filter((call) => call.method === "config/read").length, 1);
      const send = env.frames.find(({ side, frame }) => side === "page"
        && frame?.[1]?.[2]?.[0] === "sendMessage");
      assert.equal(send!.frame[1][3].length, 5);
      assert.deepEqual(send!.frame[1][3][0], {
        conversationId: "thread", message: { input: [[{ type: "text", text: "next" }]],
          submissionOptions: { model: "chosen", reasoningEffort: "high", modelProvider: "custom" } },
        acceptLocally: true,
      });
      assert.equal(env.frames.some(({ frame }) => JSON.stringify(frame).includes('"remap"')), false);
      assert.deepEqual(env.calls.filter((call) => call.prepare).at(-1).params,
        { conversationId: "thread", model: "chosen", reasoningEffort: "high", modelProvider: "custom" });
      await env.until(() => env.window.__codexPlusRpcResumeGates.get(env.port1).pending.size === 0);
      assert.deepEqual(env.failures, []);
    } finally { env.close(); }
  });

  it("drains before same-port preflight and blocks the native start until preparation finishes", async () => {
    const env = runtime();
    let release!: () => void;
    try {
      env.install();
      await env.subscribe();
      const coordinator = await env.client.getTurnCoordinator();
      env.state.waitAt = 1;
      env.state.wait = new Promise<void>((resolve) => { release = resolve; });
      const result = env.send(coordinator);
      assert.equal(await env.client.sendRequest("ping", {}), "pong");
      await env.until(() => env.calls.some((call) => call.method === "config/read"));
      assert.equal(env.state.dispatches, 1);
      assert.equal(env.state.started, 0);
      release();
      assert.equal(await result.promise, "thread");
      assert.equal(env.state.prepares, 1);
      assert.deepEqual(env.failures, []);
    } finally { release?.(); env.close(); }
  });

  it("gates automatic prepare replay after an active send without another sendMessage push", async () => {
    const env = runtime();
    let release!: () => void;
    try {
      env.install();
      await env.subscribe();
      const coordinator = await env.client.getTurnCoordinator();
      env.state.active = true;
      assert.equal(await env.send(coordinator).promise, "queued");
      assert.equal(env.state.started, 0);
      env.state.active = false;
      env.state.waitAt = 2;
      env.state.wait = new Promise<void>((resolve) => { release = resolve; });
      const replay = env.state.replay!();
      await env.until(() => env.state.prepares === 2);
      assert.equal(env.state.candidates[0].start, undefined);
      assert.equal(await env.client.sendRequest("ping", {}), "pong");
      assert.equal(env.state.started, 0);
      release();
      await replay;
      assert.equal(env.state.started, 1);
      assert.equal(env.state.dispatches, 1);
      assert.equal(env.frames.filter(({ side, frame }) => side === "page"
        && frame?.[1]?.[2]?.[0] === "sendMessage").length, 1);
      assert.deepEqual(env.failures, []);
    } finally { release?.(); env.close(); }
  });

  it("keeps callback failure original, imports/releases candidate exports and allows retry", async () => {
    const env = runtime();
    try {
      env.install();
      await env.subscribe();
      const coordinator = await env.client.getTurnCoordinator();
      env.state.failAt = 1;
      await assert.rejects(Promise.resolve(env.send(coordinator, {}, true).promise),
        { name: "TypeError", message: "LOCAL_PROVIDER_PREPARE_FAILED" });
      assert.equal(env.state.started, 0);
      await env.until(() => env.state.leases === 1);
      await env.until(() => env.window.__codexPlusRpcResumeGates.get(env.port1).pending.size === 0);
      env.state.failAt = 0;
      const result = env.send(coordinator, { model: null, reasoningEffort: null }, true);
      assert.equal(await result.promise, "thread");
      await env.until(() => env.state.leases === 2);
      assert.equal(await env.client.sendRequest("ping", {}), "pong");
      assert.equal(env.state.started, 1);
      assert.deepEqual(env.failures, []);
    } finally { env.close(); }
  });

  it("covers a preinstalled cached coordinator without subscription or receiver learning", async () => {
    const env = runtime();
    try {
      const cached = await env.client.getTurnCoordinator();
      env.install();
      assert.equal(await env.send(cached).promise, "thread");
      assert.equal(env.state.prepares, 1);
      env.install();
      assert.equal(await env.send(cached).promise, "thread");
      assert.equal(env.state.prepares, 2);
      assert.equal(await env.send(env.client.getTurnCoordinator()).promise, "thread");
      assert.equal(env.state.prepares, 3);
      assert.equal(env.window.__codexPlusRpcFrameInterceptor.version, 19);
      assert.deepEqual(env.failures, []);
    } finally { env.close(); }
  });

  it("prefers the final resume provider, then restoreMessage options, without changing choices", async () => {
    for (const mode of ["resume", "steer", "nullable", "config", "middleware"]) {
      const env = runtime();
      try {
        env.install();
        env.state.candidate = (value) => {
          value.steer.restoreMessage.submissionOptions.modelProvider = "steer-provider";
          if (mode === "resume") value.resume.modelProvider = "resume-provider";
          if (mode === "nullable") value.resume.modelProvider = null;
          if (mode === "config") value.resume.config = { model_provider: "resume-provider" };
          return value;
        };
        if (mode === "middleware") {
          env.interceptors.set("codex/request", true);
          env.state.middleware = async (value) => value.method !== "turn/steer" ? value : {
            ...value, params: { ...value.params, restoreMessage: {
              submissionOptions: { modelProvider: "middleware-provider" },
            } },
          };
        }
        const coordinator = await env.client.getTurnCoordinator();
        assert.equal(await env.send(coordinator).promise, "thread");
        const params = env.calls.find((call) => call.prepare).params;
        assert.equal(params.modelProvider, mode === "resume" || mode === "config" ? "resume-provider"
          : mode === "middleware" ? "middleware-provider" : "steer-provider", mode);
        assert.equal(params.model, null);
        assert.equal(params.reasoningEffort, null);
        const candidate = env.state.candidates[0];
        assert.equal(candidate.resume.model, null);
        assert.equal(candidate.start.request.model, null);
        assert.equal(candidate.start.request.effort, null);
        assert.equal(candidate.resume.modelProvider,
          mode === "resume" ? "resume-provider" : mode === "nullable" ? null : undefined);
        assert.equal(env.state.prepares, 1);
        assert.deepEqual(env.failures, []);
      } finally { env.close(); }
    }
  });

  it("does not prepare a candidate redirected by middleware", async () => {
    const env = runtime();
    try {
      env.install();
      env.interceptors.set("codex/request", true);
      env.state.middleware = async (value) => value.method !== "turn/start" ? value
        : { ...value, params: { ...value.params, threadId: "other" } };
      assert.equal(await env.send(await env.client.getTurnCoordinator()).promise, "thread");
      assert.equal(env.state.prepares, 0);
      assert.equal(env.state.discoveries, 0);
      assert.deepEqual(env.failures, []);
    } finally { env.close(); }
  });

  it("sends extension rejection directly without a provider gate or pending call", async () => {
    const env = runtime();
    try {
      env.install();
      env.listeners.set("agent/pre-step", new Set([{ scriptKey: "test",
        handler: () => ({ kind: "reject", reason: "EXTENSION_BLOCKED" }) }]));
      const coordinator = await env.client.getTurnCoordinator();
      await assert.rejects(Promise.resolve(env.send(coordinator).promise), /EXTENSION_BLOCKED/);
      assert.equal(env.state.prepares, 0);
      assert.equal(env.state.discoveries, 0);
      assert.equal(env.state.started, 0);
      assert.equal(env.window.__codexPlusRpcResumeGates.has(env.port1), false);
      assert.equal(await env.client.sendRequest("ping", {}), "pong");
      env.listeners.clear();
      assert.equal(await env.send(coordinator).promise, "thread");
      assert.deepEqual(env.failures, []);
    } finally { env.close(); }
  });

  it("passes remote, unidentified, mismatched and disabled candidates through", async () => {
    for (const mode of ["remote", "unknown", "resume-mismatch", "start-mismatch", "disabled"]) {
      const env = runtime();
      try {
        env.install();
        if (mode === "disabled") env.settings.providerSyncEnabled = false;
        env.state.candidate = (value) => {
          if (mode === "remote") value.executionHostId = "ssh";
          if (mode === "unknown") delete value.executionHostId;
          if (mode === "resume-mismatch") value.resume.conversationId = "other";
          if (mode === "start-mismatch") value.start.request.threadId = "other";
          return value;
        };
        const coordinator = await env.client.getTurnCoordinator();
        assert.equal(await env.send(coordinator).promise, "thread");
        assert.equal(env.state.prepares, 0, mode);
        assert.equal(env.state.discoveries, 0, mode);
        assert.equal(env.window.__codexPlusRpcResumeGates.has(env.port1), false, mode);
        assert.deepEqual(env.failures, []);
      } finally { env.close(); }
    }
  });

  it("shares existing gate ids on upgrade and keeps the resume path functional", async () => {
    const env = runtime();
    try {
      const previousRpc = rpc.replace("codexPlusRpcInterceptorVersion = 19", "codexPlusRpcInterceptorVersion = 18");
      assert.notEqual(previousRpc, rpc, "the test must install an older interceptor version");
      env.install(previousRpc);
      await env.subscribe();
      assert.deepEqual(await env.client.resumeThread({ threadId: "thread", model: null }),
        { threadId: "thread", model: null });
      env.install();
      const coordinator = await env.client.getTurnCoordinator();
      assert.equal(await env.send(coordinator).promise, "thread");
      const ids = env.frames.filter(({ side, frame }) => side === "page" && frame?.[0] === "resolve"
        && frame[1] < 0 && !Number.isInteger(frame[1])).map(({ frame }) => frame[1]);
      assert.deepEqual(ids, [-0.25, -1.25]);
      assert.deepEqual(env.failures, []);
    } finally { env.close(); }
  });
});
