import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const fragment = source.slice(source.indexOf("  const codexGatewayPreparations"),
  source.indexOf("  function codexManagedSessionConnection("));
const readFragment = source.slice(source.indexOf("  async function readCodexAppServerPreparation("),
  source.indexOf("  async function loadBackendSettingsState("));

function runtime(firstRead?: Promise<any>, active = true) {
  const calls: { method: string; params: any }[] = [];
  const state = {
    active, generation: "one", status: "idle", provider: "custom", target: "custom",
    turns: 1, ephemeral: false, failResume: false, changeDuringResume: false, changeDuringStart: false,
    readFailure: "", preflightFailure: "", pendingDuringRead: false, failUnsubscribe: false, missingMode: false,
    waitingRead: null as Promise<any> | null, failAfterResume: false,
  };
  const window = {} as any;
  const notices: string[] = [];
  const diagnostics: string[] = [];
  const profile = () => state.active ? { id: "codex-plus-gateway" } : { id: "direct" };
  let reads = 0;
  const bridge = async () => {
    if (++reads === 1 && firstRead) return firstRead;
    if (state.readFailure) throw new Error(state.readFailure);
    if (state.waitingRead) return state.waitingRead;
    return { status: "ok", active: state.active, generation: state.generation };
  };
  const api = new Function("window", "codexRemoteSessionActiveProfile",
    "postJson", "codexAppServerPreparationTimeoutMs", "sendCodexPlusDiagnostic", "showToast",
    `${readFragment}\n${fragment}\nreturn { reconnect: prepareCodexGatewayReconnect, baseline: initializeCodexGatewayBaseline,
      noteStart: noteCodexGatewayThreadStart };`)(
    window, profile, bridge, 30, (name: string) => diagnostics.push(name),
    (message: string) => notices.push(message));
  const baseline = api.baseline(active);
  const client = {
    async getConversation() {
      if (state.preflightFailure === "cache") throw new Error("cache unavailable");
      return { resumeState: state.status === "notLoaded" ? "needs_resume" : "resumed",
        threadRuntimeStatus: { type: state.status } };
    },
    async unsubscribeInactiveConversation() {
      calls.push({ method: "client/unsubscribe", params: {} });
    },
    subscribe() {
      if (state.preflightFailure === "subscribe") throw new Error("subscribe unavailable");
      const promise = Promise.resolve({ [Symbol.dispose]() {} }) as Promise<any> & { [Symbol.dispose]: () => void };
      promise[Symbol.dispose] = () => {};
      return promise;
    },
  };
  const send = async (method: string, params: any) => {
    calls.push({ method, params });
    if (method === "thread/read") {
      if (state.pendingDuringRead) window.__codexPlusContextRefreshPending = new Set(["thread"]);
      if (state.preflightFailure === "thread/read") throw new Error("thread read unavailable");
      return { thread: {
        status: { type: state.status }, modelProvider: state.provider, model: "saved", ephemeral: state.ephemeral,
      } };
    }
    if (method === "thread/start") {
      if (state.changeDuringStart) state.generation = "three";
      return { thread: { id: "thread" }, modelProvider: state.provider };
    }
    if (method === "turn/start") return { accepted: true };
    if (method === "config/read") {
      if (state.preflightFailure === "config/read") throw new Error("config read unavailable");
      return { config: { model_provider: state.target } };
    }
    if (method === "thread/turns/list") {
      if (state.preflightFailure === "thread/turns/list") throw new Error("turns unavailable");
      return { data: Array.from({ length: state.turns }) };
    }
    if (method === "thread/unsubscribe") {
      state.status = "notLoaded";
      if (state.failUnsubscribe) throw new Error("unsubscribe failed");
      return {};
    }
    if (method === "thread/resume") {
      if (state.failResume) throw new Error("resume failed");
      state.status = "idle";
      if (state.changeDuringResume) state.generation = "two";
      if (state.failAfterResume) state.readFailure = "post-resume read failed";
      return { modelProvider: state.target,
        ...(state.missingMode ? {} : { collaborationMode: { mode: "default", settings: {} } }) };
    }
    if (method === "thread/settings/update") return {};
    throw new Error(`Unexpected method: ${method}`);
  };
  const run = (params: any = { threadId: "thread", model: "chosen", effort: "high" }) =>
    api.reconnect("turn/start", params, "local", send, client);
  const start = async () => {
    const generation = await api.reconnect("thread/start", {}, "local", send, client);
    const result = await send("thread/start", {});
    api.noteStart("thread/start", {}, result, "local", generation);
  };
  return { calls, state, window, run, start, send, baseline, notices, diagnostics };
}

