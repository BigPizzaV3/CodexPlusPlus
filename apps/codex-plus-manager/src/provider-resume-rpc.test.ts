import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const fragment = (name: string) => readFileSync(
  new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const rpc = fragment("91-extension-rpc.js");
const service = fragment("30-service-tier.js");
const provider = service.slice(0, service.indexOf("  function codexManagedSessionConnection("));
type Frame = any[];

function runtime() {
  const sent: Frame[] = [];
  const received: any[] = [];
  const calls: any[] = [];
  const diagnostics: any[] = [];
  const toasts: string[] = [];
  const failures: string[] = [];
  const settings: any = { providerSyncEnabled: true, relayProfilesEnabled: true, activeRelayId: "relay",
    relayProfiles: [{ id: "relay", relayMode: "pureApi", configContents: 'model_provider = "custom"' }] };
  const state = { status: "notLoaded", source: "openai", target: "custom", cached: null as any,
    failure: null as Error | null, bridgeFailure: "", retentionWarnings: [] as string[], discoveries: 0, disposals: 0,
    wait: null as Promise<void> | null, afterRead: () => {}, afterPrepare: () => {} };
  const interceptors = new Map();
  let chain = async (value: any) => value;
  const acknowledgements = new Map<string, () => void>();
  class PortMessageEvent extends MessageEvent<any> {
    get data() { return structuredClone(super.data); }
  }
  class MessagePort {
    postMessage(frame: Frame) {
      sent.push(frame);
      if (frame[0] === "push" && frame[1][2][0] === "sendRequest") {
        acknowledgements.get(frame[1][3][0])?.();
      }
    }
    dispatchEvent(event: MessageEvent) {
      Object.defineProperty(event, "currentTarget", { value: this });
      received.push(event.data);
      return true;
    }
  }
  const window = { MessagePort, MessageEvent: PortMessageEvent } as any;
  const port = new MessagePort();
  const postJson = async (route: string, payload: any) => {
    assert.equal(route, "/thread-provider/prepare");
    calls.push({ route, payload });
    if (!state.bridgeFailure) {
      state.source = payload.target_provider;
      state.afterPrepare();
    }
    return state.bridgeFailure ? { status: "failed", message: state.bridgeFailure }
      : { status: "ok", backup_retention_warnings: state.retentionWarnings };
  };
  const api = new Function("window", "codexPlusBackendSettings", "codexModelCatalog", "sendCodexPlusDiagnostic",
    "postJson", "codexAppServerPreparationTimeoutMs", `${provider}
      return { prepare: prepareCodexPureApiResume, align: prepareCodexThreadProviderResume,
        preparations: codexThreadProviderPreparations };`)(
    window, settings, {}, (event: string, detail: any) => diagnostics.push({ event, detail }), postJson, 2000);
  const subscribe = (hostId = "local", receiver = -19, target = port) =>
    target.postMessage(["push", ["pipeline", receiver, ["subscribe"], [{
      type: "notification", key: { hostId }, methods: [["thread/tokenUsage/updated"]], listener: ["export", -3],
    }]]]);
  const client = {
    async sendRequest(method: string, params: any, options: any) {
      calls.push({ method, params, options });
      await new Promise<void>((resolve) => {
        acknowledgements.set(method, resolve);
        port.postMessage(["push", ["pipeline", -19, ["sendRequest"], [method, params, options]]]);
      });
      await state.wait;
      if (state.failure) throw state.failure;
      state.afterRead();
      if (method === "thread/read") {
        return { thread: { id: params.threadId, cwd: "C:/fixture",
          status: { type: state.status }, modelProvider: state.source } };
      }
      if (method === "thread/unsubscribe") {
        state.status = "notLoaded";
        return { status: "ok" };
      }
      if (method === "thread/resume") {
        state.status = "idle";
        return { modelProvider: params.modelProvider ?? state.source,
          thread: { id: params.threadId, modelProvider: state.source } };
      }
      assert.equal(method, "config/read");
      return { config: { model_provider: state.target } };
    },
    async getConversation(conversationId: string) {
      calls.push({ method: "getConversation", conversationId });
      return state.cached;
    },
    subscribe(params: any) {
      assert.equal(params.key.hostId, "local");
      subscribe(params.key.hostId);
      return Object.assign(Promise.resolve({ [Symbol.dispose]() { state.disposals++; } }),
        { [Symbol.dispose]() { state.disposals++; } });
    },
  };
  const discover = async (hostId: string) => {
    assert.equal(hostId, "local");
    state.discoveries++;
    return client;
  };
  const install = new Function("window", "codexPlusBackendSettings", "prepareCodexThreadProviderResume",
    "codexThreadProviderPreparations",
    "codexPlusAppServerClient", "codexPlusExtensionListeners", "codexPlusExtensionInterceptors",
    "codexPlusRunWaterfall", "codexPlusMarkExtensionFailure", "showToast",
    `${rpc}\nreturn installCodexPlusRpcFrameInterceptor;`)(
    window, settings, api.align, api.preparations, discover, new Map(), interceptors,
    (_name: string, value: any) => chain(value),
    (_key: string, message: string) => failures.push(message), (message: string) => toasts.push(message));
  const settle = async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
  };
  const release = (id = -0.25) => port.dispatchEvent(
    new PortMessageEvent("message", { data: ["release", id, 1] }));
  return { api, settings, state, client, discover, window, port, sent, received, calls, toasts, failures, diagnostics,
    install, subscribe, settle, release, interceptors, setChain: (next: typeof chain) => { chain = next; } };
}

