import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

const renderer = new URL("../../../assets/inject/renderer-inject/", import.meta.url);
const service = readFileSync(new URL("30-service-tier.js", renderer), "utf8");
const serviceFragment = service.slice(0, service.indexOf("  function codexRemoteSessionStartedThreadId("));
const catalog = readFileSync(new URL("70-model-catalog.js", renderer), "utf8");
const requestWrapper = catalog.slice(
  catalog.indexOf("  function patchAppServerModelRequestClient("),
  catalog.indexOf("  // issue #2177"),
);
const patchVersion = readFileSync(new URL("00-prelude.js", renderer), "utf8")
  .match(/const codexAppServerModelRequestPatchVersion = "([^"]+)";/)![1];
const rpc = readFileSync(new URL("91-extension-rpc.js", renderer), "utf8");

function runtime(mode: "aggregate" | "pureApi", source: string, status: "idle" | "notLoaded" | "active",
  providerSyncEnabled = true) {
  const calls: { method: string; params: any; provider?: string | null }[] = [];
  const preparations: any[] = [];
  const notices: string[] = [];
  const diagnostics: string[] = [];
  const state = {
    indexedProvider: source,
    connectedProvider: status !== "notLoaded" ? source : null as string | null,
    status,
    ephemeral: false,
    hasTurns: true,
    generation: "one",
    generationDuringResume: "",
    activateOnRead: 0,
    pendingClient: null as Promise<any> | null,
    pendingCache: null as Promise<any> | null,
    cached: { modelProvider: source, resumeState: status === "notLoaded" ? "needs_resume" : "resumed",
      threadRuntimeStatus: { type: status } },
  };
  let threadReads = 0;
  const profile = { id: "relay", relayMode: mode, configContents: 'model_provider = "custom"' };
  const settings = {
    relayProfilesEnabled: true, activeRelayId: "relay", providerSyncEnabled,
    relayProfiles: [profile],
    managedSessionConnection: {
      profileId: "relay", provider: "codex_plus_managed", baseUrl: "http://127.0.0.1:62021/v1",
    },
  };
  const client = {
    hostId: "local",
    async sendRequest(method: string, params: any) {
      const call = { method, params, provider: state.connectedProvider };
      calls.push(call);
      if (method === "thread/read") {
        if (++threadReads === state.activateOnRead) state.status = "active";
        return { thread: { id: "thread", cwd: "C:/fixture",
          model: "saved-model", reasoningEffort: "high",
          status: { type: state.status }, modelProvider: state.indexedProvider, ephemeral: state.ephemeral } };
      }
      if (method === "config/read") return { config: { model_provider: "custom" } };
      if (method === "thread/turns/list") {
        if (state.ephemeral) throw new Error("ephemeral threads do not support thread/turns/list");
        return { data: state.hasTurns ? [{}] : [] };
      }
      if (method === "thread/unsubscribe") {
        state.status = "notLoaded";
        state.connectedProvider = null;
        state.cached.resumeState = "needs_resume";
        return {};
      }
      if (method === "thread/resume") {
        if (state.status === "notLoaded") {
          state.connectedProvider = params.modelProvider ?? state.indexedProvider;
          state.status = "idle";
        }
        if (state.generationDuringResume) state.generation = state.generationDuringResume;
        return { thread: { id: "thread", status: { type: state.status } },
          modelProvider: state.connectedProvider, model: params.model,
          reasoningEffort: params.config?.model_reasoning_effort ?? null };
      }
      if (method === "turn/start" || method === "turn/steer") return { provider: state.connectedProvider };
      throw new Error(`Unexpected request: ${method}`);
    },
    async getConversation() { return state.pendingCache ?? state.cached; },
    async unsubscribeInactiveConversation() {
      state.status = "notLoaded";
      state.connectedProvider = null;
      state.cached.resumeState = "needs_resume";
    },
  };
  const context = vm.createContext({
    client, window: { __codexSessionDeleteBridge: () => {} },
    codexAppServerModelRequestPatchVersion: patchVersion,
    codexPlusBackendSettings: settings, codexModelCatalog: {}, codexAppServerPreparationTimeoutMs: 100,
    registerNativeHostClient() {}, sendCodexPlusDiagnostic: (name: string) => diagnostics.push(name),
    showToast: (message: string) => notices.push(message),
    codexPlusPluginNativeInterceptClient: () => null,
    appServerModelRequestMethod: (method: string) => method,
    loadBackendSettingsState: async () => true, loadCodexModelCatalog: async () => ({}),
    readCodexAppServerPreparation: async (route: string, payload: any) => route === "/gateway/connection-generation"
      ? { status: "ok", active: true, generation: state.generation }
      : { status: "ok", threadId: payload.thread_id, model: "saved-model", reasoningEffort: "high" },
    postJson: async (route: string, payload: any) => {
      assert.equal(route, "/thread-provider/prepare");
      preparations.push(payload);
      state.indexedProvider = payload.target_provider;
      return { status: "ok" };
    },
    prepareCodexThreadContextStart: async (_method: string, params: any) => params,
    noteCodexThreadRuntimeContext() {}, refreshCodexThreadModelBeforeTurn: async () => null,
    codexThreadModelRequestState: () => ({}), codexPlusModelUnlockEnabled: () => false,
    AbortController, AbortSignal, setTimeout, clearTimeout,
  });
  vm.runInContext(serviceFragment + requestWrapper, context);
  assert.equal(vm.runInContext("patchAppServerModelRequestClient(client)", context), true);
  const plans = new Function("window", "codexPlusBackendSettings", "prepareCodexThreadProviderResume",
    "codexThreadProviderPreparations", "codexPlusAppServerClient",
    "codexManagedSessionConnection", "codexRemoteSessionActiveProfile", "codexRemoteSessionTargetProvider",
    "warnCodexActiveOldProvider", "codexAppServerPreparationTimeoutMs", "sendCodexPlusDiagnostic",
    "codexPlusExtensionListeners", "codexPlusExtensionInterceptors", "codexPlusRunWaterfall",
    "codexPlusMarkExtensionFailure", "showToast", "codexGatewayRecoveryEnabled", "prepareCodexGatewayReconnect",
    "codexGatewayPreparations", `${rpc}\nreturn { resolve: codexPlusRpcPlanResolve, resume: codexPlusRpcPlanResume };`)(
    {}, settings, context.prepareCodexThreadProviderResume, vm.runInContext("codexThreadProviderPreparations", context),
    async () => state.pendingClient ?? client, context.codexManagedSessionConnection,
    context.codexRemoteSessionActiveProfile, context.codexRemoteSessionTargetProvider,
    context.warnCodexActiveOldProvider, 30, context.sendCodexPlusDiagnostic, new Map(), new Map(),
    (_name: string, value: any) => value, () => {}, () => {}, context.codexGatewayRecoveryEnabled,
    context.prepareCodexGatewayReconnect, vm.runInContext("codexGatewayPreparations", context));
  return { calls, client, preparations, plan: plans.resolve, resumePlan: plans.resume, settings, state, notices, diagnostics,
    providerStates: vm.runInContext("codexThreadProviderPreparations", context),
    gatewayStates: vm.runInContext("codexGatewayPreparations", context), baseline: context.initializeCodexGatewayBaseline };
}