describe("gateway reconnect on the next turn", () => {
  it("establishes the page baseline without reloading unchanged existing sessions", async () => {
    const env = runtime();
    await env.run();
    await env.run({ threadId: "another", model: "chosen", effort: "high" });
    assert.equal(env.calls.length, 0);
  });

  it("does not lose a change while the page baseline is still being read", async () => {
    let finish!: (value: any) => void;
    const env = runtime(new Promise((resolve) => { finish = resolve; }));
    env.state.generation = "two";
    const sent = env.run();
    finish({ status: "ok", active: true, generation: "one" });
    await sent;
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("detects a gateway enabled after the page started with a direct connection", async () => {
    const env = runtime(undefined, false);
    await env.baseline;
    env.state.active = true;
    env.state.generation = "two";
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("allows the turn through after a failed generation read and retries later", async () => {
    const env = runtime();
    await env.baseline;
    env.state.generation = "two";
    env.state.readFailure = "bridge unavailable";
    await env.run();
    assert.equal(env.calls.length, 0);
    env.state.readFailure = "";
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("allows a timed-out read through and ignores its late result", async () => {
    const env = runtime();
    await env.baseline;
    env.state.generation = "two";
    let finish!: (value: any) => void;
    env.state.waitingRead = new Promise((resolve) => { finish = resolve; });
    await env.run();
    assert.equal(env.calls.length, 0);
    finish({ status: "ok", active: true, generation: "one" });
    env.state.waitingRead = null;
    await Promise.resolve();
    assert.equal(env.calls.length, 0);
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("allows a send when the post-resume generation check fails and retries later", async () => {
    const env = runtime();
    env.state.generation = "two";
    env.state.failAfterResume = true;
    await env.run();
    env.state.failAfterResume = false;
    env.state.readFailure = "";
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 2);
  });

  it("establishes a baseline after the startup read fails", async () => {
    const env = runtime(Promise.reject(new Error("startup read failed")));
    await env.baseline;
    await env.run();
    assert.equal(env.calls.length, 0);
    env.state.generation = "two";
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("reloads a persisted session once per connection generation and preserves its choices", async () => {
    const env = runtime();
    env.state.generation = "two";
    await env.run();
    await env.run();
    let resumes = env.calls.filter((call) => call.method === "thread/resume");
    assert.equal(resumes.length, 1);
    assert.equal(env.calls.some((call) => call.method === "client/unsubscribe"), false);
    assert.deepEqual(resumes[0].params, { threadId: "thread", modelProvider: "custom",
      model: "chosen", config: { model_reasoning_effort: "high" } });
    env.state.generation = "three";
    await env.run();
    resumes = env.calls.filter((call) => call.method === "thread/resume");
    assert.equal(resumes.length, 2);
  });

  it("preserves an explicitly null reasoning effort", async () => {
    const env = runtime();
    env.state.generation = "two";
    await env.run({ threadId: "thread",
      collaborationMode: { mode: "default", settings: { model: "chosen", reasoning_effort: null } } });
    const update = env.calls.find((call) => call.method === "thread/settings/update");
    assert.equal(update?.params.collaborationMode.settings.reasoning_effort, null);
  });

  it("allows a null effort when neither request nor resume has a collaboration mode", async () => {
    const env = runtime();
    env.state.generation = "two";
    env.state.missingMode = true;
    await env.run({ threadId: "thread", model: "chosen" });
    assert.equal(env.calls.some((call) => call.method === "thread/settings/update"), false);
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("keeps active turns and defers attached side-chat recovery until after the first native send", async () => {
    const env = runtime();
    env.state.generation = "two";
    env.state.status = "active";
    await env.run();
    assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe"), false);
    assert.match(env.notices[0], /旧连接/);
    env.state.status = "idle";
    env.state.turns = 0;
    await env.run();
    await env.send("turn/start", { threadId: "thread" });
    assert.ok(env.calls.some((call) => call.method === "thread/turns/list"));
    assert.equal(env.calls.some((call) => ["thread/unsubscribe", "thread/resume"].includes(call.method)), false);
    env.state.turns = 1;
    await env.run();
    await env.send("turn/start", { threadId: "thread" });
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
    assert.deepEqual(env.calls.filter((call) => call.method === "turn/start").map((call) => call.params.threadId),
      ["thread", "thread"]);
  });

  it("does not reload a thread created after the gateway generation changed", async () => {
    const env = runtime();
    await env.baseline;
    env.state.generation = "two";
    env.state.turns = 0;
    await env.start();
    await env.run();
    await env.send("turn/start", { threadId: "thread" });
    env.state.turns = 1;
    await env.run();
    await env.send("turn/start", { threadId: "thread" });
    assert.equal(env.calls.filter((call) => call.method === "turn/start").length, 2);
    assert.equal(env.calls.some((call) => ["thread/read", "thread/turns/list",
      "thread/unsubscribe", "thread/resume"].includes(call.method)), false);
  });

  it("keeps a change made while a thread is starting eligible for recovery", async () => {
    const env = runtime();
    await env.baseline;
    env.state.generation = "two";
    env.state.changeDuringStart = true;
    env.state.turns = 0;
    await env.start();
    await env.run();
    env.state.turns = 1;
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("still creates a thread when its initial generation cannot be read", async () => {
    const env = runtime();
    await env.baseline;
    env.state.readFailure = "bridge unavailable";
    await env.start();
    assert.equal(env.calls.filter((call) => call.method === "thread/start").length, 1);
    env.state.readFailure = "";
    env.state.generation = "two";
    env.state.turns = 1;
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
  });

  it("continues ephemeral turns without paging, unloading, resuming, or repeated reads", async () => {
    const env = runtime();
    await env.baseline;
    env.state.generation = "two";
    env.state.ephemeral = true;
    await env.run();
    await env.send("turn/start", { threadId: "thread" });
    env.state.generation = "three";
    await env.run();
    await env.send("turn/start", { threadId: "thread" });
    assert.equal(env.calls.filter((call) => call.method === "thread/read").length, 1);
    assert.equal(env.calls.filter((call) => call.method === "turn/start").length, 2);
    assert.equal(env.calls.some((call) => ["thread/turns/list",
      "thread/unsubscribe", "thread/resume"].includes(call.method)), false);
  });

  for (const failure of ["thread/read", "cache", "config/read", "thread/turns/list", "subscribe"]) {
    it(`continues after ${failure} fails before unloading and retries later`, async () => {
      const env = runtime();
      await env.baseline;
      env.state.generation = "two";
      env.state.preflightFailure = failure;
      await env.run();
      await env.send("turn/start", { threadId: "thread" });
      assert.equal(env.calls.filter((call) => call.method === "turn/start").length, 1);
      assert.equal(env.calls.some((call) => ["thread/unsubscribe", "thread/resume"].includes(call.method)), false);
      assert.match(env.notices[0], /旧连接/);
      assert.ok(env.diagnostics.includes("gateway_recovery_preflight_failed"));
      env.state.preflightFailure = "";
      await env.run();
      assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
    });
  }

  it("continues a provider mismatch without unloading or rewriting the thread", async () => {
    const env = runtime();
    env.state.generation = "two";
    env.state.provider = "openai";
    await env.run();
    await env.send("turn/start", { threadId: "thread" });
    assert.match(env.notices[0], /供应商与网关不一致.*旧连接/);
    assert.ok(env.diagnostics.includes("gateway_recovery_provider_mismatch"));
    assert.equal(env.calls.filter((call) => call.method === "turn/start").length, 1);
    assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe"), false);
    assert.equal(env.calls.some((call) => call.method === "thread/resume"), false);
  });

  it("does not send after an unsubscribe may already have unloaded the thread", async () => {
    const env = runtime();
    env.state.generation = "two";
    env.state.failUnsubscribe = true;
    await assert.rejects(env.run(), /unsubscribe failed/);
    assert.equal(env.state.status, "notLoaded");
    assert.equal(env.calls.some((call) => call.method === "turn/start"), false);
  });

  for (const readFailure of ["", "thread/read"]) {
    it(`does not send when another recovery starts during a thread read, failure=${readFailure}`, async () => {
      const env = runtime();
      await env.baseline;
      env.state.generation = "two";
      env.state.pendingDuringRead = true;
      env.state.preflightFailure = readFailure;
      await assert.rejects(env.run());
      assert.equal(env.calls.some((call) => ["thread/unsubscribe", "thread/resume", "turn/start"]
        .includes(call.method)), false);
    });
  }

  it("retries failed resumes and changes observed during preparation", async () => {
    const env = runtime();
    env.state.generation = "changed";
    env.state.failResume = true;
    await assert.rejects(env.run(), /resume failed/);
    env.state.failResume = false;
    env.state.changeDuringResume = true;
    await assert.rejects(env.run(), /网关连接已变化/);
    env.state.changeDuringResume = false;
    await env.run();
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 3);
  });

  it("leaves direct and remote turns untouched and respects an existing context refresh", async () => {
    const env = runtime();
    env.state.active = false;
    await env.run();
    assert.equal(env.calls.length, 0);
    env.state.active = true;
    env.state.generation = "two";
    env.window.__codexPlusContextRefreshPending = new Set(["thread"]);
    await assert.rejects(env.run(), /正在检查连接/);
    assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe"), false);
    env.state.preflightFailure = "thread/read";
    await assert.rejects(env.run(), /正在检查连接/);
  });
});

describe("gateway creation through installed request wrappers", () => {
  const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
  const instancePatch = catalog.slice(catalog.indexOf("  function patchAppServerModelRequestClient("),
    catalog.indexOf("  // issue #2177"));
  const prototypePatch = catalog.slice(catalog.indexOf("    function installCodexAppServerClientPrototypePatch()"),
    catalog.indexOf("  const appServerModelRequestPatchMaxMisses"));
  const prewarm = source.slice(source.indexOf("  async function prewarmCodexManagedSession("),
    source.indexOf("  function codexRemoteSessionStartedThreadId("));
  const prelude = readFileSync(new URL("../../../assets/inject/renderer-inject/00-prelude.js", import.meta.url), "utf8");
  const version = prelude.match(/const codexAppServerModelRequestPatchVersion = "([^"]+)"/)![1];

  for (const mode of ["instance", "prototype", "prewarm", "envelope"]) {
    it(`records the confirmed thread generation through ${mode}`, async () => {
      const calls: string[] = [];
      const state = { generation: "one" };
      class Client {
        hostId = "local";
        async sendRequest(method: string, params: any): Promise<any> {
          if (method === "send-cli-request-for-host") return this.sendNative(params.method, params.params);
          return this.sendNative(method, params);
        }
        async sendNative(method: string, _params: any): Promise<any> {
          calls.push(method);
          if (method === "thread/start") return { thread: { id: "fresh" } };
          if (method === "turn/start") return { turn: { id: "turn" } };
          throw new Error(`Unexpected request: ${method}`);
        }
        async prewarmThreadStart(params: any) { return this.sendNative("thread/start", params); }
      }
      const client = new Client();
      const context = vm.createContext({
        window: { __codexPlusAppServerClientClass: Client }, client,
        codexAppServerModelRequestPatchVersion: version,
        codexRemoteSessionActiveProfile: () => ({ id: "codex-plus-gateway", relayMode: "pureApi" }),
        readCodexAppServerPreparation: async () => ({ status: "ok", active: true, generation: state.generation }),
        codexAppServerPreparationTimeoutMs: 100, sendCodexPlusDiagnostic() {},
        codexPlusBackendSettingsLoaded: true, codexRemoteSessionProviderPatchEnabled: () => true,
        codexRemoteSessionProviderRequestMethod: () => false, codexRemoteSessionProviderOverrideEnabled: () => false,
        codexPlusModelUnlockEnabled: () => false, codexPluginMarketplacePatchEnabled: () => false,
        codexPlusSettings: () => ({}), codexManagedSessionConnection: () => null,
        applyCodexManagedSessionConnection: (_method: string, params: any) => params,
        applyCodexRemoteSessionProviderOverride: (_method: string, params: any) => params,
        prepareCodexPureApiResume: async (_method: string, params: any) => params,
        prepareCodexManagedSessionConnection: async (_method: string, params: any) => params,
        prepareCodexThreadContextStart: async (_method: string, params: any) => params,
        refreshCodexThreadModelBeforeTurn: async () => null, sendCodexManagedSessionTurn: async () => null,
        codexPlusPluginNativeInterceptClient: () => null, registerNativeHostClient() {},
        appServerModelRequestMethod: (method: string, params: any) =>
          method === "send-cli-request-for-host" ? params.method : method,
        verifyCodexPureApiResumeResult() {}, noteCodexManagedSessionStart() {}, noteCodexThreadRuntimeContext() {},
        codexThreadModelRequestState: () => ({}), codexPlusExtensionInterceptors: new Map(),
        AbortController, setTimeout, clearTimeout,
      });
      vm.runInContext(fragment + prewarm + instancePatch + prototypePatch, context);
      vm.runInContext(mode === "prototype" ? "installCodexAppServerClientPrototypePatch()"
        : "patchAppServerModelRequestClient(client)", context);
      await vm.runInContext("initializeCodexGatewayBaseline()", context);
      state.generation = "two";
      if (mode === "prewarm") {
        await client.prewarmThreadStart({ model: "chosen" });
      } else if (mode === "envelope") {
        await client.sendRequest("send-cli-request-for-host",
          { hostId: "local", method: "thread/start", params: { model: "chosen" } });
      } else {
        await client.sendRequest("thread/start", { model: "chosen" });
      }
      await client.sendRequest("turn/start", { threadId: "fresh", input: [] });
      await client.sendRequest("turn/start", { threadId: "fresh", input: [] });
      assert.deepEqual(calls, ["thread/start", "turn/start", "turn/start"]);
    });
  }
});
