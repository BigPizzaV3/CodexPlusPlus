// node src/runtime-context-capnweb-failure.manual.cjs <app.asar> <capnweb-chunk> [rpc-commit]
// Reads the installed Fkn8 module; both peers and every backend method stay in memory.
// Verified with OpenAI.Codex_26.1103.4239.0, webview/assets/async/885188.3857056929.js.
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { describe, it } = require("node:test");

const [, , asarPath, chunkPath, rpcCommit] = process.argv;
assert.ok(asarPath && chunkPath, "pass the installed app.asar and capnweb chunk paths");
const fd = fs.openSync(asarPath, "r");
let chunk;
try {
  const header = Buffer.alloc(16);
  fs.readSync(fd, header, 0, header.length, 0);
  const json = Buffer.alloc(header.readUInt32LE(12));
  fs.readSync(fd, json, 0, json.length, 16);
  let entry = JSON.parse(json.toString());
  for (const part of chunkPath.split("/")) entry = entry.files[part];
  const bytes = Buffer.alloc(entry.size);
  fs.readSync(fd, bytes, 0, bytes.length, 8 + header.readUInt32LE(4) + Number(entry.offset));
  chunk = bytes.toString();
} finally {
  fs.closeSync(fd);
}
const modules = new Function(`${chunk.replaceAll("export const ", "const ")}\nreturn __webpack_modules__`)();
assert.equal(typeof modules.Fkn8, "function", "the selected chunk must contain capnweb's Fkn8 module");
const cap = {};
modules.Fkn8({}, cap, {
  d(target, getters, values) {
    Object.assign(target, values);
    for (const [key, get] of Object.entries(getters)) Object.defineProperty(target, key, { get, enumerable: true });
  },
});
const root = path.resolve(__dirname, "../../..");
let source;
if (rpcCommit) {
  assert.match(rpcCommit, /^[0-9a-f]{40}$/i, "the optional baseline must be a full commit id");
  source = execFileSync("git", ["show", `${rpcCommit}:assets/inject/renderer-inject/91-extension-rpc.js`],
    { cwd: root, encoding: "utf8" });
} else {
  source = fs.readFileSync(path.join(root, "assets/inject/renderer-inject/91-extension-rpc.js"), "utf8");
}

function fixture() {
  const calls = [];
  const toasts = [];
  const failures = [];
  const listeners = new Map();
  const interceptors = new Map();
  const state = { startFailure: "", refreshFailure: "", chain: async (value) => value };
  class Port {
    listeners = [];
    sent = [];
    start() {}
    close() {}
    addEventListener(type, listener, options) {
      if (type === "message") this.listeners.push({ listener, capture: !!options?.capture });
    }
    postMessage(message) {
      this.sent.push(structuredClone(message));
      const data = structuredClone(message);
      queueMicrotask(() => {
        let stopped = false;
        // DOM MessagePort dispatches target capture listeners before ordinary listeners.
        for (const { listener } of this.peer.listeners.toSorted((a, b) => Number(b.capture) - Number(a.capture))) {
          listener({ data, stopImmediatePropagation() { stopped = true; } });
          if (stopped) break;
        }
      });
    }
  }
  class PagePort extends Port {}
  class HostPort extends Port {}
  const page = new PagePort();
  const host = new HostPort();
  page.peer = host;
  host.peer = page;
  const window = { MessagePort: PagePort };
  const inject = new Function("window", "codexPlusExtensionListeners", "codexPlusExtensionInterceptors",
    "codexPlusRunWaterfall", "codexPlusMarkExtensionFailure", "showToast", "codexPerModelContextEnabled",
    "prepareCodexThreadContextStart", "codexPlusAppServerClient", "refreshCodexThreadModelBeforeTurn",
    "runCodexPlusExtensionCallback", source);
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
    }, (_key, _name, run) => ({ ok: true, value: run() }));
  reinject();
  class Host extends cap.TF {
    subscribe() { return null; }
    startConversation(params, callbacks) {
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
  return {
    client, window, page, host, calls, toasts, failures, state, listeners, interceptors, callback, params, callbacks,
    reinject, subscribe: () => client.subscribe({ key: { hostId: "local" } }),
    turn: () => Promise.resolve(client.getTurn(new TurnProvider())),
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
});