describe("provider and gateway recovery on the same turn", () => {
  async function switchedGateway() {
    const env = runtime("pureApi", "openai", "idle");
    env.settings.relayProfiles[0].id = env.settings.activeRelayId = "codex-plus-gateway";
    await env.baseline();
    env.state.generation = "two";
    return env;
  }

  for (const mode of ["direct", "candidate"]) {
    it(`recovers the provider and gateway once through the ${mode} path`, async () => {
      const env = await switchedGateway();
      const request = { threadId: "thread", input: [], model: "chosen-model", effort: "low" };
      if (mode === "candidate") {
        const frame = ["resolve", 1, { executionHostId: "local", conversationId: "thread",
          resume: { conversationId: "thread" }, start: { request } }];
        const plan = env.plan(frame);
        const signal = new AbortController().signal;
        await plan.before("thread", true, frame[2], signal);
        await plan.context(frame, signal);
      }
      assert.deepEqual(await env.client.sendRequest("turn/start", request), { provider: "custom" });
      assert.equal(env.preparations.length, 1);
      const resumes = env.calls.filter((call) => call.method === "thread/resume");
      assert.equal(resumes.length, 1);
      assert.equal(resumes[0].params.threadId, "thread");
      assert.equal(resumes[0].params.model, "chosen-model");
      assert.equal(resumes[0].params.config.model_reasoning_effort, "low");
      assert.equal(env.gatewayStates.get("thread").generation, "two");
      assert.equal(env.calls.filter((call) => call.method === "turn/start").length, 1);
    });
  }

  it("does not claim the new generation or send after the gateway changes during resume", async () => {
    const env = await switchedGateway();
    env.state.generationDuringResume = "three";
    await assert.rejects(env.client.sendRequest("turn/start", { threadId: "thread" }), /网关连接已变化/);
    assert.equal(env.gatewayStates.has("thread"), false);
    assert.equal(env.calls.filter((call) => call.method === "thread/resume").length, 1);
    assert.equal(env.calls.some((call) => call.method === "turn/start"), false);
  });

  for (const kind of ["ephemeral", "unmaterialized", "active"]) {
    it(`leaves a ${kind} thread on the native send path`, async () => {
      const env = await switchedGateway();
      if (kind === "ephemeral") env.state.ephemeral = true;
      else if (kind === "unmaterialized") env.state.hasTurns = false;
      else env.state.status = "active";
      assert.deepEqual(await env.client.sendRequest("turn/start", { threadId: "thread" }), { provider: "openai" });
      assert.equal(env.preparations.length, 0);
      assert.equal(env.calls.some((call) => ["thread/unsubscribe", "thread/resume"].includes(call.method)), false);
    });
  }
});

