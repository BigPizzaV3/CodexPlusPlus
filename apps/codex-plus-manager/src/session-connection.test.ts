import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

const renderer = readFileSync(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const source = renderer.slice(
  renderer.indexOf("  function codexManagedSessionConnection()"),
  renderer.indexOf("  function codexRemoteSessionStartedThreadId("),
);

function runtime() {
  const calls: { method: string; params: any }[] = [];
  const window = {} as any;
  const state = {
    profile: { id: "aggregate", relayMode: "aggregate" },
    settings: { managedSessionConnection: {
      profileId: "aggregate", provider: "codex_plus_managed", baseUrl: "http://127.0.0.1:62021/v1",
    } },
    status: "idle",
    refresh: true,
    rejectResume: false,
    provider: "",
    path: "",
    returnedProvider: "codex_plus_managed",
    snapshot: {} as any,
  };
  const client = { async sendRequest(method: string, params: any) {
    calls.push({ method, params });
    if (method === "thread/read") return { thread: { modelProvider: state.provider, path: state.path, status: { type: state.status } } };
    if (state.rejectResume && method === "thread/resume" && params.config) throw new Error("resume failed");
    if (method === "thread/resume" && !params.config) return { ...state.snapshot, modelProvider: state.returnedProvider };
    return { modelProvider: state.returnedProvider };
  } };
  const create = new Function("window", "codexPlusBackendSettings", "codexRemoteSessionActiveProfile",
    "validThreadScrollSessionKey", "loadBackendSettingsState", "codexAppServerPreparationTimeoutMs",
    "nativeHostClient", "sendCodexPlusDiagnostic", "codexRemoteSessionTargetProvider",
    "applyCodexServiceTierRequestOnly", "normalizeCodexConfiguredModelRequest", `${source}
    return { apply: applyCodexManagedSessionConnection, prepare: prepareCodexManagedSessionConnection,
      send: sendCodexManagedSessionTurn,
      prewarm: prewarmCodexManagedSession, observeError: observeCodexManagedSessionConnectionError,
      note: noteCodexManagedSessionStart };`);
  const api = create(window, state.settings, () => state.profile, (id: string) => id === "thread" ? id : "",
    async () => state.refresh, 1500, () => client, () => {}, () => "custom",
    (_method: string, params: any) => params, (_method: string, params: any) => params);
  return { ...api, state, calls, client, window };
}

describe("managed aggregate session connection", () => {
  it("pins direct prewarm requests and records only confirmed protected starts", async () => {
    const api = runtime();
    const options = { priority: "critical" };
    const result = await api.prewarm({ hostId: "local" }, async (params: any, received: any) => {
      assert.equal(received, options);
      assert.equal(params.model, "chosen");
      assert.equal(params.modelProvider, "codex_plus_managed");
      assert.equal(params.config["model_providers.codex_plus_managed"].base_url,
        "http://127.0.0.1:62021/v1");
      return { thread: { id: "prewarmed" }, modelProvider: params.modelProvider };
    }, { model: "chosen" }, options);
    assert.equal(result.thread.id, "prewarmed");
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("prewarmed").needsRecovery, false);
  });

  it("marks asynchronous connection failures without issuing requests or replaying messages", () => {
    const api = runtime();
    assert.equal(api.observeError({ type: "app-server-notification", hostId: "local",
      notification: { method: "error", params: { threadId: "thread",
        error: { message: "Connection failed: error sending request" }, willRetry: true } } }), true);
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, true);
    assert.equal(api.calls.length, 0);
    assert.equal(api.observeError({ hostId: "remote", notification: { method: "error",
      params: { threadId: "remote", error: { message: "network" } } } }), false);
    assert.equal(api.observeError({ method: "turn/completed", params: { threadId: "other",
      turn: { error: { message: "Invalid model" } } } }), false);
    assert.equal(api.window.__codexPlusManagedSessionStarts.has("other"), false);
  });

  it("does not treat an unknown turn acknowledgement as proof of protected transport", async () => {
    const api = runtime();
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "fresh" });
    assert.equal(api.window.__codexPlusManagedSessionStarts.has("fresh"), false);
    assert.deepEqual(api.calls.map((call: any) => call.method), ["turn/start"]);
  });

  it("keeps a proven transport after a transient stream failure instead of unloading it", async () => {
    const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    api.state.path = "rollout.jsonl";
    api.state.provider = "codex_plus_managed";
    api.observeError({ method: "error", params: { threadId: "thread",
      error: { message: "network disconnected" } } });
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "thread" });
    assert.deepEqual(api.calls.map((call: any) => call.method), ["thread/read", "turn/start"]);
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, false);
  });

  it("reloads a cold thread even if its persisted provider matches the previous proven signature", async () => {
    const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    api.state.path = "rollout.jsonl";
    api.state.status = "notLoaded";
    api.state.provider = "codex_plus_managed";
    api.observeError({ method: "error", params: { threadId: "thread", error: { message: "network" } } });
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "thread" });
    assert.deepEqual(api.calls.map((call: any) => call.method),
      ["thread/read", "thread/unsubscribe", "thread/resume", "turn/start"]);
  });

  it("preserves asynchronous errors delivered before the turn acknowledgement", async () => {
    const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    await api.send({ hostId: "local" }, async () => {
      api.observeError({ method: "error", params: { threadId: "thread",
        error: { message: "network disconnected" } } });
      return { turn: { id: "turn" } };
    }, "turn/start", { threadId: "thread" });
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, true);
    api.observeError({ method: "turn/completed", params: { threadId: "thread",
      turn: { status: "completed", error: null } } });
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, false);
  });

  it("recognizes structured stream errors without classifying ordinary configuration errors as network errors", () => {
    const api = runtime();
    for (const error of [
      { message: "重连中", codexErrorInfo: { responseStreamDisconnected: { httpStatusCode: null } } },
      { message: "Request failed", additionalDetails: "Connection failed: error sending request" },
    ]) {
      assert.equal(api.observeError({ method: "error", params: { threadId: "thread", error } }), true);
    }
    assert.equal(api.observeError({ method: "error", params: { threadId: "thread",
      error: { message: "Unsupported connection setting" } } }), false);
    assert.equal(api.observeError({ method: "error", params: { threadId: "x".repeat(129),
      error: { message: "network" } } }), false);
    const cycle: any = {}; cycle.message = cycle;
    assert.equal(api.observeError(cycle), false);
  });

  it("bounds cache size and ignores network-like text inside normal output notifications", () => {
    const api = runtime();
    for (let index = 0; index < 300; index += 1) {
      api.observeError({ method: "error", params: { threadId: `thread-${index}`,
        error: { message: "network disconnected" } } });
    }
    assert.equal(api.window.__codexPlusManagedSessionStarts.size, 256);
    assert.equal(api.observeError({ type: "mcp-notification", method: "item/agentMessage/delta",
      params: { threadId: "normal", delta: { method: "error", params: { threadId: "normal",
        error: { message: "network disconnected" } } } } }), false);
    assert.equal(api.window.__codexPlusManagedSessionStarts.has("normal"), false);
  });

  it("uses isolated finite RPC options for recovery and preserves submission callbacks for the user turn", async () => {
    const api = runtime();
    api.state.path = "rollout.jsonl";
    api.observeError({ method: "error", params: { threadId: "thread", error: { message: "network" } } });
    const options = { clientUserMessageId: "message", onOutcomeUnknown() {}, timeoutMs: 0,
      priority: "critical", trace: { id: "user-trace" } };
    const methods: string[] = [];
    await api.send({ hostId: "local" }, async (method: string, params: any, received: any) => {
      methods.push(method);
      if (method === "turn/start") assert.equal(received, options);
      else {
        assert.equal(received.timeoutMs, method === "thread/read" ? 800 : 5000);
        assert.equal(received.onOutcomeUnknown, undefined);
        assert.equal(received.clientUserMessageId, undefined);
        assert.equal(received.trace, undefined);
      }
      return api.client.sendRequest(method, params);
    }, "turn/start", { threadId: "thread" }, options);
    assert.equal(methods.filter(method => method === "turn/start").length, 1);
  });

  it("stops after a recovery RPC timeout and releases the per-thread lock", async () => {
    for (const stage of ["snapshot", "unsubscribe", "resume"]) {
      const api = runtime();
      api.state.path = "rollout.jsonl";
      api.observeError({ method: "error", params: { threadId: "thread", error: { message: "network" } } });
      const methods: string[] = [];
      await assert.rejects(api.send({ hostId: "local" }, async (method: string, params: any, options: any) => {
        methods.push(method);
        if ((stage === "snapshot" && method === "thread/resume" && !params.config)
            || (stage === "unsubscribe" && method === "thread/unsubscribe")
            || (stage === "resume" && method === "thread/resume" && params.config)) {
          assert.equal(options.timeoutMs, 5000);
          throw new Error("Timeout");
        }
        return api.client.sendRequest(method, params);
      }, "turn/start", { threadId: "thread" }), /Timeout/);
      assert.equal(api.window.__codexPlusConnectionRecoveryPending.size, 0);
      assert.equal(methods.includes("turn/start"), false);
      assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, true);
    }
  });

  it("releases recovery lock before waiting for the user's turn acknowledgement", async () => {
    const api = runtime();
    api.state.path = "rollout.jsonl";
    api.observeError({ method: "error", params: { threadId: "thread", error: { message: "network" } } });
    await api.send({ hostId: "local" }, async (method: string, params: any) => {
      if (method === "turn/start") assert.equal(api.window.__codexPlusConnectionRecoveryPending.size, 0);
      return api.client.sendRequest(method, params);
    }, "turn/start", { threadId: "thread" });
  });

  it("keeps a pending recovery isolated from other healthy threads", async () => {
    const api = runtime();
    api.state.path = "rollout.jsonl";
    api.observeError({ method: "error", params: { threadId: "thread", error: { message: "network" } } });
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "other" }, modelProvider: "codex_plus_managed" }, "local");
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const first = api.send({ hostId: "local" }, async (method: string, params: any) => {
      if (method === "thread/read") await gate;
      return api.client.sendRequest(method, params);
    }, "turn/start", { threadId: "thread" });
    try {
      await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "other" });
      assert.equal(api.calls.at(-1)?.params.threadId, "other");
      assert.equal(api.window.__codexPlusConnectionRecoveryPending.has("thread"), true);
    } finally {
      release();
      await first;
    }
    assert.equal(api.window.__codexPlusConnectionRecoveryPending.size, 0);
  });

  it("aborts on connection changes at each recovery boundary without sending a stale turn", async () => {
    for (const stage of ["read", "snapshot", "unsubscribe", "resume"]) {
      for (const change of ["mode", "port"]) {
        const api = runtime();
        api.state.path = "rollout.jsonl";
        api.observeError({ method: "error", params: { threadId: "thread", error: { message: "network" } } });
        const methods: string[] = [];
        await assert.rejects(api.send({ hostId: "local" }, async (method: string, params: any) => {
          methods.push(method);
          const result = await api.client.sendRequest(method, params);
          if ((stage === "read" && method === "thread/read")
              || (stage === "snapshot" && method === "thread/resume" && !params.config)
              || (stage === "unsubscribe" && method === "thread/unsubscribe")
              || (stage === "resume" && method === "thread/resume" && params.config)) {
            if (change === "mode") api.state.profile.relayMode = "official";
            else api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
          }
          return result;
        }, "turn/start", { threadId: "thread" }), /连接配置已变化/);
        assert.equal(methods.includes("turn/start"), false);
        assert.equal(api.window.__codexPlusConnectionRecoveryPending.size, 0);
      }
    }
  });

  it("does not refresh settings when prewarming remote or nonaggregate sessions", async () => {
    const api = runtime();
    api.window.__codexSessionDeleteBridge = {};
    api.state.profile.relayMode = "official";
    let calls = 0;
    await api.prewarm({ hostId: "local" }, async (params: any) => {
      assert.equal(params.modelProvider, undefined);
      calls += 1;
      return {};
    }, { model: "chosen" });
    await api.prewarm({ hostId: "remote" }, async (params: any) => {
      assert.equal(params.modelProvider, undefined);
      calls += 1;
      return {};
    }, { model: "chosen" });
    assert.equal(calls, 2);
  });
  it("upgrades installed version 16 wrappers and sends fresh threads without a resume", async () => {
    const prelude = readFileSync(new URL("../../../assets/inject/renderer-inject/00-prelude.js", import.meta.url), "utf8");
    const version = prelude.match(/const codexAppServerModelRequestPatchVersion = "([^"]+)"/)?.[1];
    assert.notEqual(version, "16", "the previous mandatory-preflight closure must be replaced");
    const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
    const instancePatch = catalog.slice(catalog.indexOf("  function patchAppServerModelRequestClient("),
      catalog.indexOf("  // issue #2177"));
    const prototypePatch = catalog.slice(catalog.indexOf("    function installCodexAppServerClientPrototypePatch()"),
      catalog.indexOf("  const appServerModelRequestPatchMaxMisses"));
    for (const mode of ["instance", "prototype", "envelope"]) {
      const calls: string[] = [];
      let allowContextResume = false;
      let rejectContextResume = false;
      const nativeSend = async function(this: any, method: string, params: any): Promise<any> {
        assert.equal(this.hostId, "local", "preserve the native method receiver during upgrade");
        if (method === "send-cli-request-for-host") return nativeSend.call(this, params.method, params.params);
        calls.push(method);
        if (method === "thread/resume") {
          if (!allowContextResume) throw new Error("no rollout found for thread id fresh");
          assert.equal(params.model, "new");
          assert.equal(params.modelProvider, "codex_plus_managed");
          if (rejectContextResume) throw new Error("context refresh failed");
        }
        return method === "thread/start" ? { thread: { id: "fresh" } } : { turn: { id: "first" } };
      };
      class Client {
        hostId = "local";
        sendRequest = nativeSend;
        async prewarmThreadStart(params: any) { return nativeSend.call(this, "thread/start", params); }
      }
      const nativePrewarm = Client.prototype.prewarmThreadStart;
      const oldWrapper = async () => { throw new Error("old mandatory preflight wrapper"); };
      Object.assign(Client.prototype, {
        sendRequest: oldWrapper,
        __codexPlusModelOriginalSendRequest: nativeSend,
        __codexPlusModelRequestPatch: "16",
        __codexPlusServiceTierOriginalPrewarmThreadStart: nativePrewarm,
        prewarmThreadStart: oldWrapper,
      });
      const client = new Client();
      if (mode === "prototype") delete (client as any).sendRequest;
      else Object.assign(client, {
        sendRequest: oldWrapper,
        __codexPlusModelOriginalSendRequest: nativeSend,
        __codexPlusModelRequestPatch: "16",
      });
      const profile: any = { id: "aggregate", relayMode: "aggregate" };
      const context = vm.createContext({
        window: { __codexPlusAppServerClientClass: Client,
          __codexPlusAppServerClientPrototypePatchInstalled: "16", __codexSessionDeleteBridge: {} },
        client, codexAppServerModelRequestPatchVersion: version,
        codexPlusBackendSettings: { managedSessionConnection: {
          profileId: "aggregate", provider: "codex_plus_managed", baseUrl: "http://127.0.0.1:62021/v1",
        } },
        codexRemoteSessionActiveProfile: () => profile,
        codexPlusBackendSettingsLoaded: true,
        registerNativeHostClient() {}, codexPlusPluginNativeInterceptClient: () => null,
        appServerModelRequestMethod: (method: string, params: any) =>
          method === "send-cli-request-for-host" ? params.method : method,
        codexRemoteSessionProviderRequestMethod: () => true,
        codexRemoteSessionProviderPatchEnabled: () => true,
        codexRemoteSessionProviderOverrideEnabled: () => false,
        applyCodexRemoteSessionProviderOverride: (_method: string, params: any) => params,
        applyCodexServiceTierRequestOnly: (_method: string, params: any) => params,
        sendCodexRetainedModelRequest: (_client: unknown, send: Function, method: string, params: unknown, options: unknown) =>
          send(method, params, options),
        normalizeCodexConfiguredModelRequest: (_method: string, params: any) => params,
        loadBackendSettingsState: async () => { calls.push("settings/get"); return true; },
        codexAppServerPreparationTimeoutMs: 1500,
        sendCodexPlusDiagnostic() {}, codexPlusModelUnlockEnabled: () => false,
        codexPluginMarketplacePatchEnabled: () => false, codexPlusSettings: () => ({}),
        setTimeout, clearTimeout,
        codexRemoteSessionTargetProvider: () => "openai",
      });
      const contextHelpers = catalog.slice(catalog.indexOf("  function codexPerModelContextEnabled("),
        catalog.indexOf("  function patchAppServerModelRequestClient("));
      vm.runInContext(source + contextHelpers + instancePatch + prototypePatch, context);
      vm.runInContext(mode === "prototype" ? "installCodexAppServerClientPrototypePatch()"
        : "patchAppServerModelRequestClient(client)", context);
      const send = (method: string, params: any) => mode === "envelope"
        ? client.sendRequest("send-cli-request-for-host", { hostId: "local", method, params })
        : client.sendRequest(method, params);
      if (mode === "envelope") await send("thread/start", { model: "selected" });
      else await client.prewarmThreadStart({ model: "selected" });
      calls.length = 0;
      await send("turn/start", { threadId: "fresh", model: "selected" });
      assert.deepEqual(calls, ["turn/start"], mode);
      assert.equal((client as any).__codexPlusModelRequestPatch, version);
      allowContextResume = true;
      // 使用真实刷新函数，分别覆盖窗口、压缩和 metadata 配置。
      for (const key of ["modelWindows", "modelAutoCompact", "modelMetadata"]) {
        profile[key] = '{"old":200000,"new":1000000}';
        await send("thread/start", { model: "old" });
        calls.length = 0;
        await send("turn/start", { threadId: "fresh", model: "new" });
        assert.deepEqual(calls, ["thread/resume", "turn/start"], `${mode}:${key}`);
        calls.length = 0;
        await send("turn/start", { threadId: "fresh", model: "new" });
        assert.deepEqual(calls, ["turn/start"], `${mode}:unchanged`);
        // 同线程正在恢复时不得抢先刷新上下文或发送消息。
        await send("thread/start", { model: "old" });
        (context.window as any).__codexPlusConnectionRecoveryPending = new Set(["fresh"]);
        calls.length = 0;
        await assert.rejects(send("turn/start", { threadId: "fresh", model: "new" }), /正在检查连接/);
        assert.deepEqual(calls, []);
        (context.window as any).__codexPlusConnectionRecoveryPending.clear();
        // 刷新失败不能由受管发送覆盖缓存，下一条消息仍应重试刷新。
        await send("thread/start", { model: "old" });
        rejectContextResume = true;
        calls.length = 0;
        await send("turn/start", { threadId: "fresh", model: "new" });
        assert.deepEqual(calls, ["thread/resume", "turn/start"]);
        rejectContextResume = false;
        calls.length = 0;
        await send("turn/start", { threadId: "fresh", model: "new" });
        assert.deepEqual(calls, ["thread/resume", "turn/start"], `${mode}:retry`);
        delete profile[key];
      }
    }
  });

  it("accepts local native bridge error events and rejects foreign window events", () => {
    const api = runtime();
    api.window.location = { origin: "app://-" };
    api.window.addEventListener = () => {};
    api.window.removeEventListener = () => {};
    const listenerSource = renderer.slice(renderer.indexOf("  function installCodexRemoteSessionRecoveryListener("),
      renderer.indexOf("  function installCodexRemoteSessionDispatcherSubscription("));
    new Function("window", "codexRemoteSessionRecoveryVersion", "observeCodexRemoteSessionNotification",
      "sendCodexPlusDiagnostic", `${listenerSource}\ninstallCodexRemoteSessionRecoveryListener();`)(
      api.window, "test", api.observeError, () => {});
    const data = { type: "mcp-notification", hostId: "local", method: "error",
      params: { threadId: "thread", error: { message: "Reconnecting... waiting for network" } } };
    const handler = api.window.__codexPlusRemoteSessionRecoveryMessageHandler;
    assert.equal(handler({ source: {}, origin: "https://foreign.invalid", data }), false);
    assert.equal(handler({ source: null, origin: "", data: { ...data, hostId: "remote" } }), false);
    assert.equal(api.window.__codexPlusManagedSessionStarts, undefined);
    assert.equal(handler({ source: null, origin: "", data }), true);
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, true);
  });

  it("pins transport without changing model, context or unrelated configuration", () => {
    const api = runtime();
    const params = { model: "provider/model", config: {
      model_context_window: 12345,
      model_providers: {
        other: { name: "other" }, codex_plus_managed: { env_key: "MISSING" },
      },
      "model_providers.codex_plus_managed.base_url": "https://changed.invalid",
      "model_providers.codex_plus_managed.experimental_bearer_token": "changed",
    } };
    const patched = api.apply("thread/resume", params, "local");
    assert.equal(patched.model, params.model);
    assert.equal(patched.modelProvider, "codex_plus_managed");
    assert.equal(patched.config.model_context_window, 12345);
    assert.deepEqual(patched.config.model_providers, { other: { name: "other" } });
    assert.equal(params.config.model_providers.codex_plus_managed.env_key, "MISSING");
    assert.equal(patched.config["model_providers.codex_plus_managed"].base_url, "http://127.0.0.1:62021/v1");
    assert.equal(patched.config["model_providers.codex_plus_managed"].experimental_bearer_token, "codex-plus-aggregate");
    assert.equal(patched.config["model_providers.codex_plus_managed.base_url"], undefined);
    assert.equal(params.config["model_providers.codex_plus_managed.base_url"], "https://changed.invalid");
  });

  it("only patches local starts, resumes and ordinary forks", () => {
    const api = runtime();
    for (const method of ["thread/start", "thread/resume", "thread/fork"]) {
      assert.equal(api.apply(method, {}, "local").modelProvider, "codex_plus_managed");
      const params = {};
      assert.equal(api.apply(method, params, "remote"), params);
      assert.equal(api.apply(method, params, ""), params);
    }
    const inherited = { experimentalPredictionMode: true };
    assert.equal(api.apply("thread/fork", inherited, "local"), inherited);
    const turn = { input: [] };
    assert.equal(api.apply("turn/start", turn, "local"), turn);
  });

  it("protects aggregate failover sessions as well as model aggregation sessions", () => {
    const api = runtime();
    api.state.profile.aggregate = { modelAggregationEnabled: false, strategy: "failover" };
    const patched = api.apply("thread/start", { model: "chosen" }, "local");
    assert.equal(patched.modelProvider, "codex_plus_managed");
    assert.equal(patched.config["model_providers.codex_plus_managed"].base_url,
      "http://127.0.0.1:62021/v1");
  });

  it("rejects stale profile data, unsupported identities and non-loopback endpoints", () => {
    for (const mutation of [
      (api: any) => { api.state.profile.id = "other"; },
      (api: any) => { api.state.profile.relayMode = "official"; },
      (api: any) => { api.state.settings.managedSessionConnection.provider = "openai"; },
      (api: any) => { api.state.settings.managedSessionConnection.baseUrl = "https://external.invalid/v1"; },
    ]) {
      const api = runtime();
      mutation(api);
      const params = {};
      assert.equal(api.apply("thread/start", params, "local"), params);
    }
  });

  it("migrates protected threads to the selected provider after disabling protection", async () => {
    const api = runtime();
    api.state.profile.relayMode = "pureApi";
    const params = { threadId: "thread" };
    const patched = await api.prepare("thread/resume", params, "local",
      async () => ({ thread: { modelProvider: "codex_plus_managed" } }));
    assert.equal(patched.modelProvider, "custom");
    const ordinary = await api.prepare("thread/resume", params, "local",
      async () => ({ thread: { modelProvider: "other" } }));
    assert.equal(ordinary, params);
  });

  it("bounds identity probes for official and formerly managed resumes and preserves native fallback", async () => {
    for (const mode of ["official", "pureApi"]) {
      for (const failure of ["pending", "reject"]) {
        const api = runtime();
        if (mode === "pureApi") api.note("thread/start", api.apply("thread/start", {}, "local"),
          { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
        api.state.profile.relayMode = mode;
        const params = { threadId: "thread", cwd: "workspace" };
        let reads = 0;
        const result = await api.prepare("thread/resume", params, "local",
          async (method: string, received: any, options: any) => {
            reads++;
            assert.equal(method, "thread/read");
            assert.equal(received.includeTurns, false);
            assert.equal(options.timeoutMs, 800);
            assert.equal(options.onOutcomeUnknown, undefined);
            if (failure === "reject") throw new Error("read unavailable");
            return new Promise(() => {});
          });
        assert.equal(result, params);
        assert.equal(reads, 1);
      }
    }
  });

  it("bounds migration reads and ignores late provider results after native resume continues", async () => {
    const api = runtime();
    api.state.profile.relayMode = "pureApi";
    const params = { threadId: "thread", model: "provider/k3" };
    let finish!: (value: any) => void, reads = 0;
    const patched = await api.prepare("thread/resume", params, "local",
      async (method: string, received: any, options: any) => {
        reads++;
        assert.equal(method, "thread/read"); assert.equal(received.threadId, "thread");
        assert.equal(options.timeoutMs, 800);
        return new Promise(resolve => { finish = resolve; });
      });
    assert.equal(patched, params); assert.equal(reads, 1);
    finish({ thread: { modelProvider: "codex_plus_managed" } });
    await Promise.resolve();
    assert.deepEqual(params, { threadId: "thread", model: "provider/k3" });
  });

  it("patches a local host envelope at the nested request and preserves remote envelopes", async () => {
    const api = runtime();
    const params = { hostId: "local", method: "thread/start", params: { model: "chosen" } };
    const patched = await api.prepare("send-cli-request-for-host", params, "local", async () => ({}));
    assert.equal(patched.params.model, "chosen");
    assert.equal(patched.params.modelProvider, "codex_plus_managed");
    assert.equal(patched.config, undefined);
    const remote = { ...params, hostId: "remote" };
    assert.equal(await api.prepare("send-cli-request-for-host", remote, "local", async () => ({})), remote);
  });

  it("corrects the transport with the exact selected model before sending once", async () => {
    const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    const client = { hostId: "local", __codexPlusThreadModels: new Map() };
    const params = { threadId: "thread", model: "cpx-member-B/model-sol", input: [{ text: "hello" }] };
    await api.send(client, api.client.sendRequest, "turn/start", params);
    assert.deepEqual(api.calls.map((call: any) => call.method), ["turn/start"]);
    assert.equal(api.calls[0].params, params);
    assert.equal(client.__codexPlusThreadModels.get("thread"), params.model);
    api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
    api.state.path = "rollout.jsonl";
    await api.send(client, api.client.sendRequest, "turn/start", { ...params, model: "cpx-member-A/model" });
    assert.deepEqual(api.calls.map((call: any) => call.method),
      ["turn/start", "thread/read", "thread/resume", "thread/unsubscribe", "thread/resume", "turn/start"]);
    assert.equal(api.calls[4].params.model, "cpx-member-A/model");
    assert.equal(api.calls[5].params.model, "cpx-member-A/model");
    assert.equal(api.calls.filter((call: any) => call.method === "turn/start").length, 2);
  });

  it("does not block the first send on settings refresh and permits a later recovery retry", async () => {
    const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    const client = { hostId: "local" };
    const params = { threadId: "thread", model: "selected" };
    await api.send(client, api.client.sendRequest, "turn/start", params);
    assert.equal(api.calls.filter((call: any) => call.method === "turn/start").length, 1);
    api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
    api.state.path = "rollout.jsonl";
    api.state.rejectResume = true;
    await assert.rejects(api.send(client, api.client.sendRequest, "turn/start", params), /resume failed/);
    assert.equal(api.calls.filter((call: any) => call.method === "turn/start").length, 1);
    api.state.rejectResume = false;
    await api.send(client, api.client.sendRequest, "turn/start", params);
    assert.equal(api.calls.filter((call: any) => call.method === "turn/start").length, 2);
  });

  it("serializes automatic correction without waiting for normal turns", async () => {
    const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    const client = { hostId: "local" };
    const params = { threadId: "thread" };
    await api.send(client, api.client.sendRequest, "turn/start", params);
    api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
    api.state.path = "rollout.jsonl";
    let release!: () => void;
    const gate = new Promise<void>((done) => { release = done; });
    const first = api.send(client, async (method: string, values: any) => {
      if (method === "thread/read") await gate;
      return api.client.sendRequest(method, values);
    }, "turn/start", params);
    await assert.rejects(api.send(client, api.client.sendRequest, "turn/start", params), /正在检查/);
    release();
    await first;
    assert.equal(api.calls.at(-1)?.method, "turn/start");
    api.state.status = "active";
    await api.send(client, api.client.sendRequest, "turn/start", params);
    assert.equal(api.calls.filter((call: any) => call.method === "turn/start").length, 3);
  });

  it("wraps preflight RPCs inside host envelopes and ignores remote or unsupported modes", async () => {
    const api = runtime();
    const calls: any[] = [];
    const send = async (method: string, params: any, options: any) => {
      calls.push({ method, params, options });
      assert.equal(method, "send-cli-request-for-host");
      return api.client.sendRequest(params.method, params.params);
    };
    const client = { hostId: "local" };
    const params = { hostId: "local", method: "turn/start", params: { threadId: "thread", model: "chosen" } };
    const options = { tag: "keep" };
    await api.send(client, send, "send-cli-request-for-host", params, options);
    assert.deepEqual(calls.map((call) => call.params.method), ["turn/start"]);
    assert.ok(calls.every((call) => call.options === options && call.params.hostId === "local"));
    assert.equal(await api.send(client, send, "send-cli-request-for-host", { ...params, hostId: "remote" }), null);
    assert.equal(await api.send({ hostId: "remote" }, send, "turn/start", params.params), null);
    api.state.profile.relayMode = "pureApi";
    assert.equal(await api.send(client, send, "turn/start", params.params), null);
    assert.equal(calls.length, 1);
  });

  it("keeps normal model changes in the loaded session and only reloads changed connections", async () => {
    const api = runtime();
    const client = { hostId: "local" };
    const params = { threadId: "thread", model: "selected" };
    const start = api.apply("thread/start", { model: "selected" }, "local");
    api.note("thread/start", start, { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    api.state.provider = "codex_plus_managed";
    await api.send(client, api.client.sendRequest, "turn/start", params);
    assert.deepEqual(api.calls.map((call: any) => call.method), ["turn/start"]);
    // Fresh threads cannot be resumed until a first message creates their rollout.
    api.state.path = "rollout.jsonl";
    await api.send(client, api.client.sendRequest, "turn/start", { ...params, model: "new-selection" });
    assert.equal(api.calls[1].method, "turn/start");
    assert.equal(api.calls[1].params.model, "new-selection");
    await api.send(client, api.client.sendRequest, "turn/start", { ...params, model: "new-selection" });
    assert.equal(api.calls.filter((call: any) => call.method === "thread/resume").length, 0);
    api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
    await api.send(client, api.client.sendRequest, "turn/start", params);
    assert.equal(api.calls.filter((call: any) => call.params.config).length, 1);
  });

  it("does not preflight an unknown session before its first send", async () => {
    const api = runtime();
    api.state.provider = "codex_plus_managed";
    api.state.returnedProvider = "wrong";
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "thread", model: "selected" });
    assert.deepEqual(api.calls.map((call: any) => call.method), ["turn/start"]);
    assert.equal(api.calls.filter((call: any) => call.method === "turn/start").length, 1);
  });

  it("marks connection failures for the next send without replaying the failed message", async () => {
    const api = runtime();
    const client = { hostId: "local" };
    const params = { threadId: "thread", model: "selected" };
    const failure = new Error("stream disconnected before completion");
    let sends = 0;
    await assert.rejects(api.send(client, async () => {
      sends += 1;
      throw failure;
    }, "turn/start", params), (error: unknown) => error === failure);
    assert.equal(sends, 1);
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, true);
    api.state.path = "rollout.jsonl";
    await api.send(client, api.client.sendRequest, "turn/start", params);
    assert.deepEqual(api.calls.map((call: any) => call.method),
      ["thread/read", "thread/resume", "thread/unsubscribe", "thread/resume", "turn/start"]);
    assert.equal(api.window.__codexPlusManagedSessionStarts.get("thread").needsRecovery, false);
  });

  it("leaves ordinary request errors outside connection recovery", async () => {
    const api = runtime();
    const params = { threadId: "thread" };
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", params);
    await assert.rejects(api.send({ hostId: "local" }, async () => {
      throw new Error("invalid model");
    }, "turn/start", params), /invalid model/);
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", params);
    assert.deepEqual(api.calls.map((call: any) => call.method), ["turn/start", "turn/start"]);
  });

  it("never resumes a fresh thread even when its connection signature changes", async () => {
    const api = runtime();
    const start = api.apply("thread/start", {}, "local");
    api.note("thread/start", start, { thread: { id: "thread" } }, "local");
    api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "thread" });
    assert.deepEqual(api.calls.map((call: any) => call.method), ["thread/read", "turn/start"]);
  });

  it("bounds recovery reads and allows native sending after timeout or read failure", async () => {
    for (const timeout of [false, true]) {
      const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
      const params = { threadId: "thread" };
      const client = { hostId: "local" };
      await api.send(client, api.client.sendRequest, "turn/start", params);
      api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
      const result = await api.send(client, async (method: string) => {
        assert.equal(method, "thread/read");
        if (timeout) return await new Promise(() => {});
        throw new Error("read unavailable");
      }, "turn/start", params);
      assert.equal(result, null);
      assert.equal(api.window.__codexPlusConnectionRecoveryPending.size, 0);
      assert.deepEqual(api.calls.map((call: any) => call.method), ["turn/start"]);
    }
  });

  it("preserves exposed runtime preferences during repair and respects this turn's explicit choice", async () => {
    const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    api.state.snapshot = {
      model: "old-model", serviceTier: "flex", reasoningEffort: "low", cwd: "C:\\project",
      approvalPolicy: "on-request", approvalsReviewer: "user", runtimeWorkspaceRoots: ["C:\\project"],
      sandbox: { type: "workspaceWrite", writableRoots: ["C:\\project"], networkAccess: false,
        excludeTmpdirEnvVar: true, excludeSlashTmp: true },
    };
    const params = { threadId: "thread", model: "selected", effort: "high", serviceTier: "fast" };
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start",
      { threadId: "thread", model: "old-model" });
    api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
    api.state.path = "rollout.jsonl";
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", params);
    const resume = api.calls.find((call: any) => call.params.config).params;
    assert.equal(resume.model, "selected");
    assert.equal(resume.serviceTier, "fast");
    assert.equal(resume.config.model_reasoning_effort, "high");
    assert.equal(resume.cwd, "C:\\project");
    assert.equal(resume.approvalPolicy, "on-request");
    assert.deepEqual(resume.runtimeWorkspaceRoots, ["C:\\project"]);
    assert.equal(resume.sandbox, "workspace-write");
    assert.deepEqual(resume.config.sandbox_workspace_write, {
      writable_roots: ["C:\\project"], network_access: false,
      exclude_tmpdir_env_var: true, exclude_slash_tmp: true,
    });
    assert.equal(api.calls.at(-1)?.params, params);
  });

  it("does not unload when live permissions or a newly active turn cannot be preserved", async () => {
    for (const snapshot of [
      { sandbox: { type: "externalSandbox" } },
      { thread: { status: { type: "active" } } },
    ]) {
      const api = runtime();
    api.note("thread/start", api.apply("thread/start", {}, "local"),
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
      api.state.snapshot = snapshot;
      await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "thread" });
      api.state.settings.managedSessionConnection.baseUrl = "http://127.0.0.1:62022/v1";
      api.state.path = "rollout.jsonl";
      if (snapshot.thread?.status?.type === "active") api.state.status = "active";
      if (snapshot.thread?.status?.type === "active") {
        await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "thread" });
      } else {
        await assert.rejects(api.send({ hostId: "local" }, api.client.sendRequest, "turn/start",
          { threadId: "thread", model: "selected" }), /不支持/);
      }
      assert.equal(api.calls.filter((call: any) => call.method === "thread/unsubscribe").length, 0);
      assert.equal(api.calls.filter((call: any) => call.method === "turn/start").length,
        1);
    }
  });

  it("inherits proven connection provenance for ephemeral forks without resuming them", async () => {
    const api = runtime();
    const start = api.apply("thread/start", {}, "local");
    api.note("thread/start", start, { thread: { id: "parent" }, modelProvider: "codex_plus_managed" }, "local");
    api.note("thread/fork", { threadId: "parent", experimentalPredictionMode: true },
      { thread: { id: "thread" }, modelProvider: "codex_plus_managed" }, "local");
    api.state.provider = "codex_plus_managed";
    await api.send({ hostId: "local" }, api.client.sendRequest, "turn/start", { threadId: "thread", model: "new-model" });
    assert.deepEqual(api.calls.map((call: any) => call.method), ["turn/start"]);
  });
});