function resume(method: string, params: any, callbacks: any = {}, receiver = -19): Frame {
  return ["push", ["pipeline", receiver, [method],
    method === "sendRequest" ? ["thread/resume", params, callbacks] : [params, callbacks]]];
}

describe("provider resume RPC gate without context helpers", () => {
  it("does not discover a native client for direct connections when provider sync is off", async () => {
    for (const method of ["resumeConversation", "resumeThread", "sendRequest"]) {
      const env = runtime();
      env.subscribe();
      env.settings.providerSyncEnabled = false;
      const frame = resume(method, { threadId: "thread", conversationId: "thread" });
      env.port.postMessage(frame);
      await env.settle();
      assert.equal(env.state.discoveries, 0);
      assert.deepEqual(env.calls, []);
      assert.equal(env.sent.at(-1), frame);
    }
  });

  it("reports backup retention warnings without rejecting the committed preparation", async () => {
    const env = runtime();
    env.state.retentionWarnings = ["backup removal failed"];
    env.subscribe();
    env.port.postMessage(resume("resumeThread", { threadId: "thread" }));
    await env.settle();
    assert.equal(env.sent.at(-1)?.[0], "resolve");
    assert.deepEqual(env.diagnostics, [{
      event: "thread_provider_backup_retention_warning",
      detail: { warnings: ["backup removal failed"] },
    }]);
    env.release();
  });

  it("removes the internal resume marker even when the receiver is unknown or conflicting", async () => {
    for (const mode of ["unknown", "conflicting"]) {
      const env = runtime();
      env.subscribe();
      if (mode === "conflicting") env.subscribe("ssh");
      env.api.preparations.set("thread", { resuming: 42 });
      const frame = resume("sendRequest", { threadId: "thread" },
        { __codexPlusProviderResume: 42, timeoutMs: 2000 }, mode === "unknown" ? -20 : -19);
      env.port.postMessage(frame);
      await env.settle();
      assert.deepEqual(env.sent.at(-1)?.[1][3][2], { timeoutMs: 2000 });
      assert.equal(env.calls.length, 0);
    }
  });

  it("passes disabled, remote, unknown and conflicting hosts through synchronously", () => {
    for (const mode of ["disabled", "remote", "unknown", "other-port", "conflicting", "envelope"]) {
      const env = runtime();
      env.subscribe(mode === "remote" ? "ssh" : "local");
      if (mode === "disabled") env.settings.providerSyncEnabled = false;
      if (mode === "conflicting") env.subscribe("ssh");
      for (const method of ["resumeConversation", "resumeThread", "sendRequest"]) {
        const params = { threadId: "thread", conversationId: "thread", model: null,
          ...(mode === "envelope" ? { host_id: "ssh" } : {}) };
        const frame = resume(method, params, {}, mode === "unknown" ? -20 : -19);
        const port = mode === "other-port" ? new env.window.MessagePort() : env.port;
        port.postMessage(frame);
        assert.equal(env.sent.at(-1), frame, `${mode}: ${method}`);
      }
      assert.equal(env.state.discoveries, 0);
      assert.deepEqual(env.calls, []);
      assert.equal(env.window.__codexPlusRpcResumeGates.has(env.port), false);
    }
  });

  it("preserves nullable, explicit and preserveServerConfiguration params plus all callback exports", async () => {
    for (const method of ["resumeConversation", "resumeThread", "sendRequest"]) {
      for (const choice of [{ model: null, modelProvider: null, config: null, reasoningEffort: null },
        { model: "saved-outside-catalog", config: { model_reasoning_effort: "high" } },
        { preserveServerConfiguration: true }]) {
        const env = runtime();
        env.subscribe();
        const params = { conversationId: "thread", threadId: "thread",
          workspaceRoots: [["C:/fixture"]], ...choice };
        const callbacks = { readResumeInputs: ["export", -7], hydrationObserver: ["export", -8],
          onSettled: ["export", -9] };
        const frame = resume(method, params, callbacks);
        const before = structuredClone(frame);
        env.port.postMessage(frame);
        const later: Frame = ["push", ["pipeline", -19, ["markRead"], ["thread"]]];
        env.port.postMessage(later);
        await env.settle();
        const index = method === "sendRequest" ? 1 : 0;
        assert.deepEqual(env.sent[1][1][3], frame[1][3].map((arg: any, i: number) =>
          i === index ? ["promise", -0.25] : arg));
        assert.equal(env.sent[2], later);
        assert.deepEqual(env.sent.at(-1), ["resolve", -0.25, params]);
        assert.deepEqual(frame, before);
        assert.deepEqual(env.calls.slice(0, 4).map((call) => call.method || call.route),
          ["thread/read", "getConversation", "config/read", "/thread-provider/prepare"]);
        assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 1);
        env.release();
        assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 0);
        assert.deepEqual(env.failures, []);
      }
    }
  });

  it("keeps embedded exports in the original push and consumes only its own releases", async () => {
    for (const method of ["resumeConversation", "resumeThread", "sendRequest"]) {
      const env = runtime();
      env.subscribe();
      const frame = resume(method, { threadId: "thread", conversationId: "thread",
        trace: ["export", -12], model: null }, { readResumeInputs: ["export", -7] });
      env.port.postMessage(frame);
      await env.settle();
      assert.deepEqual(env.sent[1][1][3], [...frame[1][3], ["promise", -0.25]]);
      assert.deepEqual(env.sent.at(-1), ["resolve", -0.25, ["undefined"]]);
      env.release(-12);
      env.release(-0.5);
      env.release();
      env.release();
      assert.deepEqual(env.received, [
        ["release", -12, 1], ["release", -0.5, 1], ["resolve", 0.5, ["undefined"]], ["release", -0.25, 1],
      ]);
      assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 0);
    }
  });

  it("opens only an exactly matching cached mismatch after successful alignment", async () => {
    for (const error of [
      { type: "modelProviderMismatch", modelProvider: "openai", currentModelProvider: "custom" },
      { type: "modelProviderMismatch", modelProvider: "other", currentModelProvider: "custom" },
      { type: "modelProviderMismatch", modelProvider: "openai", currentModelProvider: "other" },
      { type: "modelProviderNotConfigured", modelProvider: "openai" },
      { type: "modelProviderPolicyConflict", modelProvider: "openai", requiredModelProvider: "custom" }, null,
    ]) {
      const env = runtime();
      env.subscribe();
      env.state.cached = { providerBlock: error && { error } };
      const params = { conversationId: "thread", intent: "view" };
      env.port.postMessage(resume("resumeConversation", params));
      await env.settle();
      assert.equal(env.sent.at(-1)![2].intent, error?.type === "modelProviderMismatch"
        && error.modelProvider === "openai" && error.currentModelProvider === "custom" ? "open" : "view");
      assert.equal(params.intent, "view");
    }
    const env = runtime();
    env.subscribe();
    env.state.source = "custom";
    env.state.cached = { providerBlock: { error: {
      type: "modelProviderMismatch", modelProvider: "openai", currentModelProvider: "custom",
    } } };
    env.port.postMessage(resume("resumeConversation", { conversationId: "thread", intent: "view" }));
    await env.settle();
    assert.equal(env.sent.at(-1)![2].intent, "view");
    assert.equal(env.calls.some((call) => call.route), false);
  });

  it("falls back on read errors, preserves write errors and permits retry after upgrading", async () => {
    const env = runtime();
    env.subscribe();
    const original = env.window.__codexPlusRpcFrameInterceptor.original;
    const frame = resume("resumeThread", { threadId: "thread", model: null });
    env.state.failure = new RangeError("read failed");
    env.port.postMessage(frame);
    await env.settle();
    assert.deepEqual(env.sent.at(-1), ["resolve", -0.25, frame[1][3][0]]);
    env.release();
    env.state.failure = null;
    env.state.bridgeFailure = "CAS conflict";
    env.port.postMessage(frame);
    await env.settle();
    assert.deepEqual(env.sent.at(-1), ["reject", -1.25, ["error", "Error", "CAS conflict"]]);
    env.release(-1.25);
    env.state.bridgeFailure = "";
    env.window.__codexPlusRpcFrameInterceptor.version = 11;
    env.install();
    assert.equal(env.window.__codexPlusRpcFrameInterceptor.version, 19);
    assert.equal(env.window.__codexPlusRpcFrameInterceptor.original, original);
    env.port.postMessage(frame);
    await env.settle();
    assert.deepEqual(env.sent.at(-1), ["resolve", -2.25, frame[1][3][0]]);
    env.release(-2.25);
    assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 0);
  });

  it("clears disconnected gates and suppresses late settlement without holding the port queue", async () => {
    const env = runtime();
    env.subscribe();
    let finish!: () => void;
    env.state.wait = new Promise<void>((resolve) => { finish = resolve; });
    env.port.postMessage(resume("resumeThread", { threadId: "thread" }));
    await env.settle();
    const later: Frame = ["push", ["pipeline", -19, ["markRead"], ["thread"]]];
    env.port.postMessage(later);
    assert.equal(env.sent.at(-1), later);
    env.port.dispatchEvent(new env.window.MessageEvent("message", { data: null }));
    assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 0);
    finish();
    await env.settle();
    assert.equal(env.sent.some((frame) => ["resolve", "reject"].includes(frame[0])), false);
  });

  it("waits for an existing extension queue before sending same-port preflight", async () => {
    const env = runtime();
    env.subscribe();
    let release!: () => void;
    env.interceptors.set("codex/request", true);
    env.setChain(async (value) => {
      if (value.method === "hooks/list") await new Promise<void>((resolve) => { release = resolve; });
      return value;
    });
    const first: Frame = ["push", ["pipeline", -19, ["sendRequest"], ["hooks/list", {}]]];
    env.port.postMessage(first);
    await env.settle();
    env.port.postMessage(resume("resumeThread", { threadId: "thread" }));
    assert.equal(env.calls.length, 0);
    release();
    await env.settle();
    assert.equal(env.sent[1], first);
    assert.deepEqual(env.sent[2][1][3][0], ["promise", -0.25]);
    assert.equal(env.calls[0].method, "thread/read");
    assert.equal(env.sent.at(-1)![0], "resolve");
  });
});

