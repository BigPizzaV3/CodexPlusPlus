import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/91-extension-rpc.js", import.meta.url), "utf8");
type Frame = any[];

function runtime() {
  const sent: Frame[] = [];
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
  class MessagePort {
    postMessage(frame: Frame) {
      sent.push(frame);
      if (frame[0] === "push") acknowledgements.get(frame[1][2][0])?.();
    }
  }
  const window = { MessagePort } as any;
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
    `${source}\nreturn { decode: codexPlusRpcDecode, install: installCodexPlusRpcFrameInterceptor };`);
  const api = create(window, new Map(), interceptors, (_name: string, value: any) => state.chain(value),
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
    });
  const settle = async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
  };
  const subscribe = (hostId: string, receiver = -19, target = port) =>
    target.postMessage(["push", ["pipeline", receiver, ["subscribe"], [{
      type: "notification", key: { hostId }, methods: [["turn/completed"]], listener: ["export", -3],
    }]]]);
  return { api, window, port, sent, calls, state, toasts, failures, interceptors, subscribe, settle };
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
    assert.deepEqual(env.sent[1], ["push", ["pipeline", -19, ["__codexPlusContextRefreshFailed"], []]]);
    assert.equal(env.sent[2], later);
    assert.deepEqual(env.toasts, ["catalog unreadable"]);
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
