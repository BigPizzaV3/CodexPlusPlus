import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/91-extension-rpc.js", import.meta.url), "utf8");
type Frame = any[];

function runtime() {
  const sent: Frame[] = [];
  const received: Frame[] = [];
  const calls: any[] = [];
  const toasts: string[] = [];
  const failures: string[] = [];
  const interceptors = new Map<string, boolean>();
  const state = {
    enabled: true,
    startFailure: "",
    refreshFailure: "",
    chain: async (value: any) => value,
  };
  const acknowledgements = new Map<string, () => void>();
  class PortMessageEvent extends MessageEvent<any> {
    get data() { return super.data; }
  }
  class MessagePort {
    listeners: any[] = [];
    addEventListener(type: string, listener: any, options: any) {
      assert.equal(type, "message");
      assert.equal(options.capture, true);
      this.listeners.push(listener);
    }
    postMessage(frame: Frame) {
      sent.push(frame);
      if (frame[0] === "push") acknowledgements.get(frame[1][2][0])?.();
    }
    dispatchEvent(event: MessageEvent) {
      Object.defineProperty(event, "currentTarget", { value: this });
      for (const listener of this.listeners) listener(event);
      received.push(event.data);
      return true;
    }
  }
  const window = { MessagePort, MessageEvent: PortMessageEvent } as any;
  const port = new MessagePort();
  const client = {
    async sendRequest(method: string, params: any, options: any) {
      calls.push({ method, params, options });
      await new Promise<void>((resolve) => {
        acknowledgements.set(method, resolve);
        port.postMessage(["push", ["pipeline", -19, [method], []]]);
      });
      return {};
    },
    async unsubscribeInactiveConversation(threadId: string) {
      calls.push({ method: "unsubscribeInactiveConversation", threadId });
    },
    async resumeThread(params: any) {
      calls.push({ method: "resumeThread", params });
    },
  };
  const create = new Function("window", "codexPlusExtensionListeners", "codexPlusExtensionInterceptors",
    "codexPlusRunWaterfall", "codexPlusMarkExtensionFailure", "showToast", "codexPerModelContextEnabled",
    "prepareCodexThreadContextStart", "codexPlusAppServerClient", "refreshCodexThreadModelBeforeTurn",
    "runCodexPlusExtensionCallback",
    `${source}\nreturn { decode: codexPlusRpcDecode, install: installCodexPlusRpcFrameInterceptor };`);
  const listeners = new Map();
  const api = create(window, listeners, interceptors, (_name: string, value: any) => state.chain(value),
    (_key: string, message: string) => failures.push(message), (message: string) => toasts.push(message),
    () => state.enabled,
    async (method: string, params: any, hostId: string) => {
      calls.push({ method: "prepare", requestMethod: method, params, hostId });
      if (state.startFailure) throw new Error(state.startFailure);
      return { ...params, config: { ...params.config, model_context_window: 262144,
        model_auto_compact_token_limit: 235929 } };
    },
    async (hostId: string) => {
      assert.equal(hostId, "local");
      return client;
    },
    async (host: any, sendRequest: any, method: string, params: any) => {
      calls.push({ method: "refresh", host, requestMethod: method, params });
      if (state.refreshFailure) throw new Error(state.refreshFailure);
      await sendRequest("thread/read", { threadId: params.threadId }, { timeoutMs: 5000 });
      await sendRequest("thread/resume", { threadId: params.threadId, excludeTurns: true }, { timeoutMs: 5000 });
      await sendRequest("thread/unsubscribe", { threadId: params.threadId }, { timeoutMs: 5000 });
      await sendRequest("thread/resume", { threadId: params.threadId, config: { model_context_window: 262144 } });
    }, (_key: string, _name: string, run: () => any) => ({ ok: true, value: run() }));
  const settle = async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
  };
  const subscribe = (hostId: string, receiver = -19, target = port) =>
    target.postMessage(["push", ["pipeline", receiver, ["subscribe"], [{
      type: "notification", key: { hostId }, methods: [["turn/completed"]], listener: ["export", -3],
    }]]]);
  return { api, window, port, sent, received, calls, state, toasts, failures, listeners, interceptors, subscribe, settle };
}