describe("provider preparation", () => {
  it("switches both unloaded and idle providers in either direction without touching active turns", async () => {
    for (const [source, target] of [["openai", "custom"], ["custom", "openai"]]) {
      for (const status of ["notLoaded", "idle", "active"]) {
        const env = runtime();
        env.subscribe();
        Object.assign(env.state, { source, target, status });
        const params = { threadId: "thread", model: null, config: { model_reasoning_effort: null } };
        const before = structuredClone(params);
        const result = await env.api.align(params, "local", env.client.sendRequest);
        assert.deepEqual(params, before);
        assert.equal(env.calls.some((call) => call.route), status !== "active");
        assert.deepEqual(result, status !== "active" ? { sourceProvider: source, targetProvider: target } : null);
        assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe"), status === "idle");
      }
    }
  });

  it("uses the host lifecycle and retries after disk alignment when unsubscribe fails", async () => {
    const env = runtime();
    env.subscribe();
    env.state.status = "idle";
    env.state.cached = { modelProvider: "openai", resumeState: "resumed", threadRuntimeStatus: { type: "idle" } };
    let failed = true;
    const host = {
      getConversation: env.client.getConversation,
      async unsubscribeInactiveConversation(threadId: string) {
        env.calls.push({ method: "unsubscribeInactiveConversation", threadId });
        if (!failed) {
          env.state.cached.resumeState = "needs_resume";
          env.state.status = "notLoaded";
        }
      },
    };
    const params = { conversationId: "thread", model: null, reasoningEffort: null };
    await assert.rejects(env.api.align(params, "local", env.client.sendRequest, host), /尚未卸载/);
    assert.equal(env.state.source, "custom");
    failed = false;
    const result = await env.api.align(params, "local", env.client.sendRequest, host);
    assert.deepEqual(result, { sourceProvider: "openai", targetProvider: "custom" });
    assert.equal(env.calls.filter((call) => call.route).length, 1);
    assert.equal(env.calls.filter((call) => call.method === "unsubscribeInactiveConversation").length, 2);
    assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe"), false);
    assert.deepEqual(params, { conversationId: "thread", model: null, reasoningEffort: null });
  });

  it("confirms the actual connection once despite stale host labels and switches back after index sync", async () => {
    const env = runtime();
    env.subscribe();
    env.state.status = "idle";
    env.state.source = "custom";
    env.state.target = "openai";
    env.state.cached = { modelProvider: "custom", resumeState: "resumed" };
    const host = {
      getConversation: env.client.getConversation,
      async unsubscribeInactiveConversation() {
        env.calls.push({ method: "unsubscribeInactiveConversation" });
        env.state.cached.resumeState = "needs_resume";
        env.state.status = "notLoaded";
      },
    };
    const send = async (method: string, params: any, options: any) => {
      if (method === "thread/resume") {
        assert.equal(await env.api.align(params, "local", send, host, options?.__codexPlusProviderResume), null,
          "The internal explicit resume must not await its own preparation lock.");
      }
      return env.client.sendRequest(method, params, options);
    };
    const params = { conversationId: "thread", model: null, reasoningEffort: null };
    assert.deepEqual(await env.api.align(params, "local", send, host), {
      sourceProvider: "custom", targetProvider: "openai",
    });
    // Ordinary native hydration retains this old label even after the real connection changes.
    env.state.cached.resumeState = "resumed";
    assert.equal(await env.api.align(params, "local", send, host), null);
    assert.equal(await env.api.align(params, "local", send, host), null);
    assert.equal(env.calls.filter(call => call.method === "unsubscribeInactiveConversation").length, 1);
    // The same RPC client can survive a native unload; its earlier connection proof then expires.
    env.state.status = "notLoaded";
    assert.deepEqual(await env.api.align(params, "local", send, host), {
      sourceProvider: "custom", targetProvider: "openai",
    });
    assert.equal(env.calls.filter(call => call.method === "unsubscribeInactiveConversation").length, 2);
    env.state.cached.resumeState = "resumed";
    // Startup synchronization can align the index before the next reverse transition.
    env.state.source = "custom";
    env.state.target = "custom";
    assert.deepEqual(await env.api.align(params, "local", send, host), {
      sourceProvider: "openai", targetProvider: "custom",
    });
    assert.equal(env.calls.filter(call => call.method === "unsubscribeInactiveConversation").length, 3);
    assert.equal(env.calls.filter(call => call.route).length, 1);
    assert.deepEqual(env.calls.filter(call => call.method === "thread/resume").map(call => call.params), [
      { threadId: "thread", modelProvider: "openai", model: null, config: null },
      { threadId: "thread", modelProvider: "openai", model: null, config: null },
      { threadId: "thread", modelProvider: "custom", model: null, config: null },
    ]);
  });

  it("keeps internal resume middleware without reentering the held preparation lock", async () => {
    const env = runtime();
    env.subscribe();
    env.state.status = "idle";
    env.state.cached = { modelProvider: "openai", resumeState: "resumed" };
    env.interceptors.set("codex/request", true);
    env.setChain(async (value) => value.method === "thread/resume"
      ? { ...value, params: { ...value.params, config: { model_reasoning_effort: "high" } } }
      : value);
    const host = {
      getConversation: env.client.getConversation,
      async unsubscribeInactiveConversation() {
        env.state.cached.resumeState = "needs_resume";
        env.state.status = "notLoaded";
      },
    };
    assert.deepEqual(await env.api.align({ threadId: "thread" }, "local", env.client.sendRequest, host), {
      sourceProvider: "openai", targetProvider: "custom",
    });
    const frames = env.sent.filter(frame => frame[0] === "push" && frame[1][2][0] === "sendRequest"
      && frame[1][3][0] === "thread/resume");
    assert.equal(frames.length, 1);
    assert.deepEqual(frames[0][1][3][1], {
      threadId: "thread", modelProvider: "custom", model: null, config: { model_reasoning_effort: "high" },
    });
    assert.equal(env.window.__codexPlusRpcResumeGates.has(env.port), false);
    assert.equal(env.calls.filter(call => call.route).length, 1);
  });

  it("does not record failed or wrong-provider resumes as successfully prepared", async () => {
    for (const failure of ["reject", "wrong-provider", "configuration"]) {
      const env = runtime();
      env.subscribe();
      env.state.status = "idle";
      env.state.cached = { modelProvider: "openai", resumeState: "resumed" };
      const host = {
        getConversation: env.client.getConversation,
        async unsubscribeInactiveConversation() {
          env.state.cached.resumeState = "needs_resume";
          env.state.status = "notLoaded";
        },
      };
      let failed = true;
      const send = async (method: string, params: any, options: any) => {
        if (failed && method === "thread/resume") {
          if (failure === "reject") throw new Error("LOCAL_RESUME_FAILED");
          if (failure === "wrong-provider") return { modelProvider: "openai" };
          env.settings.relayProfiles[0].configContents = 'model_provider = "changed"';
        }
        return env.client.sendRequest(method, params, options);
      };
      await assert.rejects(env.api.align({ threadId: "thread" }, "local", send, host),
        failure === "reject" ? /LOCAL_RESUME_FAILED/
          : failure === "wrong-provider" ? /目标供应商/ : /配置已变化/);
      failed = false;
      assert.deepEqual(await env.api.align({ threadId: "thread" }, "local", send, host), {
        sourceProvider: "openai", targetProvider: "custom",
      });
      assert.equal(env.calls.filter(call => call.route).length, 1);
      assert.equal(env.calls.filter(call => call.method === "thread/resume").length,
        failure === "configuration" ? 2 : 0);
    }
  });

  it("does not unload a thread which becomes active before or after the canonical write", async () => {
    for (const stage of ["before", "after"]) {
      const env = runtime();
      env.subscribe();
      env.state.status = "idle";
      if (stage === "before") env.state.afterRead = () => { env.state.status = "active"; };
      else env.state.afterPrepare = () => { env.state.status = "active"; };
      assert.equal(await env.api.align({ threadId: "thread" }, "local", env.client.sendRequest), null);
      assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe"), false);
      assert.equal(env.calls.some((call) => call.route), stage === "after");
      env.state.afterRead = () => {};
      env.state.afterPrepare = () => {};
      env.state.status = "idle";
      assert.deepEqual(await env.api.align({ threadId: "thread" }, "local", env.client.sendRequest), {
        sourceProvider: "openai", targetProvider: "custom",
      });
      assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe"), true);
    }
  });

  it("serializes same-thread preparations and lets native resume keep the legacy turn choices", async () => {
    const env = runtime();
    env.subscribe();
    env.state.status = "idle";
    const params = { threadId: "thread", model: "chosen", effort: "high",
      input: [{ type: "text", text: "next" }] };
    const before = structuredClone(params);
    const results = await Promise.all([
      env.api.prepare("turn/start", params, "local", false, env.client.sendRequest),
      env.api.prepare("turn/start", params, "local", false, env.client.sendRequest),
    ]);
    assert.equal(env.calls.filter((call) => call.route).length, 1);
    assert.equal(env.calls.filter((call) => call.method === "thread/unsubscribe").length, 1);
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
    assert.deepEqual(env.calls.find((call) => call.method === "thread/resume").params, {
      threadId: "thread", modelProvider: "custom", model: "chosen", config: { model_reasoning_effort: "high" },
    });
    assert(results.every((result) => result === params));
    assert.deepEqual(params, before);
  });

  it("rejects a changed provider configuration before the bridge write", async () => {
    const env = runtime();
    env.subscribe();
    env.state.afterRead = () => { env.settings.relayProfiles[0].configContents = 'model_provider = "changed"'; };
    await assert.rejects(env.api.align({ threadId: "thread" }, "local", env.client.sendRequest), /配置已变化/);
    assert.equal(env.calls.some((call) => call.route), false);
  });

  it("keeps local host envelopes and explicit provider overrides intact", async () => {
    const env = runtime();
    env.subscribe();
    for (const choice of [{ modelProvider: "chosen" }, { model_provider: "chosen" },
      { config: { model_provider: "chosen" } }]) {
      const params = { threadId: "thread", model: null, ...choice };
      const before = structuredClone(params);
      await env.api.prepare("send-cli-request-for-host",
        { hostId: "local", method: "thread/resume", params }, "local", false,
        (method: string, envelope: any, options: any) => {
          assert.equal(method, "send-cli-request-for-host");
          assert.equal(envelope.hostId, "local");
          return env.client.sendRequest(envelope.method, envelope.params, options);
        });
      assert.equal(env.calls.filter((call) => call.route).at(-1).payload.target_provider, "chosen");
      assert.deepEqual(params, before);
    }
  });

  it("keeps managed aliases out of index synchronization while unloading an old managed connection", async () => {
    const target = runtime();
    target.subscribe();
    target.state.source = "custom";
    target.state.target = "codex_plus_managed";
    target.state.status = "idle";
    assert.equal(await target.api.align({ threadId: "thread" }, "local", target.client.sendRequest), null);
    assert.equal(target.calls.some((call) => call.route), false);
    assert.equal(target.calls.some((call) => call.method === "thread/unsubscribe"), false);

    const source = runtime();
    source.subscribe();
    source.state.source = "codex_plus_managed";
    source.state.target = "custom";
    source.state.status = "idle";
    const prepared = await source.api.align({ threadId: "thread" }, "local", source.client.sendRequest);
    assert.equal(prepared.targetProvider, "custom");
    assert.equal(source.calls.some((call) => call.route), false);
    assert.equal(source.calls.some((call) => call.method === "thread/unsubscribe"), true);
  });

  it("binds the local receiver from settings without a context observer and immediately disposes the subscription", async () => {
    for (const enabled of [false, true]) {
      const env = runtime();
      const load = new Function("window", "codexPlusBackendSettings", "codexPlusBackendSettingsSeq",
        "postJson", "codexPlusAppServerClient", "sendCodexPlusDiagnostic",
        `let codexPlusBackendSettingsLoaded = false;
         ${service.slice(service.indexOf("  async function loadBackendSettingsState("))}
         return loadBackendSettingsState;`)(
        env.window, {}, 1, async () => ({ providerSyncEnabled: enabled }), env.discover, () => {});
      assert.equal(await load(), true);
      await env.settle();
      assert.equal(env.state.discoveries, enabled ? 1 : 0);
      assert.equal(env.state.disposals, enabled ? 2 : 0);
      assert.equal(env.window.__codexPlusRpcHostReceivers.get(env.port)?.get(-19), enabled ? "local" : undefined);
      assert.equal(env.window.__codexPlusThreadContextObserver, undefined);
    }
  });

  it("discovers the numeric-chunk manager from cached exports without executing module factories", async () => {
    const source = fragment("91-extension-runtime.js");
    const body = source.slice(source.indexOf("  const codexPlusAppServerModuleHints"),
      source.indexOf("  /** 把 append")).replaceAll("await import(url)", "await importModule(url)");
    const atom = {};
    const manager = { forHost: (hostId: string) => ({ hostId }) };
    const discover = new Function("codexPlusFindAppScope", "document", "importModule",
      "loadOptionalCodexAppModule", "codexAppAssetCandidateUrls", "sendCodexPlusDiagnostic",
      `let codexPlusManagerAtom = null; ${body}\nreturn codexPlusAppServerClient;`)(
      () => ({ get: (candidate: unknown) => candidate === atom ? manager : null }),
      { scripts: [{ src: "https://fixture/assets/123.abc123.js" }] },
      async () => ({ __webpack_require__: { m: { fixture: function () {
        throw new Error("AppServerManager RPC is unavailable for hostId: .forHost(");
      } }, c: { fixture: { exports: { atom } } } } }),
      () => { assert.fail("numeric chunks must resolve before the legacy fallback"); },
      () => [], () => {});
    assert.deepEqual(await discover("local"), { hostId: "local" });
  });
});
