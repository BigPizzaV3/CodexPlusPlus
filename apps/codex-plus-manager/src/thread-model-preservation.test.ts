import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL(
  "../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const override = source.slice(0, source.indexOf("  function codexManagedSessionConnection("));
const requestPatchVersion = readFileSync(new URL(
  "../../../assets/inject/renderer-inject/00-prelude.js", import.meta.url), "utf8")
  .match(/const codexAppServerModelRequestPatchVersion = "([^"]+)";/)![1];

function runtime(mode = "pureApi", providerSyncEnabled = false) {
  return new Function("codexPlusBackendSettings", "codexModelCatalog", "sendCodexPlusDiagnostic",
    `${override}\nreturn applyCodexRemoteSessionProviderOverride;`)(
    { relayProfilesEnabled: true, activeRelayId: "relay", providerSyncEnabled,
      relayProfiles: [{ id: "relay", relayMode: mode, configContents: 'model_provider = "custom"' }] },
    {}, () => {});
}

describe("thread model restoration ownership", () => {
  it("leaves thread-only resumes intact across distinct saved model and reasoning choices", () => {
    const apply = runtime();
    for (const [threadId, model, effort] of [
      ["thread-a", "model-a", "high"], ["thread-b", "model-b", "ultra"],
      ["thread-c", "outside-current-catalog", "low"],
    ]) {
      const params: { threadId: string; model?: string; modelProvider?: string; reasoningEffort?: string } = { threadId };
      const patched = apply("thread/resume", params);
      assert.equal(patched, params);
      assert.equal(Object.hasOwn(patched, "modelProvider"), false);
      assert.deepEqual(params, { threadId });
      const saved = { model, reasoningEffort: effort };
      const native = patched.model || patched.modelProvider || patched.reasoningEffort
        ? { model: "global-default", reasoningEffort: "medium" } : saved;
      assert.equal(native.model, model);
      assert.equal(native.reasoningEffort, effort);
    }
  });

  it("preserves field-free and foreign-provider resumes", () => {
    const apply = runtime();
    for (const params of [
      { threadId: "thread", reasoningEffort: "high" },
      { threadId: "thread", model: "chosen", modelProvider: "other", reasoningEffort: "ultra" },
    ]) {
      assert.equal(apply("thread/resume", params), params);
    }
  });

  it("rewrites explicit openai providers with supplier sync enabled or disabled", () => {
    for (const sync of [false, true]) {
      const apply = runtime("pureApi", sync);
      for (const key of ["modelProvider", "model_provider"]) {
        const params = { threadId: "old-openai", [key]: "openai" };
        const snapshot = { ...params };
        const patched = apply("thread/resume", params);
        assert.equal(patched.modelProvider, "custom");
        assert.equal(Object.hasOwn(patched, "model_provider"), false);
        assert.equal(Object.hasOwn(patched, "model"), false);
        assert.deepEqual(params, snapshot);
      }
    }
  });

  it("treats model and provider fields by presence, including null and empty values", () => {
    const apply = runtime();
    for (const params of [
      { threadId: "thread", model: null },
      { threadId: "thread", model: "" },
      { threadId: "thread", modelProvider: null },
      { threadId: "thread", model_provider: "" },
    ]) {
      assert.equal(apply("thread/resume", params).modelProvider, "custom");
    }
  });

  it("retains provider routing for starts and explicit model switches", () => {
    const apply = runtime();
    const params = { threadId: "thread", model: "chosen", reasoningEffort: "high" };
    assert.deepEqual(apply("thread/resume", params), { ...params, modelProvider: "custom" });
    assert.equal(apply("thread/start", {}).modelProvider, "custom");
    assert.equal(apply("thread/resume", params).model, "chosen");
    assert.equal(runtime("official")("thread/resume", params), params);
  });
});

const readSource = source.slice(source.indexOf("  async function readCodexAppServerPreparation("),
  source.indexOf("  async function loadBackendSettingsState("));

function snapshotRuntime(read?: (payload: any) => Promise<any>) {
  const settings: any = { relayProfilesEnabled: true, activeRelayId: "relay", providerSyncEnabled: false,
    relayProfiles: [{ id: "relay", relayMode: "pureApi", configContents: 'model_provider = "custom"' }] };
  const calls: any[] = [];
  const diagnostics: any[] = [];
  const notices: any[] = [];
  const api = new Function("codexPlusBackendSettings", "codexModelCatalog", "sendCodexPlusDiagnostic",
    "postJson", "codexAppServerPreparationTimeoutMs", "window", "showToast", `${override}\n${readSource}
      return { prepare: prepareCodexPureApiResume, apply: applyCodexRemoteSessionProviderOverride,
        verify: verifyCodexPureApiResumeResult };`)(
    settings, {}, (event: string, detail: any) => diagnostics.push({ event, detail }),
    async (route: string, payload: any) => {
      assert.equal(route, "/thread-resume-model-metadata");
      calls.push(payload);
      return read ? read(payload) : { status: "ok", threadId: payload.thread_id,
        model: `saved-${payload.thread_id}`, reasoningEffort: "high" };
    }, 30, {}, (message: string, options: any) => notices.push({ message, options }));
  return { ...api, settings, calls, diagnostics, notices };
}

describe("pure API persisted model snapshot", () => {
  it("keeps loaded matching-provider settings instead of restoring an older disk snapshot", async () => {
    for (const status of ["idle", "active"]) {
      for (const metadataFails of [false, true]) {
        const api = snapshotRuntime(async ({ thread_id }) => {
          if (metadataFails) throw new Error("snapshot unavailable");
          return { status: "ok", threadId: thread_id, model: "older-model", reasoningEffort: "high" };
        });
        const params = { threadId: "a", config: { model_context_window: 500000 } };
        const result = await api.prepare("thread/resume", params, "local", false,
          async () => ({ thread: { id: "a", status: { type: status }, modelProvider: "custom" },
            model: "user-chosen-model", reasoningEffort: "low" }));
        assert.equal(result, params);
        assert.equal(Object.hasOwn(result, "model"), false);
        assert.equal(Object.hasOwn(result.config, "model_reasoning_effort"), false);
        assert.deepEqual(api.notices, []);
      }
    }
  });

  it("warns and keeps the active old-provider session running", async () => {
    const api = snapshotRuntime();
    const params = { threadId: "a", model: null };
    const result = await api.prepare("thread/resume", params, "local", false,
      async () => ({ thread: { id: "a", status: { type: "active" }, modelProvider: "openai" } }));
    assert.equal(result, params);
    assert.equal(api.notices.length, 1);
    assert.match(api.notices[0].message, /旧供应商/);
    assert.equal(api.notices[0].options.type, "warn");
  });

  it("restores saved choices with provider sync enabled for an unloaded matching session", async () => {
    const api = snapshotRuntime();
    api.settings.providerSyncEnabled = true;
    const methods: string[] = [];
    const result = await api.prepare("thread/resume", { threadId: "a", model: null }, "local", false,
      async (method: string) => {
        methods.push(method);
        if (method === "thread/read") {
          return { thread: { id: "a", modelProvider: "custom", status: { type: "notLoaded" } } };
        }
        assert.equal(method, "config/read");
        return { config: { model_provider: "custom" } };
      });
    assert.equal(result.model, "saved-a");
    assert.equal(result.modelProvider, "custom");
    assert.equal(result.config.model_reasoning_effort, "high");
    assert.ok(methods.includes("thread/read"));
  });

  it("restores each saved model and effort after provider routing without mutating the request", async () => {
    for (const optional of [{}, { model: null }, { modelProvider: "openai" },
      { model_provider: "openai" }, { config: { model_reasoning_effort: "low" } }]) {
      const api = snapshotRuntime();
      const params = { threadId: "a", ...optional };
      const before = structuredClone(params);
      const result = await api.prepare("thread/resume", api.apply("thread/resume", params), "local");
      assert.equal(result.model, "saved-a");
      assert.equal(result.modelProvider, "custom");
      assert.equal(result.config.model_reasoning_effort,
        "config" in optional ? "low" : "high");
      assert.deepEqual(params, before);
      assert.equal(api.calls.length, 1);
      api.verify("thread/resume", result,
        { model: "wrong", modelProvider: "custom", reasoningEffort: result.config.model_reasoning_effort }, "local");
      assert.equal(api.diagnostics.at(-1).event, "pure_api_resume_result_mismatch");
    }
  });

  it("preserves explicit choices, remote requests and native OpenAI resumes", async () => {
    const api = snapshotRuntime();
    for (const params of [{ threadId: "a", model: "chosen" }, { threadId: "a", modelProvider: "other" }]) {
      assert.equal(await api.prepare("thread/resume", params, "local"), params);
    }
    const remote = { threadId: "a" };
    assert.equal(await api.prepare("thread/resume", remote, "remote"), remote);
    api.settings.relayProfiles[0].configContents = 'model_provider = "openai"';
    assert.equal(await api.prepare("thread/resume", remote, "local"), remote);
    assert.equal(api.calls.length, 0);
    api.settings.relayProfiles[0].relayMode = "official";
    assert.equal(await api.prepare("thread/resume", remote, "local"), remote);
    assert.equal(api.calls.length, 0);
  });

  it("leaves path-only resumes to the native client", async () => {
    const api = snapshotRuntime();
    const params = { path: "C:/fixture/rollout.jsonl" };
    assert.equal(await api.prepare("thread/resume", params, "local"), params);
    assert.deepEqual(api.calls, []);
  });

  it("does not use stale or incomplete metadata and rejects concurrent changes", async () => {
    const missing = snapshotRuntime(async () => ({ status: "failed" }));
    const result = await missing.prepare("thread/resume", { threadId: "a" }, "local");
    assert.equal(result.modelProvider, "custom");
    assert.equal(Object.hasOwn(result, "model"), false);
    assert.equal(missing.diagnostics.at(-1).detail.reason, "metadata_unavailable");

    const changed = snapshotRuntime(async ({ thread_id }) => {
      changed.settings.relayProfiles[0].configContents = 'model_provider = "changed"';
      return { status: "ok", threadId: thread_id, model: "saved", reasoningEffort: "high" };
    });
    await assert.rejects(changed.prepare("thread/resume", { threadId: "a" }, "local"), /配置已变化/);
  });

  it("falls back after refresh failure or a timed-out snapshot and ignores a late result", async () => {
    let finish!: (value: any) => void;
    const api = snapshotRuntime(() => new Promise((resolve) => { finish = resolve; }));
    const params = { threadId: "a", model: null };
    const fallback = await api.prepare("thread/resume", params, "local");
    assert.equal(Object.hasOwn(fallback, "model"), false);
    assert.equal(Object.hasOwn(fallback.config, "model_reasoning_effort"), false);
    finish({ status: "ok", threadId: "a", model: "late-model", reasoningEffort: "high" });
    await Promise.resolve();
    assert.equal(Object.hasOwn(fallback, "model"), false);

    const refresh = snapshotRuntime(async () => assert.fail("refresh failure must not read metadata"));
    const result = await refresh.prepare("thread/resume", params, "local", true);
    assert.equal(Object.hasOwn(result, "model"), false);
    assert.equal(refresh.diagnostics.at(-1).detail.reason, "provider_refresh_failed");
  });

  it("does not inject a saved model when the thread state read fails", async () => {
    const api = snapshotRuntime();
    const result = await api.prepare("thread/resume", { threadId: "a", model: null }, "local", false,
      async () => { throw new Error("thread state unavailable"); });
    assert.equal(Object.hasOwn(result, "model"), false);
    assert.equal(Object.hasOwn(result.config, "model_reasoning_effort"), false);
    assert.equal(api.diagnostics.at(-1).detail.reason, "thread_state_unavailable");
  });

  it("passes a sync-enabled resume through when its initial preflight read fails", async () => {
    for (const failure of ["thread/read", "config/read"]) {
      const api = snapshotRuntime(async () => assert.fail("no snapshot after failed preflight"));
      api.settings.providerSyncEnabled = true;
      const params = { threadId: "a", model: null };
      const result = await api.prepare("thread/resume", params, "local", false, async (method: string) => {
        if (method === failure) throw new Error("read unavailable");
        return { thread: { modelProvider: "openai", status: { type: "notLoaded" } } };
      });
      assert.equal(result, params);
      assert.equal(api.calls.length, 0);
    }
  });

  it("keeps the loaded-provider guard and supports local request envelopes", async () => {
    const api = snapshotRuntime();
    await assert.rejects(api.prepare("thread/resume", { threadId: "a" }, "local", false,
      async () => ({ thread: { modelProvider: "openai", status: { type: "idle" } } })), /旧供应商连接/);
    const envelope = { hostId: "local", method: "thread/resume", params: { threadId: "b" } };
    const result = await api.prepare("send-cli-request-for-host", envelope, "local");
    assert.equal(result.params.model, "saved-b");
    assert.deepEqual(envelope.params, { threadId: "b" });
  });
});

describe("nullable restoration through existing upstream wrappers", () => {
  const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
  const instance = catalog.slice(catalog.indexOf("  function patchAppServerModelRequestClient("), catalog.indexOf("  // issue #2177"));
  const prototype = catalog.slice(catalog.indexOf("    function installCodexAppServerClientPrototypePatch()"),
    catalog.indexOf("  const appServerModelRequestPatchMaxMisses"));

  for (const [relayMode, officialMixApiKey, source, target] of [
    ["pureApi", false, "openai", "custom"],
    ["pureApi", false, "custom", "openai"],
    ["official", false, "custom", "openai"],
    ["mixedApi", false, "openai", "custom"],
    ["mixedApi", false, "custom", "openai"],
    ["official", true, "openai", "custom"],
    ["official", true, "custom", "openai"],
  ] as const) {
    it(`retains native choices for ${relayMode}/${officialMixApiKey}: ${source} -> ${target}`, async () => {
      for (const mode of ["instance", "prototype", "envelope"]) {
        for (const [enabled, status] of [
          [false, "notLoaded"], [true, "notLoaded"], [true, "idle"], [true, "active"],
        ] as const) {
          const calls: any[] = [];
          const bridges: any[] = [];
          let currentStatus: string = status;
          class Client {
            hostId = "local";
            async sendRequest(method: string, params: any) {
              const name = method === "send-cli-request-for-host" ? params.method : method;
              const input = method === "send-cli-request-for-host" ? params.params : params;
              calls.push({ method: name, params: input });
              if (name === "thread/read") {
                return { thread: { id: "a", cwd: "C:/fixture", status: { type: currentStatus }, modelProvider: source } };
              }
              if (name === "config/read") return { config: { model_provider: target } };
              if (name === "thread/unsubscribe") currentStatus = "notLoaded";
              return { thread: { id: "a" }, model: "native-saved", reasoningEffort: "native-saved" };
            }
          }
          const client = new Client();
          const profile = { id: "relay", relayMode,
            officialMixApiKey, configContents: `model_provider = "${target}"` };
          const context = vm.createContext({
            window: { __codexPlusAppServerClientClass: Client, __codexSessionDeleteBridge: () => {} }, client, AbortController,
            codexAppServerModelRequestPatchVersion: requestPatchVersion,
            codexPlusBackendSettings: { relayProfilesEnabled: true, activeRelayId: "relay", providerSyncEnabled: enabled,
              relayProfiles: [profile] },
            codexModelCatalog: {}, codexPlusBackendSettingsLoaded: true, codexAppServerPreparationTimeoutMs: 2000,
            readCodexAppServerPreparation: async (_route: string, payload: any) => ({
              status: "ok", threadId: payload.thread_id, model: "native-saved", reasoningEffort: "native-saved",
            }),
            postJson: async (route: string, payload: any) => {
              assert.equal(route, "/thread-provider/prepare");
              bridges.push(payload);
              return { status: "ok" };
            },
            registerNativeHostClient() {}, sendCodexPlusDiagnostic() {}, codexPlusPluginNativeInterceptClient: () => null,
            appServerModelRequestMethod: (method: string, params: any) => method === "send-cli-request-for-host" ? params.method : method,
            loadBackendSettingsState: async () => true, loadCodexModelCatalog: async () => ({}),
            prepareCodexManagedSessionConnection: async (_method: string, params: any) => params,
            prepareCodexThreadContextStart: async (_method: string, params: any) => params,
            noteCodexThreadRuntimeContext() {},
            codexManagedSessionConnection: () => null, refreshCodexThreadModelBeforeTurn: async () => null,
            sendCodexManagedSessionTurn: async () => null, noteCodexManagedSessionStart() {},
            codexThreadModelRequestState: () => ({}), codexPlusModelUnlockEnabled: () => false,
            codexPluginMarketplacePatchEnabled: () => false, codexPlusSettings: () => ({}), setTimeout, clearTimeout,
          });
          vm.runInContext(override + instance + prototype, context);
          assert.equal(vm.runInContext(mode === "prototype"
            ? "installCodexAppServerClientPrototypePatch()" : "patchAppServerModelRequestClient(client)", context),
          mode !== "prototype" || enabled || relayMode === "pureApi" || officialMixApiKey);
          for (const choice of [
            { model: null, reasoningEffort: null, config: { model_reasoning_effort: null } },
            { model: "chosen", reasoningEffort: "low", config: { model_reasoning_effort: "high" } },
          ]) {
            const params = { threadId: "a", ...choice };
            currentStatus = status;
            const before = structuredClone(params);
            const bridgeCount = bridges.length;
            const callCount = calls.length;
            const result = await client.sendRequest(mode === "envelope" ? "send-cli-request-for-host" : "thread/resume",
              mode === "envelope" ? { hostId: "local", method: "thread/resume", params } : params);
            const dispatched = calls.slice(callCount);
            const snapshotCandidate = relayMode === "pureApi" && target !== "openai" && choice.model === null;
            const restoreSnapshot = snapshotCandidate && (!enabled || status !== "active");
            const expectedCalls = !enabled
              ? restoreSnapshot ? ["thread/read", "thread/resume"] : ["thread/resume"]
              : status === "notLoaded" ? ["thread/read", "config/read", "thread/resume"]
              : status === "active" ? ["thread/read", "thread/resume"]
              : ["thread/read", "config/read", "thread/read", "thread/read", "thread/unsubscribe", "thread/read", "thread/resume"];
            if (enabled && snapshotCandidate) expectedCalls.splice(expectedCalls.length - 1, 0, "thread/read");
            assert.deepEqual(dispatched.map((call) => call.method), expectedCalls);
            assert.equal(dispatched.at(-1).params.model, restoreSnapshot ? "native-saved" : choice.model);
            assert.equal(dispatched.at(-1).params.reasoningEffort, choice.reasoningEffort);
            assert.equal(dispatched.at(-1).params.config.model_reasoning_effort,
              restoreSnapshot ? "native-saved" : choice.config.model_reasoning_effort);
            assert.deepEqual(params, before);
            assert.equal(result.model, "native-saved");
            assert.equal(result.reasoningEffort, "native-saved");
            assert.equal(bridges.length - bridgeCount, enabled && status !== "active" ? 1 : 0,
              `${mode}: ${relayMode}/${officialMixApiKey}: ${source} -> ${target}, ${enabled}/${status}`);
            if (bridges.length > bridgeCount) {
              assert.deepEqual({ ...bridges.at(-1) }, { host_id: "local", thread_id: "a",
                source_provider: source, target_provider: target });
            }
          }
        }
      }
    });
  }

  for (const mode of ["instance", "prototype"]) {
    it(`replaces the ${mode} v37 helper closure on reinjection without stacking sends`, async () => {
      const helpers: string[] = [];
      const sent: any[] = [];
      class Client {
        hostId = "local";
        async sendRequest(method: string, params: any, options: any) {
          sent.push({ method, params, options, hostId: this.hostId });
          return { thread: { id: "a" } };
        }
      }
      const client = new Client();
      const context = vm.createContext({
        client, window: { __codexPlusAppServerClientClass: Client },
        codexPlusBackendSettingsLoaded: true, codexPlusBackendSettings: { providerSyncEnabled: true },
        codexPlusSettings: () => ({}), codexPlusModelUnlockEnabled: () => false,
        codexPluginMarketplacePatchEnabled: () => false, codexRemoteSessionProviderPatchEnabled: () => false,
        registerNativeHostClient() {}, codexPlusPluginNativeInterceptClient: () => null,
        appServerModelRequestMethod: (method: string) => method,
        codexRemoteSessionProviderRequestMethod: () => false, codexRemoteSessionProviderOverrideEnabled: () => false,
        applyCodexRemoteSessionProviderOverride: (_method: string, params: any) => params,
        prepareCodexManagedSessionConnection: async (_method: string, params: any) => params,
        prepareCodexThreadContextStart: async (_method: string, params: any) => params,
        prepareCodexGatewayReconnect: async () => {}, noteCodexGatewayThreadStart() {},
        noteCodexThreadRuntimeContext() {},
        refreshCodexThreadModelBeforeTurn: async () => null, sendCodexManagedSessionTurn: async () => null,
        verifyCodexPureApiResumeResult() {}, noteCodexManagedSessionStart() {}, codexThreadModelRequestState: () => ({}),
        sendCodexPlusDiagnostic() {},
        helper: async (_method: string, params: any) => { helpers.push("old"); return params; },
      });
      const inject = (version: string) => vm.runInContext(`((prepareCodexPureApiResume) => {
        const codexAppServerModelRequestPatchVersion = ${JSON.stringify(version)};
        ${instance}
        ${prototype}
        return ${mode === "instance" ? "patchAppServerModelRequestClient(client)" : "installCodexAppServerClientPrototypePatch()"};
      })(helper)`, context);
      assert.equal(inject("37"), true);
      const oldWrapper = client.sendRequest;
      const params = { threadId: "a", model: null, reasoningEffort: null };
      const options = { timeoutMs: 2000 };
      await client.sendRequest("thread/resume", params, options);
      context.helper = async (_method: string, input: any) => { helpers.push("new"); return input; };
      assert.equal(inject(requestPatchVersion), true);
      assert.notEqual(client.sendRequest, oldWrapper);
      const newWrapper = client.sendRequest;
      assert.equal(inject(requestPatchVersion), true);
      assert.equal(client.sendRequest, newWrapper);
      await client.sendRequest("thread/resume", params, options);
      assert.deepEqual(helpers, ["old", "new"]);
      assert.equal(sent.length, 2);
      for (const call of sent) {
        assert.equal(call.params, params);
        assert.equal(call.options, options);
        assert.equal(call.hostId, "local");
      }
    });
  }
});

describe("legacy provider sync cold installation", () => {
  const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
  const installs = catalog.slice(catalog.indexOf("  const appServerModelRequestPatchMaxMisses"),
    catalog.indexOf("  function runCodexModelWhitelistRefreshPass("));

  function startup(relayMode: string, foundOnSweep = 2) {
    const timers = new Map<number, { callback: () => void; delay: number }>();
    const delays: number[] = [];
    const routes: string[] = [];
    let sweeps = 0;
    let captures = 0;
    let patched = 0;
    let timerId = 0;
    const settings = { providerSyncEnabled: false, relayProfilesEnabled: true, activeRelayId: "relay",
      relayProfiles: [{ id: "relay", relayMode }] };
    const context = vm.createContext({
      codexPlusBackendSettings: settings, codexPlusBackendSettingsLoaded: false,
      window: { setTimeout(callback: () => void, delay: number) {
        timers.set(++timerId, { callback, delay });
        delays.push(delay);
        return timerId;
      } },
      clearTimeout: (id: number) => timers.delete(id),
      codexAppServerModelRequestPatchVersion: requestPatchVersion,
      codexPlusModelUnlockEnabled: () => false, codexPluginMarketplacePatchEnabled: () => false,
      codexPerModelContextEnabled: () => false,
      codexPlusSettings: () => ({}), installCodexThreadGoalForwarding() {}, installDictationSupportPatch() {},
      installModelJsonResponsePatch: () => assert.fail("provider sync must not unlock models"),
      patchAppServerModelMessages: () => assert.fail("provider sync must not patch model responses"),
      installCodexAppServerClientCapture: async () => { captures++; },
      postJson: async (route: string) => { routes.push(route); },
      loadAppServerRequestCandidates: async () => {
        sweeps++;
        return { modules: [{}], candidates: sweeps >= foundOnSweep ? [{}] : [], sources: [], discovery: "fixture" };
      },
      patchAppServerModelRequestClient: () => { patched++; return true; },
      sendCodexPlusDiagnostic() {},
    });
    vm.runInContext(override + installs, context);
    return {
      settings, routes, timers, delays, context,
      ensure: () => vm.runInContext("ensureCodexModelWhitelistInstalls()", context),
      settle: () => vm.runInContext("appServerModelRequestPatchPromise", context),
      counts: () => ({ sweeps, captures, patched }),
    };
  }

  for (const relayMode of ["official", "mixedApi"]) {
    it(`cold-starts ${relayMode} sync only after settings load and retries late client discovery`, async () => {
      const env = startup(relayMode);
      env.settings.providerSyncEnabled = true;
      env.ensure();
      assert.deepEqual(env.counts(), { sweeps: 0, captures: 0, patched: 0 });
      env.settings.providerSyncEnabled = false;
      env.context.codexPlusBackendSettingsLoaded = true;
      env.ensure();
      assert.deepEqual(env.counts(), { sweeps: 0, captures: 0, patched: 0 });
      assert.deepEqual(env.routes, []);
      assert.equal(env.timers.size, 0);

      env.settings.providerSyncEnabled = true;
      env.ensure();
      await env.settle();
      assert.deepEqual(env.counts(), { sweeps: 1, captures: 1, patched: 0 });
      assert.deepEqual(env.routes, ["/extensions/request-capture"]);
      const [id, timer] = env.timers.entries().next().value!;
      env.timers.delete(id);
      timer.callback();
      await env.settle();
      env.ensure();
      assert.equal(env.counts().sweeps, 2);
      assert.equal(env.counts().patched, 1);
      assert.deepEqual(env.routes, ["/extensions/request-capture"]);
      assert.deepEqual(env.delays, [250]);
      assert.equal(env.timers.size, 0);
    });

    it(`bounds ${relayMode} opt-in discovery retries without a legacy client`, async () => {
      const env = startup(relayMode, Infinity);
      env.settings.providerSyncEnabled = true;
      env.context.codexPlusBackendSettingsLoaded = true;
      env.ensure();
      await env.settle();
      for (let round = 0; round < 10 && env.timers.size; round++) {
        const [id, timer] = env.timers.entries().next().value!;
        env.timers.delete(id);
        timer.callback();
        await env.settle();
      }
      env.ensure();
      assert.equal(env.counts().sweeps, 8);
      assert.equal(env.counts().patched, 0);
      assert.deepEqual(env.delays, [250, 1000, 4000, 16000, 30000, 30000, 30000]);
      assert.equal(env.timers.size, 0);
      assert.deepEqual(env.routes, ["/extensions/request-capture"]);
    });
  }
});

describe("settings-driven catalog refresh ownership", () => {
  const settingsSource = source.slice(source.indexOf("  async function loadBackendSettingsState("));
  function settingsRuntime() {
    const previous = { enhancementsEnabled: true, providerSyncEnabled: false, modelCatalogFingerprint: "before" };
    const next = { enhancementsEnabled: true, providerSyncEnabled: true, modelCatalogFingerprint: "after" };
    const context = vm.createContext({
      window: { __CODEX_PLUS_TEST_SERVICE_TIER__: true },
      codexPlusBackendSettings: previous, codexPlusBackendSettingsLoaded: false, codexPlusBackendSettingsSeq: 1,
      postJson: async () => next,
    });
    vm.runInContext(settingsSource, context);
    return { context, previous, next };
  }

  it("publishes current settings before the guarded catalog refresh with the previous snapshot", async () => {
    const { context, previous, next } = settingsRuntime();
    const calls: any[] = [];
    context.refreshCodexModelCatalogForSettings = (before: any, after: any) => {
      assert.equal(before, previous);
      assert.equal(after, context.codexPlusBackendSettings);
      assert.equal(context.codexPlusBackendSettingsLoaded, true);
      calls.push(after);
    };
    assert.equal(await context.loadBackendSettingsState(), true);
    assert.equal(calls.length, 1);
    assert.deepEqual({ ...calls[0] }, next);
    assert.equal(previous.modelCatalogFingerprint, "before");
  });

  it("loads settings when the catalog shard is absent and does not publish stale or invalid settings", async () => {
    const absent = settingsRuntime();
    assert.equal(await absent.context.loadBackendSettingsState(), true);
    for (const scenario of ["stale", "invalid"]) {
      const { context, previous, next } = settingsRuntime();
      context.refreshCodexModelCatalogForSettings = () => { assert.fail("must not refresh"); };
      context.postJson = async () => {
        if (scenario === "stale") context.codexPlusBackendSettingsSeq++;
        return scenario === "invalid" ? {} : next;
      };
      assert.equal(await context.loadBackendSettingsState(), false);
      assert.equal(context.codexPlusBackendSettings, previous);
      assert.equal(context.codexPlusBackendSettingsLoaded, false);
    }
  });

  it("reloads the actual catalog and only invalidates local model queries without selecting a thread model", async () => {
    const { context, previous, next } = settingsRuntime();
    const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
    const refresh = catalog.slice(catalog.indexOf("  function codexModelSettingsSource("),
      catalog.indexOf("  function codexPlusModelMetadata("));
    const selected = { threadId: "a", model: "saved-outside-catalog", reasoningEffort: "ultra",
      onSelectModel: () => assert.fail("catalog refresh must not select a model") };
    const runtime = { catalog: { status: "ok", models: ["old-model"] }, source: "", settingsKey: "",
      loadedAt: 1, retryAt: 0, failure: null };
    const queries = [
      ["models", "local"], ["models", "remote"], ["thread", "local", "a"], ["thread", "local", "b"],
    ];
    const invalidated: any[] = [];
    const calls: string[] = [];
    Object.assign(context, {
      codexModelCatalogRuntime: runtime, codexModelCatalog: runtime.catalog, codexModelCatalogLoadedAt: 1,
      codexModelCatalogPromise: null, codexModelCatalogRevision: 0, selected,
      readCodexAppServerPreparation: async (route: string) => {
        calls.push(route);
        assert.equal(route, "/codex-model-catalog");
        return { status: "ok", model: "new-default", default_model: "new-default", models: ["new-default"] };
      },
      findCodexQueryClient: () => ({
        getQueryCache: () => ({ getAll: () => queries.map(queryKey => ({ queryKey, state: { data: selected } })) }),
        invalidateQueries: async (options: any) => { invalidated.push(options.queryKey); },
      }),
      codexPlusModelPickerCatalogChanged() {}, renderCodexPlusMenu() {}, scheduleCodexModelWhitelistRefresh() {},
      syncCodexPlusGroupModelQueries() {},
      sendCodexPlusDiagnostic() {},
    });
    context.window.__codexPlusModelCatalogRuntime = runtime;
    vm.runInContext(refresh, context);
    const original = { ...selected };
    assert.equal(await context.loadBackendSettingsState(), true);
    await context.loadCodexModelCatalog();
    assert.deepEqual(calls, ["/codex-model-catalog"]);
    assert.deepEqual(invalidated, [["models", "local"]]);
    assert.equal(context.codexModelCatalog.default_model, "new-default");
    assert.deepEqual(selected, original);
    assert.equal(previous.modelCatalogFingerprint, "before");
    assert.equal(context.codexPlusBackendSettings.modelCatalogFingerprint, next.modelCatalogFingerprint);
  });
});

describe("native default model fallback in pure API filtering", () => {
  const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
  const descriptor = catalog.slice(catalog.indexOf("  function codexPlusModelDescriptor("),
    catalog.indexOf("  function modelArrayLooksPatchable("));
  const defaults = catalog.slice(catalog.indexOf("  function patchModelDefaults("),
    catalog.indexOf("  // 只收窄当前纯 API"));
  it("retains upstream replacement for unsupported new-thread defaults without changing the thread model", () => {
    const snapshots = new WeakMap();
    const patch = new Function("codexModelCatalog", "codexPlusModelDefaultSnapshots", "modelFilterPolicy",
      "codexPlusModelMetadata", "modelReasoningEfforts", `${descriptor}\n${defaults}\nreturn patchModelDefaults;`)(
      { model: "gpt-6.1-sol", default_model: "gpt-6.1-sol" }, snapshots, () => "pureApi:local",
      () => null, () => []);
    const original = { model: "gpt-6.1-terra", defaultReasoningEffort: "xhigh" };
    const host: any = { defaultModel: original, default_model: "gpt-6.1-terra", model: "saved-choice" };
    assert.equal(patch(host, new Set(["gpt-6.1-sol"])), true);
    assert.equal(host.defaultModel.model, "gpt-6.1-sol");
    assert.equal(host.default_model, "gpt-6.1-sol");
    assert.equal(host.model, "saved-choice");
    const missing = {};
    patch(missing, new Set(["gpt-6.1-sol"]));
    assert.deepEqual(missing, {});

    const oldPatched = { model: "gpt-6.1-sol", defaultReasoningEffort: "medium" };
    host.defaultModel = oldPatched;
    snapshots.set(host, { policy: "old", fields: { defaultModel: { original, patched: oldPatched } } });
    assert.equal(patch(host, null), true);
    assert.equal(host.defaultModel, original);
    host.defaultModel = { model: "new-user-choice" };
    const userChoice = host.defaultModel;
    snapshots.set(host, { policy: "old", fields: { defaultModel: { original, patched: oldPatched } } });
    patch(host, null);
    assert.equal(host.defaultModel, userChoice);
  });
});
