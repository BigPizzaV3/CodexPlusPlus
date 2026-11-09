// node src/runtime-context-capnweb-failure.manual.cjs <app.asar> <capnweb-chunk> [rpc-commit]
// Reads the installed Fkn8 module; both peers and every backend method stay in memory.
// Verified with OpenAI.Codex_26.1103.4239.0, webview/assets/async/885188.3857056929.js.
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { describe, it } = require("node:test");
const { MessageChannel } = require("node:worker_threads");

const [, , asarPath, chunkPath, rpcCommit] = process.argv;
assert.ok(asarPath && chunkPath, "pass the installed app.asar and capnweb chunk paths");
const fd = fs.openSync(asarPath, "r");
let chunk;
let callerChunk;
let hostSource;
try {
  const header = Buffer.alloc(16);
  fs.readSync(fd, header, 0, header.length, 0);
  const json = Buffer.alloc(header.readUInt32LE(12));
  fs.readSync(fd, json, 0, json.length, 16);
  const root = JSON.parse(json.toString());
  const read = (relative) => {
    let entry = root;
    for (const part of relative.split("/")) entry = entry.files[part];
    const bytes = Buffer.alloc(entry.size);
    fs.readSync(fd, bytes, 0, bytes.length, 8 + header.readUInt32LE(4) + Number(entry.offset));
    return bytes.toString();
  };
  chunk = read(chunkPath);
  callerChunk = read("webview/assets/async/440338.88139063bc.js");
  hostSource = read(".vite/build/early-bootstrap-199-e3316cddb4ea6a6b.js");
} finally {
  fs.closeSync(fd);
}
const modules = new Function(`${chunk.replaceAll("export const ", "const ")}\nreturn __webpack_modules__`)();
assert.equal(typeof modules.Fkn8, "function", "the selected chunk must contain capnweb's Fkn8 module");
const cap = {};
const exportsApi = {
  d(target, getters, values) {
    Object.assign(target, values);
    for (const [key, get] of Object.entries(getters)) Object.defineProperty(target, key, { get, enumerable: true });
  },
};
modules.Fkn8({}, cap, exportsApi);
const callerModules = new Function(`${callerChunk.replaceAll("export const ", "const ")}\nreturn __webpack_modules__`)();
const startOffset = hostSource.indexOf("async startConversation(e,t){return async function(");
const startEnd = hostSource.indexOf("async forkConversationFromLatest(", startOffset);
const inputsOffset = hostSource.indexOf("async createConversation(e,t,r,n,i,s){");
const inputsEnd = hostSource.indexOf("}async ", inputsOffset);
const normalizeOffset = hostSource.indexOf("async function Rl(");
const normalizeEnd = hostSource.indexOf("var Rc=", normalizeOffset);
assert.ok(startOffset > 0 && startEnd > startOffset && inputsOffset > 0 && inputsEnd > inputsOffset);
assert.ok(normalizeOffset > 0 && normalizeEnd > normalizeOffset);
const readBeforeCreate = new Function(`return ({${hostSource.slice(inputsOffset, inputsEnd + 1)}}).createConversation`)();
const normalize = new Function("bA", "b8", `${hostSource.slice(normalizeOffset, normalizeEnd)}\nreturn Rl;`)(
  "cloud", () => ({}));
const nativeStart = new Function("Rl", "bT", "p0", "bC",
  `return ({${hostSource.slice(startOffset, startEnd)}}).startConversation;`)(
  normalize, () => Promise.withResolvers(), { q3: class extends Error {} }, (error) => error.message);
const root = path.resolve(__dirname, "../../..");
let source;
if (rpcCommit) {
  assert.match(rpcCommit, /^[0-9a-f]{40}$/i, "the optional baseline must be a full commit id");
  source = execFileSync("git", ["show", `${rpcCommit}:assets/inject/renderer-inject/91-extension-rpc.js`],
    { cwd: root, encoding: "utf8" });
} else {
  source = fs.readFileSync(path.join(root, "assets/inject/renderer-inject/91-extension-rpc.js"), "utf8");
}