function startFrame(receiver = -19): Frame {
  return ["push", ["pipeline", receiver, ["startConversation"], [{
    clientUserMessageId: "message", input: [[{ type: "text", text: "fixture" }]],
    config: { "features.web_search": false },
    collaborationMode: { mode: "default", settings: { model: "supplier/model", reasoning_effort: "high" } },
  }, { onSettled: ["export", -5] }]]];
}

function turnFrame(hostId: string | undefined = "local"): Frame {
  return ["resolve", 7, {
    executionHostId: hostId, conversationId: "thread",
    resume: { conversationId: "thread", collaborationMode: { settings: { model: "supplier/model" } } },
    start: { request: { threadId: "thread", input: [[{ type: "text", text: "fixture" }]],
      collaborationMode: { settings: { model: "supplier/model" } } } },
    steer: { input: [[{ type: "text", text: "fixture" }]] },
  }];
}

describe("runtime context through host RPC", () => {
  it("adds fresh settings before creating a proven local conversation and preserves callbacks", async () => {
    const env = runtime();
    env.subscribe("local");
    const frame = startFrame();
    env.port.postMessage(frame);
    await env.settle();
    const created = env.sent.at(-1)![1][3];
    assert.deepEqual(created[0].config, { "features.web_search": false,
      model_context_window: 262144, model_auto_compact_token_limit: 235929 });
    assert.equal(created[1], frame[1][3][1]);
    assert.equal(env.calls[0].requestMethod, "thread/start");
    assert.equal(env.calls.some((call) => call.method === "refresh"), false);
  });

  it("leaves remote, unknown and same-receiver other-port starts untouched", () => {
    const env = runtime();
    env.subscribe("local");
    env.subscribe("remote", -20);
    const remote = startFrame(-20);
    const unknown = startFrame(-21);
    const otherPort = new env.window.MessagePort();
    const other = startFrame();
    env.port.postMessage(remote);
    env.port.postMessage(unknown);
    otherPort.postMessage(other);
    assert.equal(env.sent.at(-3), remote);
    assert.equal(env.sent.at(-2), unknown);
    assert.equal(env.sent.at(-1), other);
    assert.equal(env.calls.length, 0);
  });

  it("also adds context to local raw thread/start and thread/fork calls", async () => {
    for (const method of ["thread/start", "thread/fork"]) {
      const env = runtime();
      env.subscribe("local");
      env.port.postMessage(["push", ["pipeline", -19, ["sendRequest"], [method, { model: "supplier/model" }]]]);
      await env.settle();
      assert.equal(env.sent.at(-1)![1][3][1].config.model_context_window, 262144);
      assert.equal(env.calls[0].requestMethod, method);
    }
  });

  it("refreshes a local turn without blocking its own same-port requests", async () => {
    const env = runtime();
    const frame = turnFrame();
    env.port.postMessage(frame);
    await env.settle();
    assert.deepEqual(env.calls.map((call) => call.method),
      ["refresh", "thread/read", "thread/resume", "unsubscribeInactiveConversation", "thread/unsubscribe", "resumeThread"]);
    assert.deepEqual(env.calls[0].host, { hostId: "local" });
    assert.equal(env.calls[3].threadId, "thread");
    assert.equal(env.calls[5].params.config.model_context_window, 262144);
    assert.equal(env.sent.at(-1)![0], "resolve");
    assert.equal(env.sent.at(-1)![2].resume.conversationId, "thread");
  });

  it("waits for an existing push queue, then lets preflight RPC progress", async () => {
    const env = runtime();
    let release!: () => void;
    env.interceptors.set("codex/request", true);
    env.state.chain = async (value) => {
      if (value.method === "hooks/list") await new Promise<void>((resolve) => { release = resolve; });
      return value;
    };
    const first: Frame = ["push", ["pipeline", -19, ["sendRequest"], ["hooks/list", {}]]];
    env.port.postMessage(first);
    await env.settle();
    env.port.postMessage(turnFrame());
    const later: Frame = ["push", ["pipeline", -19, ["markRead"], ["thread"]]];
    env.port.postMessage(later);
    assert.equal(env.sent.length, 0);
    release();
    await env.settle();
    assert.equal(env.sent[0], first);
    assert.equal(env.sent[1], later);
    assert.equal(env.sent.at(-1)![0], "resolve");
    assert.equal(env.calls.at(-1).method, "resumeThread");
  });

  it("rejects a failed refresh without sending the original turn", async () => {
    const env = runtime();
    env.state.refreshFailure = "window refresh failed";
    env.port.postMessage(turnFrame());
    await env.settle();
    assert.deepEqual(env.sent, [["reject", 7, ["error", "Error", "window refresh failed"]]]);
    assert.deepEqual(env.toasts, ["window refresh failed"]);
  });

  it("preserves push numbering when reading a new conversation config fails", async () => {
    const env = runtime();
    env.subscribe("local");
    env.state.startFailure = "catalog unreadable";
    env.port.postMessage(startFrame());
    const later: Frame = ["push", ["pipeline", -19, ["markRead"], ["thread"]]];
    env.port.postMessage(later);
    await env.settle();
    assert.deepEqual(env.sent[1], ["push", ["pipeline", -19, ["startConversation"],
      [...startFrame()[1][3], ["promise", -0.5]]]]);
    assert.deepEqual(env.sent[2], ["reject", -0.5, ["error", "Error", "catalog unreadable"]]);
    assert.equal(env.sent[3], later);
    assert.deepEqual(env.toasts, ["catalog unreadable"]);
  });

  it("rejects raw create calls with their original errors and preserves callback exports", async () => {
    for (const method of ["thread/start", "thread/fork"]) {
      const env = runtime();
      env.subscribe("local");
      env.state.startFailure = "catalog unreadable";
      const args = [method, { model: "supplier/model", callback: ["export", -8] }, { onSettled: ["export", -9] }];
      env.port.postMessage(["push", ["pipeline", -19, ["sendRequest"], args]]);
      await env.settle();
      assert.deepEqual(env.sent[1], ["push", ["pipeline", -19, ["sendRequest"], [...args, ["promise", -0.5]]]]);
      assert.deepEqual(env.sent[2], ["reject", -0.5, ["error", "Error", "catalog unreadable"]]);
    }
  });

  it("rejects the native creation-input callback without changing the original start push", async () => {
    const env = runtime();
    env.subscribe("local");
    env.state.startFailure = "catalog unreadable";
    const frame = startFrame();
    frame[1][3][1].readThreadCreationInputs = ["export", -8];
    env.port.postMessage(frame);
    await env.settle();
    assert.deepEqual(env.sent[1], frame);
    assert.equal(env.sent.length, 2);
    const callback: Frame = ["push", ["pipeline", -8, [], [{
      clientUserMessageId: "message", config: { original: true }, nested: ["export", -17],
    }, "full"]]];
    const originalArgs = [...callback[1][3]];
    env.port.dispatchEvent(new env.window.MessageEvent("message", { data: callback }));
    await env.settle();
    assert.deepEqual(env.received, [
      ["push", ["pipeline", -8, [], [...originalArgs, ["promise", -0.5]]]],
      ["reject", -0.5, ["error", "Error", "catalog unreadable"]],
    ]);
    env.port.postMessage(["release", -0.5, 1]);
    assert.equal(env.sent.length, 2);
    env.port.postMessage(["release", -17, 1]);
    assert.deepEqual(env.sent.at(-1), ["release", -17, 1]);
    const creation = env.window.__codexPlusRpcCreationFailures.get(env.port);
    assert.equal(creation.pending.size, 0);
    assert.equal(creation.callbacks.size, 0);
  });

  it("matches creation failures by port, callback export and client message", async () => {
    const env = runtime();
    env.subscribe("local");
    env.state.startFailure = "catalog unreadable";
    const frame = startFrame();
    frame[1][3][1].readThreadCreationInputs = ["export", -8];
    env.port.postMessage(frame);
    await env.settle();
    const unrelated: Frame = ["push", ["pipeline", -8, [], [{ clientUserMessageId: "other" }, "full"]]];
    env.port.dispatchEvent(new env.window.MessageEvent("message", { data: unrelated }));
    assert.deepEqual(unrelated[1][3], [{ clientUserMessageId: "other" }, "full"]);
    const otherPort = new env.window.MessagePort();
    const other: Frame = ["push", ["pipeline", -8, [], [{ clientUserMessageId: "message" }, "full"]]];
    otherPort.dispatchEvent(new env.window.MessageEvent("message", { data: other }));
    assert.deepEqual(other[1][3], [{ clientUserMessageId: "message" }, "full"]);
    env.api.install();
    env.port.dispatchEvent(new env.window.MessageEvent("message", { data: ["release", -8, 1] }));
    assert.equal(env.window.__codexPlusRpcCreationFailures.get(env.port).callbacks.size, 0);
    assert.equal(env.port.listeners.length, 0);
  });

  it("returns untouched data for events without port state and unrelated frames on a tracked port", async () => {
    const env = runtime();
    const data = { marker: "other event" };
    assert.equal(new env.window.MessageEvent("message", { data }).data, data);
    const other = new env.window.MessagePort();
    other.dispatchEvent(new env.window.MessageEvent("message", { data }));
    assert.equal(env.received.at(-1), data);
    env.subscribe("local");
    env.state.startFailure = "catalog unreadable";
    env.port.postMessage(startFrame());
    await env.settle();
    const frame: Frame = ["resolve", 19, { value: "original" }];
    env.port.dispatchEvent(new env.window.MessageEvent("message", { data: frame }));
    assert.equal(env.received.at(-1), frame);
  });

  it("consumes only its synthetic promise releases and continues numbering across failures", async () => {
    const env = runtime();
    env.subscribe("local");
    env.state.startFailure = "catalog unreadable";
    env.port.postMessage(startFrame());
    await env.settle();
    for (const id of [-5, -0.5, -0.5]) {
      env.port.dispatchEvent(new env.window.MessageEvent("message", { data: ["release", id, 1] }));
    }
    assert.deepEqual(env.received, [["release", -5, 1], ["resolve", 0.5, ["undefined"]], ["release", -0.5, 1]]);
    env.port.postMessage(startFrame());
    await env.settle();
    assert.deepEqual(env.sent.at(-1), ["reject", -1.5, ["error", "Error", "catalog unreadable"]]);
    assert.equal(env.port.listeners.length, 0);
  });

  it("preserves extension rejection reasons without preparing context", async () => {
    const env = runtime();
    env.subscribe("local");
    env.state.startFailure = "context must not run";
    env.listeners.set("agent/pre-step", new Set([{ scriptKey: "fixture",
      handler: () => ({ kind: "reject", reason: "blocked by extension" }) }]));
    env.port.postMessage(startFrame());
    await env.settle();
    assert.deepEqual(env.sent[2], ["reject", -0.5, ["error", "Error", "blocked by extension"]]);
    assert.deepEqual(env.toasts, ["blocked by extension"]);
    assert.deepEqual(env.calls, []);
  });

  it("upgrades an older interceptor without wrapping it", () => {
    const env = runtime();
    const original = env.window.__codexPlusRpcFrameInterceptor.original;
    env.window.__codexPlusRpcFrameInterceptor.version = 6;
    env.api.install();
    assert.equal(env.window.__codexPlusRpcFrameInterceptor.version, 7);
    assert.equal(env.window.__codexPlusRpcFrameInterceptor.original, original);
  });

  it("skips remote, unknown, steer-only and disabled context turns synchronously", () => {
    const env = runtime();
    for (const frame of [turnFrame("remote"), turnFrame(""), turnFrame()]) {
      if (frame[2].executionHostId === "local") delete frame[2].start;
      env.port.postMessage(frame);
      assert.equal(env.sent.at(-1), frame);
    }
    env.state.enabled = false;
    const local = turnFrame();
    env.port.postMessage(local);
    assert.equal(env.sent.at(-1), local);
    assert.equal(env.calls.length, 0);
  });

  it("keeps extension failures fail-open while still applying required context", async () => {
    const env = runtime();
    env.interceptors.set("codex/request", true);
    env.state.chain = async () => { throw new Error("extension broken"); };
    env.port.postMessage(turnFrame());
    await env.settle();
    assert.equal(env.sent.at(-1)![0], "resolve");
    assert.ok(env.failures.some((failure) => failure.includes("extension broken")));
    assert.equal(env.calls.at(-1).method, "resumeThread");
  });
});