describe("aggregate transitions through the installed request wrapper", () => {
  for (const source of ["custom", "openai"]) {
    it(`does not prepare the ordinary index when entering aggregate from ${source}`, async () => {
      const env = runtime("aggregate", source, "notLoaded");
      const result = await env.client.sendRequest("thread/resume", { threadId: "thread" });

      assert.equal(result.modelProvider, "codex_plus_managed");
      assert.equal(env.preparations.length, 0);
    });
  }

  it("does not send on a loaded managed connection after switching to pure API", async () => {
    const env = runtime("pureApi", "codex_plus_managed", "idle");
    let failure: unknown;
    try {
      await env.client.sendRequest("turn/start", { threadId: "thread", input: [] });
    } catch (error) {
      failure = error;
    }

    assert.equal(env.preparations.length, 0);
    const turns = env.calls.filter((call) => call.method === "turn/start");
    if (turns.length) {
      assert.equal(turns.length, 1);
      assert.equal(turns[0].provider, "custom");
    } else {
      assert.match(String(failure), /供应商|连接|provider|connection/i);
    }
  });

  for (const enabled of [false, true]) {
    for (const status of ["idle", "notLoaded"] as const) {
      for (const mode of ["aggregate", "pureApi"] as const) {
        it(`reconnects ${status} through the real turn candidate for ${mode}, sync=${enabled}`, async () => {
          const source = mode === "aggregate" ? "openai" : "codex_plus_managed";
          const target = mode === "aggregate" ? "codex_plus_managed" : "custom";
          for (const field of ["resume", "steer", "absent"]) {
            const env = runtime(mode, source, status, enabled);
            const candidate: any = {
              executionHostId: "local", conversationId: "thread",
              resume: { conversationId: "thread" },
              start: { request: { threadId: "thread", input: [[]] } },
              steer: { input: [[]], restoreMessage: { submissionOptions: {} } },
            };
            if (field === "resume") candidate.resume.modelProvider = source;
            if (field === "steer") candidate.steer.restoreMessage.submissionOptions.modelProvider = source;
            const plan = env.plan(["resolve", 1, candidate]);
            assert.ok(plan?.before);
            await plan.before("thread", true, candidate, new AbortController().signal);
            assert.equal(env.state.connectedProvider, target, `${mode}/${enabled}/${status}/${field}`);
            assert.equal(env.preparations.length, 0);
            const resume = env.calls.find((call) => call.method === "thread/resume");
            assert.equal(resume?.params.model, "saved-model");
            assert.equal(resume?.params.config?.model_reasoning_effort, "high");
            if (mode === "aggregate") {
              assert.equal(resume?.params.config?.["model_providers.codex_plus_managed"]?.base_url,
                "http://127.0.0.1:62021/v1");
            }
          }
        });
      }
    }
  }

  it("warns from the active cache for steer-only pure API turns when sync is disabled", async () => {
    const env = runtime("pureApi", "openai", "active", false);
    const candidate = { executionHostId: "local", conversationId: "thread",
      resume: { conversationId: "thread" }, steer: { input: [[]] } };
    const plan = env.plan(["resolve", 1, candidate]);
    assert.ok(plan?.before);
    await plan.before("thread", true, candidate, new AbortController().signal);
    const result = await env.client.sendRequest("turn/steer", { threadId: "thread", input: [] });
    assert.equal(result.provider, "openai");
    assert.match(env.notices[0], /旧供应商/);
    assert.equal(env.calls.some((call) => ["thread/read", "config/read", "thread/turns/list",
      "thread/unsubscribe", "thread/resume"].includes(call.method)), false);
  });

  for (const pending of ["client", "cache"] as const) {
    it(`does not block a steer when the optional provider ${pending} never responds`, async () => {
      const env = runtime("pureApi", "openai", "active", false);
      let finish!: (value: any) => void;
      const waiting = new Promise((resolve) => { finish = resolve; });
      if (pending === "client") env.state.pendingClient = waiting;
      else env.state.pendingCache = waiting;
      const candidate = { executionHostId: "local", conversationId: "thread",
        resume: { conversationId: "thread" }, steer: { input: [[]] } };
      const before = env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
      const outcome = await Promise.race([before.then(() => "continued"),
        new Promise((resolve) => setTimeout(() => resolve("blocked"), 150))]);
      finish(pending === "client" ? env.client : env.state.cached);
      await before;
      await Promise.resolve();
      assert.equal(outcome, "continued");
      assert.deepEqual(env.notices, []);
      assert.ok(env.diagnostics.includes("thread_provider_cache_read_failed"));
    });
  }

  it("keeps an active aggregate transition untouched", async () => {
    const env = runtime("aggregate", "openai", "active", true);
    const candidate = { executionHostId: "local", conversationId: "thread",
      resume: { conversationId: "thread" }, start: { request: { threadId: "thread", input: [[]] } } };
    await env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
    assert.equal(env.state.connectedProvider, "openai");
    assert.match(env.notices[0], /旧供应商/);
    assert.equal(env.calls.some((call) => call.method === "thread/unsubscribe" || call.method === "thread/resume"), false);
    assert.equal(env.preparations.length, 0);
  });

  for (const enabled of [false, true]) {
    it(`warns and continues an active steer-only aggregate turn, sync=${enabled}`, async () => {
      const env = runtime("aggregate", "openai", "active", enabled);
      const candidate = { executionHostId: "local", conversationId: "thread",
        resume: { conversationId: "thread" }, steer: { input: [[]] } };
      const plan = env.plan(["resolve", 1, candidate]);
      assert.ok(plan?.before);
      await plan.before("thread", true, candidate, new AbortController().signal);
      const result = await env.client.sendRequest("turn/steer", { threadId: "thread", input: [] });
      assert.equal(result.provider, "openai");
      assert.match(env.notices[0], /旧供应商/);
      assert.equal(env.calls.some((call) => ["thread/turns/list", "thread/unsubscribe", "thread/resume"]
        .includes(call.method)), false);
    });
  }

  for (const activateOnRead of [2, 3]) {
    it(`warns when an old-provider session becomes active on read ${activateOnRead}`, async () => {
      const env = runtime("pureApi", "openai", "idle", true);
      env.state.activateOnRead = activateOnRead;
      const candidate = { executionHostId: "local", conversationId: "thread",
        resume: { conversationId: "thread" }, start: { request: { threadId: "thread", input: [[]] } } };
      await env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
      assert.equal(env.state.connectedProvider, "openai");
      assert.match(env.notices[0], /旧供应商/);
      assert.equal(env.calls.some((call) => ["thread/unsubscribe", "thread/resume"].includes(call.method)), false);
    });
  }

  for (const enabled of [false, true]) {
    it(`sends an ephemeral aggregate turn without trying to migrate it, sync=${enabled}`, async () => {
      const env = runtime("aggregate", "custom", "idle", enabled);
      env.state.ephemeral = true;
      const candidate = { executionHostId: "local", conversationId: "thread",
        resume: { conversationId: "thread" }, start: { request: { threadId: "thread", input: [[]] } } };
      await env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
      const result = await env.client.sendRequest("turn/start", { threadId: "thread", input: [] });
      assert.equal(result.provider, "custom");
      assert.equal(env.calls.filter((call) => call.method === "turn/start").length, 1);
      assert.equal(env.calls.some((call) =>
        ["thread/turns/list", "thread/unsubscribe", "thread/resume"].includes(call.method)), false);
      assert.deepEqual(env.preparations, []);
    });

    it(`defers an attached side chat's provider change until it has history, sync=${enabled}`, async () => {
      const env = runtime("aggregate", "custom", "idle", enabled);
      const request = { threadId: "thread", input: [] };
      const candidate = { executionHostId: "local", conversationId: "thread",
        resume: { conversationId: "thread" }, start: { request } };
      env.state.hasTurns = false;
      await env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
      assert.equal((await env.client.sendRequest("turn/start", request)).provider, "custom");
      assert.ok(env.calls.some(call => call.method === "thread/turns/list"));
      assert.ok(!env.calls.some(call => ["thread/unsubscribe", "thread/resume"].includes(call.method)));
      assert.deepEqual(env.preparations, []);

      env.state.hasTurns = true;
      await env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
      assert.equal((await env.client.sendRequest("turn/start", request)).provider, "codex_plus_managed");
      assert.deepEqual(env.calls.filter(call => call.method === "turn/start").map(call => call.params.threadId),
        ["thread", "thread"]);
    });

    it(`warns when an active ephemeral turn keeps its old provider, sync=${enabled}`, async () => {
      const env = runtime("aggregate", "custom", "active", enabled);
      env.state.ephemeral = true;
      const candidate = { executionHostId: "local", conversationId: "thread",
        resume: { conversationId: "thread" }, steer: { input: [[]] } };
      const plan = env.plan(["resolve", 1, candidate]);
      assert.ok(plan?.before);
      await plan.before("thread", true, candidate, new AbortController().signal);
      const result = await env.client.sendRequest("turn/steer", { threadId: "thread", input: [] });
      assert.equal(result.provider, "custom");
      assert.match(env.notices[0], /旧供应商/);
      assert.equal(env.calls.some((call) => ["thread/turns/list", "thread/unsubscribe", "thread/resume"]
        .includes(call.method)), false);
    });
  }

  it("does not read ordinary session or configuration state when sync is off", async () => {
    const env = runtime("pureApi", "custom", "idle", false);
    const candidate = { executionHostId: "local", conversationId: "thread",
      resume: { conversationId: "thread" }, start: { request: { threadId: "thread", input: [[]] } } };
    await env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
    assert.deepEqual(env.calls, []);
    assert.deepEqual(env.preparations, []);
  });

  it("restores the saved model when a pure API resume carries the old managed alias", async () => {
    const env = runtime("pureApi", "codex_plus_managed", "notLoaded", false);
    const result = await env.client.sendRequest("thread/resume",
      { threadId: "thread", model: null, modelProvider: "codex_plus_managed" });
    assert.equal(result.modelProvider, "custom");
    assert.equal(result.model, "saved-model");
    assert.equal(result.reasoningEffort, "high");
    assert.equal(env.preparations.length, 0);
  });

  for (const enabled of [false, true]) {
    for (const mode of ["aggregate", "pureApi"] as const) {
      it(`reconnects a loaded ${mode} resume through RPC with sync=${enabled}`, async () => {
        const source = mode === "aggregate" ? "openai" : "codex_plus_managed";
        const target = mode === "aggregate" ? "codex_plus_managed" : "custom";
        for (const includeProvider of [false, true]) {
          const env = runtime(mode, source, "idle", enabled);
          const params = { threadId: "thread", ...(includeProvider ? { modelProvider: source } : {}) };
          const frame = ["push", ["pipeline", -19, ["resumeThread"], [params]]];
          const plan = env.resumePlan(frame, "local", null);
          assert.ok(plan?.before);
          await plan.before(params, true, frame, new AbortController().signal);
          assert.equal(env.state.connectedProvider, target);
          assert.equal(env.preparations.length, 0);
        }
      });
    }
  }

  it("unloads an explicit old managed provider before a direct pure API resume", async () => {
    const env = runtime("pureApi", "codex_plus_managed", "idle", false);
    const result = await env.client.sendRequest("thread/resume",
      { threadId: "thread", model: "chosen", modelProvider: "codex_plus_managed" });
    assert.equal(result.modelProvider, "custom");
    assert.equal(result.model, "chosen");
    assert.equal(env.preparations.length, 0);
  });

  it("continues on the native connection when aggregate preparation is unavailable before any changes", async () => {
    const env = runtime("aggregate", "openai", "idle", true);
    env.settings.managedSessionConnection.baseUrl = "";
    const candidate = { executionHostId: "local", conversationId: "thread",
      resume: { conversationId: "thread" }, start: { request: { threadId: "thread", input: [[]] } } };
    await env.plan(["resolve", 1, candidate]).before("thread", true, candidate, new AbortController().signal);
    const result = await env.client.sendRequest("turn/start", { threadId: "thread", input: [] });
    assert.equal(result.provider, "openai");
    assert.match(env.notices[0], /聚合供应商连接.*旧供应商/);
    assert.equal(env.calls.filter((call) => call.method === "turn/start").length, 1);
    assert.equal(env.calls.some((call) => ["thread/unsubscribe", "thread/resume"].includes(call.method)), false);
    assert.equal(env.state.connectedProvider, "openai");
    assert.equal(env.preparations.length, 0);
  });

  it("retains an error if aggregate preparation is unavailable after an earlier change", async () => {
    const env = runtime("aggregate", "openai", "idle", true);
    env.settings.managedSessionConnection.baseUrl = "";
    env.providerStates.set("thread", { tail: null, pending: { sourceProvider: "openai", targetProvider: "custom" } });
    const candidate = { executionHostId: "local", conversationId: "thread",
      resume: { conversationId: "thread" }, start: { request: { threadId: "thread", input: [[]] } } };
    await assert.rejects(env.plan(["resolve", 1, candidate])
      .before("thread", true, candidate, new AbortController().signal), /聚合供应商连接不可用/);
    assert.equal(env.calls.some((call) => call.method === "turn/start"), false);
  });
});