const nativeWorld = { MessageEvent };

function fixture(nativeLifecycle = false, nativePorts = false, cloneReads = false) {
  const calls = [];
  const toasts = [];
  const failures = [];
  const listeners = new Map();
  const interceptors = new Map();
  const state = { startFailure: "", refreshFailure: "", chain: async (value) => value,
    nativeReadFailure: true, inputs: 0, settlements: [], pending: true };
  class PortMessageEvent extends MessageEvent {
    get data() { return cloneReads ? structuredClone(super.data) : super.data; }
  }
  const EventClass = nativePorts ? MessageEvent : PortMessageEvent;
  class Port {
    listeners = [];
    sent = [];
    received = [];
    start() {}
    close() {}
    addEventListener(type, listener, options) {
      if (type === "message") this.listeners.push({ listener, capture: !!options?.capture });
    }
    postMessage(message) {
      this.sent.push(structuredClone(message));
      const data = structuredClone(message);
      queueMicrotask(() => this.peer.dispatchEvent(new EventClass("message", { data })));
    }
    dispatchEvent(event) {
      Object.defineProperty(event, "currentTarget", { configurable: true, value: this });
      let stopped = false;
      const stop = event.stopImmediatePropagation.bind(event);
      event.stopImmediatePropagation = () => { stopped = true; stop(); };
      const ordered = cloneReads ? this.listeners
        : this.listeners.toSorted((a, b) => Number(b.capture) - Number(a.capture));
      for (const { listener } of ordered) {
        listener(event);
        if (stopped) break;
      }
      this.received.push(structuredClone(event.data));
      return true;
    }
  }
  class PagePort extends Port {}
  class HostPort extends Port {}
  const channel = nativePorts ? new MessageChannel() : null;
  const page = channel?.port1 || new PagePort();
  const host = channel?.port2 || new HostPort();
  if (nativePorts) {
    for (const port of [page, host]) {
      const send = port.postMessage.bind(port);
      port.sent = [];
      port.postMessage = (message) => { port.sent.push(structuredClone(message)); return send(message); };
    }
  } else {
    page.peer = host;
    host.peer = page;
  }
  const window = nativePorts ? nativeWorld : { MessageEvent: EventClass };
  window.MessagePort = nativePorts ? { prototype: page } : PagePort;
  // Each native fixture creates a different endpoint, in the same Node event realm.
  if (nativePorts) delete window.__codexPlusRpcFrameInterceptor;
  const inject = new Function("window", "codexPlusExtensionListeners", "codexPlusExtensionInterceptors",
    "codexPlusRunWaterfall", "codexPlusMarkExtensionFailure", "showToast", "codexPerModelContextEnabled",
    "prepareCodexThreadContextStart", "codexPlusAppServerClient", "refreshCodexThreadModelBeforeTurn",
    "runCodexPlusExtensionCallback", "MessageEvent", source);
  let client;
  const reinject = () => inject(window, listeners, interceptors, (_name, value) => state.chain(value),
    (_key, message) => failures.push(message), (message) => toasts.push(message), () => true,
    async (_method, params) => {
      if (state.startFailure) throw new Error(state.startFailure);
      return { ...params, config: { model_context_window: 262144 } };
    }, async () => client,
    async (_host, sendRequest, _method, params) => {
      if (state.refreshFailure) throw new Error(state.refreshFailure);
      await sendRequest("thread/read", { threadId: params.threadId });
      await sendRequest("thread/resume", { threadId: params.threadId });
    }, (_key, _name, run) => ({ ok: true, value: run() }), EventClass);
  reinject();
  const manager = {
    assertActive() {}, getHostId: () => "local",
    readDefaultPersonality: async () => { await state.beforeInputs; return null; },
    settings: { readDefaultFeatureOverrides: () => ({}) },
    threadStartedNotificationDeferral: { run: (_defer, run) => run() },
    threadCreation: {
      async createConversation(params, readInputs) {
        // Failure runs the installed method up to its real input await. The backend is never run.
        if (state.nativeReadFailure) {
          return readBeforeCreate.call({ getRequestedThreadHistoryMode: () => "full" }, params, readInputs);
        }
        // Successful retry keeps the installed outer lifecycle, with an in-memory backend.
        try {
          const inputs = await readInputs(params, "full");
          inputs[Symbol.dispose]?.();
        } finally { readInputs[Symbol.dispose](); }
        calls.push("thread/start");
        return { conversationId: "created", conversationResponse: { thread: {} } };
      },
      async saveInitialDaybreak() {},
    },
    threadStore: { notifyConversationCallbacks() {} }, broadcastConversationSnapshot() {},
    updateThreadDaybreakEnabled() {},
    executeTurnStart() { throw new Error("the lifecycle fixture must not generate a turn"); },
    runtimeSettings: {}, logger: { warning: (_message, error) => failures.push(error) },
  };
  class Host extends cap.TF {
    subscribe() { return null; }
    startConversation(params, callbacks) {
      if (nativeLifecycle) {
        calls.push("native:startConversation");
        return nativeStart.call(manager, params, callbacks);
      }
      calls.push("startConversation");
      return callbacks.onSettled(params.config.model_context_window);
    }
    sendRequest(method, params, options) {
      calls.push(method);
      return options?.onSettled ? options.onSettled(params.config?.model_context_window) : params;
    }
    echo(callback, value) {
      calls.push("echo");
      return callback.echo(value);
    }
    async getTurn(provider) {
      const turn = await provider.getTurn();
      calls.push("turn/start");
      return turn.conversationId;
    }
  }
  client = cap.ab(page);
  cap.ab(host, new Host());
  class Callback extends cap.TF {
    echo(value) { return value; }
  }
  class TurnProvider extends cap.TF {
    getTurn() {
      return { executionHostId: "local", conversationId: "thread", start: { request: {
        threadId: "thread", clientUserMessageId: "turn-message", input: [{ type: "text", text: "fixture" }],
      } } };
    }
  }
  const callback = new cap.AX(new Callback());
  const params = { clientUserMessageId: "message", input: [{ type: "text", text: "fixture" }],
    nested: { callback } };
  const callbacks = { onSettled: (value) => value };
  const nativeCaller = {};
  const deps = {
    iRMi: { CodexRemoteSshThreadAction: {}, CodexRemoteSshThreadResult: {} },
    HVBu: (run) => run, kpNK: { n: () => false }, S8um: { F2: "fixture-feature" },
    IV7t: { Ax: () => ({ trackThreadEvent() {} }) }, Dgwo: { V: () => client },
    TSah: { v: () => ({ readCreationInputs: async () => {
      state.inputs++;
      return { hasDesktopRuntime: true, readDynamicTools: () => [], nested: callback.dup() };
    } }) }, cQEU: { sv() {} }, SjAp: { j: {
      markThreadCreationStarted() {}, getTrace() {}, markThreadInputsReady() {}, markThreadCreated() {},
    } }, kdqP: { Q() {} },
  };
  const requireCaller = (name) => deps[name] || {};
  requireCaller.d = exportsApi.d;
  requireCaller.n = (value) => () => value;
  callerModules.lqHG({}, nativeCaller, requireCaller);
  return {
    client, window, page, host, calls, toasts, failures, state, listeners, interceptors, callback, params, callbacks,
    reinject, subscribe: () => client.subscribe({ key: { hostId: "local" } }),
    turn: () => Promise.resolve(client.getTurn(new TurnProvider())),
    nativeStart: (input = params, onSettled = () => {}) => nativeCaller.g({ get: () => false }, "local", input, {
      onSettled: async (outcome) => {
        state.settlements.push(outcome);
        state.pending = false;
        await onSettled(outcome);
      },
    }),
  };
}

describe("installed capnweb rejected push calls", () => {
  for (const entry of ["startConversation", "thread/start", "thread/fork"]) {
    it(`preserves catalog errors, callback exports and later RPC after ${entry} fails`, { timeout: 3000 }, async () => {
      const env = fixture();
      await env.subscribe();
      env.state.startFailure = "catalog unreadable";
      const blocked = entry === "startConversation"
        ? env.client.startConversation(env.params, env.callbacks)
        : env.client.sendRequest(entry, env.params, env.callbacks);
      const rejection = assert.rejects(Promise.resolve(blocked), { name: "Error", message: "catalog unreadable" });
      const pipeline = assert.rejects(Promise.resolve(blocked.unreachable()), { message: "catalog unreadable" });
      const later = Promise.resolve(env.client.echo(env.callback, "later RPC"));
      await Promise.all([rejection, pipeline]);
      assert.equal(await later, "later RPC");
      assert.deepEqual(env.calls, ["echo"]);
      assert.deepEqual(env.toasts, ["catalog unreadable"]);
      const failedFrame = env.page.sent.find((frame) => frame?.[0] === "push"
        && frame[1]?.[2]?.[0] === (entry === "startConversation" ? entry : "sendRequest"));
      const args = failedFrame[1][3];
      const paramsIndex = entry === "startConversation" ? 0 : 1;
      for (const [kind, id] of [args[paramsIndex].nested.callback, args[paramsIndex + 1].onSettled]) {
        assert.equal(kind, "export", "both nested and callback exports must reach the peer");
        assert.ok(env.host.sent.some((frame) => frame?.[0] === "release" && frame[1] === id),
          `the peer must release callback export ${id}`);
      }
      assert.equal(env.window.__codexPlusRpcRejectionPorts.get(env.page).pending.size, 0);
      env.state.startFailure = "";
      assert.equal(await env.client.startConversation({ ...env.params, clientUserMessageId: "fresh" }, env.callbacks),
        262144);
      assert.deepEqual(env.calls, ["echo", "startConversation"]);
      assert.deepEqual(env.failures, []);
      assert.ok(!env.page.sent.includes(null) && !env.host.sent.includes(null), "neither peer may abort");
    });
  }

  it("uses distinct rejection promises across repeated failures and reinjection", { timeout: 3000 }, async () => {
    const env = fixture();
    await env.subscribe();
    env.state.startFailure = "catalog unreadable";
    await assert.rejects(Promise.resolve(env.client.sendRequest("thread/start", env.params, env.callbacks)),
      { message: "catalog unreadable" });
    env.reinject();
    await assert.rejects(Promise.resolve(env.client.sendRequest("thread/fork", env.params, env.callbacks)),
      { message: "catalog unreadable" });
    const rejections = env.page.sent.filter((frame) => frame?.[0] === "reject");
    assert.equal(rejections.length, 2);
    assert.notEqual(rejections[0][1], rejections[1][1]);
    assert.equal(env.window.__codexPlusRpcRejectionPorts.get(env.page).pending.size, 0);
    assert.equal(await env.client.echo(env.callback, "still connected"), "still connected");
    assert.ok(!env.page.sent.includes(null) && !env.host.sent.includes(null));
  });

  it("preserves an extension rejection without running context preparation", { timeout: 3000 }, async () => {
    const env = fixture();
    await env.subscribe();
    env.state.startFailure = "context preparation must not run";
    env.listeners.set("agent/pre-step", new Set([{ scriptKey: "fixture",
      handler: () => ({ kind: "reject", reason: "blocked by extension" }) }]));
    await assert.rejects(Promise.resolve(env.client.startConversation(env.params, env.callbacks)),
      { message: "blocked by extension" });
    assert.deepEqual(env.calls, []);
    assert.deepEqual(env.toasts, ["blocked by extension"]);
    assert.equal(await env.client.echo(env.callback, "still connected"), "still connected");
  });

  it("refreshes through the same port and keeps a failed turn from dispatching", { timeout: 3000 }, async () => {
    const env = fixture();
    await env.subscribe();
    assert.equal(await env.turn(), "thread");
    assert.deepEqual(env.calls, ["thread/read", "thread/resume", "turn/start"]);
    env.state.refreshFailure = "turn refresh failed";
    await assert.rejects(env.turn(), { message: "turn refresh failed" });
    assert.deepEqual(env.calls, ["thread/read", "thread/resume", "turn/start"]);
    assert.equal(await env.client.echo(env.callback, "still connected"), "still connected");
  });

  it("runs installed caller/host settlement and finally after creation inputs reject", { timeout: 3000 }, async () => {
    const env = fixture(true);
    await env.subscribe();
    env.state.startFailure = "catalog unreadable";
    await assert.rejects(env.nativeStart(), { message: "catalog unreadable" });
    assert.deepEqual(env.calls, ["native:startConversation"]);
    assert.deepEqual(env.state.settlements, [{ status: "failed", message: "catalog unreadable" }]);
    assert.equal(env.state.pending, false);
    assert.equal(env.state.inputs, 0, "the rejected callback must not execute page input preparation");
    const start = env.page.sent.find((frame) => frame[0] === "push" && frame[1]?.[2]?.[0] === "startConversation");
    assert.equal(start[1][3].length, 2, "the original host method must receive its original arguments");
    const readInputs = start[1][3][1].readThreadCreationInputs;
    assert.equal(readInputs[0], "export", "the installed caller really exports this callback");
    const callbackIndex = env.host.sent.findIndex((frame) =>
      frame[0] === "push" && frame[1]?.[1] === readInputs[1] && frame[1]?.[2]?.length === 0);
    assert.ok(callbackIndex >= 0);
    const resultId = env.host.sent.slice(0, callbackIndex + 1)
      .filter((frame) => ["push", "stream", "pipe"].includes(frame[0])).length;
    assert.ok(env.page.sent.some((frame) => frame[0] === "reject" && frame[1] === resultId
      && frame[2][2] === "catalog unreadable"), "real capnweb assigns the callback result id");
    for (const value of Object.values(start[1][3][1])) {
      if (Array.isArray(value) && value[0] === "export") {
        assert.ok(env.host.sent.some((frame) => frame[0] === "release" && frame[1] === value[1]),
          `native finally must release callback export ${value[1]}`);
      }
    }
    const creation = env.window.__codexPlusRpcCreationFailures.get(env.page);
    assert.equal(creation.pending.size, 0);
    assert.equal(creation.callbacks.size, 0);
    env.reinject();
    env.state.startFailure = "";
    env.state.nativeReadFailure = false;
    const retry = await env.nativeStart({ ...env.params, clientUserMessageId: "retry", input: [] });
    assert.equal(retry.status, "created");
    assert.equal(retry.firstTurn.status, "not-requested");
    assert.equal(env.state.inputs, 1);
    assert.equal(await env.client.echo(env.callback, "still connected"), "still connected");
    assert.deepEqual(env.failures, []);
    assert.ok(!env.page.sent.includes(null) && !env.host.sent.includes(null));
  });

  it("uses the installed host lifecycle for extension rejection too", { timeout: 3000 }, async () => {
    const env = fixture(true);
    await env.subscribe();
    env.state.startFailure = "context preparation must not run";
    env.listeners.set("agent/pre-step", new Set([{ scriptKey: "fixture",
      handler: () => ({ kind: "reject", reason: "blocked by extension" }) }]));
    await assert.rejects(env.nativeStart(), { message: "blocked by extension" });
    assert.deepEqual(env.state.settlements, [{ status: "failed", message: "blocked by extension" }]);
    assert.deepEqual(env.calls, ["native:startConversation"]);
    assert.equal(env.state.inputs, 0);
    assert.equal(await env.client.echo(env.callback, "still connected"), "still connected");
    assert.deepEqual(env.failures, []);
  });

  it("rejects native creation inputs over real in-memory MessagePorts and recovers", { timeout: 3000 }, async (t) => {
    const env = fixture(true, true);
    t.after(() => { env.page.close(); env.host.close(); });
    await env.subscribe();
    env.state.startFailure = "catalog unreadable";
    await assert.rejects(env.nativeStart(), { message: "catalog unreadable" });
    assert.deepEqual(env.calls, ["native:startConversation"]);
    assert.deepEqual(env.state.settlements, [{ status: "failed", message: "catalog unreadable" }]);
    assert.equal(env.state.pending, false);
    const creation = env.window.__codexPlusRpcCreationFailures.get(env.page);
    assert.equal(creation.pending.size, 0);
    assert.equal(creation.callbacks.size, 0);
    env.state.startFailure = "";
    env.state.nativeReadFailure = false;
    const retry = await env.nativeStart({ ...env.params, clientUserMessageId: "retry", input: [] });
    assert.equal(retry.status, "created");
    assert.equal(retry.firstTurn.status, "not-requested");
    assert.equal(await env.client.echo(env.callback, "still connected"), "still connected");
    assert.deepEqual(env.failures, []);
  });

  it("keeps an in-flight native callback failure across reinjection and settlement RPC", { timeout: 3000 }, async () => {
    const env = fixture(true);
    await env.subscribe();
    const gate = Promise.withResolvers();
    env.state.beforeInputs = gate.promise;
    env.state.startFailure = "catalog unreadable";
    let settledRpc = false;
    const rejection = assert.rejects(env.nativeStart(undefined, async () => {
      assert.equal(await env.client.echo(env.callback, "settlement RPC"), "settlement RPC");
      settledRpc = true;
    }), { message: "catalog unreadable" });
    assert.equal(await env.client.echo(env.callback, "while host waits"), "while host waits");
    env.reinject();
    gate.resolve();
    await rejection;
    assert.equal(settledRpc, true);
    assert.deepEqual(env.state.settlements, [{ status: "failed", message: "catalog unreadable" }]);
    assert.equal(env.state.inputs, 0);
    assert.equal(env.window.__codexPlusRpcCreationFailures.get(env.page).callbacks.size, 0);
    assert.deepEqual(env.failures, []);
  });

  it("rejects creation when the existing transport reads first and data is cloned on each read", { timeout: 3000 }, async () => {
    const env = fixture(true, false, true);
    await env.subscribe();
    env.state.nativeReadFailure = false;
    env.state.startFailure = "catalog unreadable";
    await assert.rejects(env.nativeStart({ ...env.params, input: [] }), { message: "catalog unreadable" });
    assert.deepEqual(env.calls, ["native:startConversation"]);
    assert.equal(env.state.inputs, 0);
    assert.deepEqual(env.state.settlements, [{ status: "failed", message: "catalog unreadable" }]);
    assert.equal(await env.client.echo(env.callback, "settled"), "settled");
    assert.deepEqual(env.failures, []);
  });

  it("treats resolve for an unknown half-integer import as a protocol no-op on real MessagePorts", { timeout: 3000 }, async (t) => {
    const env = fixture(false, true);
    t.after(() => { env.page.close(); env.host.close(); });
    await env.subscribe();
    env.host.postMessage(["resolve", 0.5, ["undefined"]]);
    assert.equal(await env.client.echo(env.callback, "after unknown resolve"), "after unknown resolve");
    env.state.startFailure = "catalog unreadable";
    await assert.rejects(Promise.resolve(env.client.sendRequest("thread/start", env.params, env.callbacks)),
      { message: "catalog unreadable" });
    assert.equal(env.window.__codexPlusRpcRejectionPorts.get(env.page).pending.size, 0);
    assert.equal(await env.client.echo(env.callback, "after synthetic release"), "after synthetic release");
    assert.ok(!env.page.sent.includes(null) && !env.host.sent.includes(null));
    assert.deepEqual(env.failures, []);
  });
});
