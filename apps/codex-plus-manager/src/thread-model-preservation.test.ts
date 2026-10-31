import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL(
  "../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const override = source.slice(0, source.indexOf("  function codexManagedSessionConnection("));

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
  const create = new Function("codexPlusBackendSettings", "codexModelCatalog", "sendCodexPlusDiagnostic",
    "postJson", "codexAppServerPreparationTimeoutMs", "window", `${override}\n${readSource}
      return { prepare: prepareCodexPureApiResume, apply: applyCodexRemoteSessionProviderOverride,
        verify: verifyCodexPureApiResumeResult };`);
  const api = create(settings, {}, (event: string, detail: any) => diagnostics.push({ event, detail }), async (route: string, payload: any) => {
    assert.equal(route, "/thread-resume-model-metadata");
    calls.push(payload);
    return read ? read(payload) : { status: "ok", threadId: payload.thread_id, model: "saved-model", reasoningEffort: "high" };
  }, 30, {});
  return { ...api, settings, calls, diagnostics };
}

describe("pure API persisted model snapshot", () => {
  it("restores each saved model and effort with the current provider without changing caller objects", async () => {
    const api = snapshotRuntime(async ({ thread_id }) => ({ status: "ok", threadId: thread_id,
      model: `saved-${thread_id}`, reasoningEffort: thread_id === "a" ? "ultra" : "low" }));
    for (const sync of [false, true]) {
      api.settings.providerSyncEnabled = sync;
      for (const threadId of ["a", "b"]) {
        const params = { threadId, config: { model_context_window: 500000 } };
        const patched = await api.prepare("thread/resume", params, "local");
        assert.equal(patched.model, `saved-${threadId}`);
        assert.equal(patched.modelProvider, "custom");
        assert.equal(patched.config.model_reasoning_effort, threadId === "a" ? "ultra" : "low");
        assert.equal(patched.config.model_context_window, 500000);
        assert.deepEqual(params, { threadId, config: { model_context_window: 500000 } });
      }
    }
  });

  it("preserves explicit reasoning overrides and native semantics for an unspecified saved effort", async () => {
    const api = snapshotRuntime();
    const patched = await api.prepare("thread/resume",
      { threadId: "a", config: { model_reasoning_effort: "low" } }, "local");
    assert.equal(patched.config.model_reasoning_effort, "low");
    const unspecified = snapshotRuntime(async ({ thread_id }) =>
      ({ status: "ok", threadId: thread_id, model: "saved-model", reasoningEffort: null }));
    const result = await unspecified.prepare("thread/resume", { threadId: "a" }, "local");
    assert.equal(result.model, "saved-model");
    assert.equal(Object.hasOwn(result.config, "model_reasoning_effort"), false);
  });

  it("never probes remote hosts, official profiles, explicit models or foreign providers", async () => {
    const api = snapshotRuntime(async () => { throw new Error("must not read"); });
    for (const host of ["remote", "", undefined]) {
      const params = { threadId: "a" };
      assert.equal(await api.prepare("thread/resume", params, host), params);
    }
    for (const params of [
      { threadId: "a", model: "chosen" }, { threadId: "a", modelProvider: "foreign" },
      { threadId: "a", model_provider: "foreign" },
      { hostId: "remote", method: "thread/resume", params: { threadId: "a" } },
    ]) {
      assert.equal(await api.prepare("method" in params ? "send-cli-request-for-host" : "thread/resume", params, "local"), params);
    }
    const configured = await api.prepare("thread/resume", { threadId: "a", config: { model: "explicit" } }, "local");
    assert.equal(configured.model, "explicit");
    assert.equal(configured.modelProvider, "custom");
    api.settings.relayProfiles[0].relayMode = "official";
    const params = { threadId: "a" };
    assert.equal(await api.prepare("thread/resume", params, "local"), params);
    assert.equal(api.calls.length, 0);
  });

  it("passes OpenAI session identity resumes through unchanged without auxiliary reads", async () => {
    for (const sync of [false, true]) {
      for (const refreshFailed of [false, true]) {
        const api = snapshotRuntime(async () => { throw new Error("must not read"); });
        api.settings.providerSyncEnabled = sync;
        api.settings.relayProfiles[0].configContents = 'model_provider = "openai"';
        for (const optional of [{}, { model: null }, { model_provider: "openai" }, { config: { model: "chosen" } }]) {
          const params = { threadId: "a", ...optional };
          const routed = api.apply("thread/resume", params);
          const result = await api.prepare("thread/resume", routed, "local", refreshFailed,
            async () => { throw new Error("must not read state"); });
          assert.equal(result, params);
        }
        assert.equal(api.calls.length, 0);
        assert.equal(api.diagnostics.length, 0);
      }
    }
  });

  it("restores persisted choices through provider-only and empty optional fields in the actual routing order", async () => {
    for (const optional of [
      { model: null }, { model: "" }, { model: " " }, { model: undefined },
      { modelProvider: "openai" }, { model_provider: "openai" },
      { modelProvider: "custom" }, { modelProvider: null }, { model_provider: "" },
      { config: { model: null, model_reasoning_effort: null } },
      { config: { model: "", model_reasoning_effort: "" } },
    ] as Record<string, any>[]) {
      const api = snapshotRuntime(async ({ thread_id }) =>
        ({ status: "ok", threadId: thread_id, model: "gpt-6.1-terra", reasoningEffort: "xhigh" }));
      const params: Record<string, any> = { threadId: "a", ...optional };
      const routed = api.apply("thread/resume", params);
      const result = await api.prepare("thread/resume", routed, "local");
      assert.equal(api.calls.length, 1, JSON.stringify(optional));
      assert.equal(result.model, "gpt-6.1-terra");
      assert.equal(result.modelProvider, "custom");
      assert.equal(result.config.model_reasoning_effort, "xhigh");
      assert.equal(Object.hasOwn(result, "model_provider"), false);
      assert.equal(Object.hasOwn(result.config, "model"), false);
      assert.equal(Object.hasOwn(params, "config") ? params.config : undefined, optional.config);
    }
  });

  it("leaves new-thread model fallback with native defaults", async () => {
    const api = snapshotRuntime();
    const params = api.apply("thread/start", {});
    const prepared = await api.prepare("thread/start", params, "local");
    assert.equal(prepared.modelProvider, "custom");
    assert.equal(Object.hasOwn(prepared, "model"), false);
    assert.equal(api.calls.length, 0);
  });

  it("falls back to the current provider without a model for incomplete, conflicting or failed metadata", async () => {
    for (const data of [
      null, { status: "failed" }, { status: "failed", error: "conflicting thread models" },
      { status: "ok", threadId: "other", model: "saved", reasoningEffort: "high" },
      { status: "ok", threadId: "a", model: null, reasoningEffort: "high" },
      { status: "ok", threadId: "a", model: "", reasoningEffort: "high" },
      { status: "ok", threadId: "a", model: "saved" },
      { status: "ok", threadId: "a", model: "saved", reasoningEffort: "" },
    ]) {
      const api = snapshotRuntime(async () => data);
      const params = { threadId: "a", model: null, model_provider: "openai",
        config: { model: "", model_reasoning_effort: "low", model_context_window: 500000 } };
      const original = structuredClone(params);
      const result = await api.prepare("thread/resume", params, "local");
      assert.equal(result.modelProvider, "custom");
      assert.equal(Object.hasOwn(result, "model"), false);
      assert.equal(Object.hasOwn(result, "model_provider"), false);
      assert.equal(Object.hasOwn(result.config, "model"), false);
      assert.equal(result.config.model_reasoning_effort, "low");
      assert.equal(result.config.model_context_window, 500000);
      assert.deepEqual(params, original);
      assert.deepEqual(api.diagnostics, [{ event: "pure_api_resume_model_fallback",
        detail: { reason: "metadata_unavailable" } }]);
      api.verify("thread/resume", result,
        { model: "native-default", modelProvider: "custom", reasoningEffort: "low" }, "local");
      api.verify("thread/resume", result,
        { model: "native-default", modelProvider: "openai", reasoningEffort: "low" }, "local");
      assert.deepEqual(api.diagnostics.at(-1), { event: "pure_api_resume_result_mismatch",
        detail: { modelMismatch: false, providerMismatch: true, reasoningEffortMismatch: false } });
    }
    const failed = snapshotRuntime(async () => { throw new Error("SQLite unavailable"); });
    const result = await failed.prepare("thread/resume", { threadId: "a" }, "local");
    assert.equal(result.modelProvider, "custom");
    assert.equal(Object.hasOwn(result, "model"), false);
    assert.equal(Object.hasOwn(result.config, "model_reasoning_effort"), false);
    assert.equal(failed.calls.length, 1);
  });

  it("uses the current provider after refresh failure without reading or injecting saved defaults", async () => {
    const api = snapshotRuntime(async () => { throw new Error("must not read"); });
    api.settings.relayProfiles[0].configContents = 'model_provider = "deepseek"';
    api.settings.activeRelayCodexProvider = "stale-provider";
    const result = await api.prepare("thread/resume",
      { threadId: "a", model: null, config: { model: null, model_reasoning_effort: null } }, "local", true);
    assert.equal(result.modelProvider, "deepseek");
    assert.equal(Object.hasOwn(result, "model"), false);
    assert.equal(Object.hasOwn(result.config, "model"), false);
    assert.equal(Object.hasOwn(result.config, "model_reasoning_effort"), false);
    assert.equal(api.calls.length, 0);
    assert.deepEqual(api.diagnostics, [{ event: "pure_api_resume_model_fallback",
      detail: { reason: "provider_refresh_failed" } }]);
    const explicit = await api.prepare("thread/resume",
      { threadId: "a", config: { model: "chosen" } }, "local", true);
    assert.equal(explicit.model, "chosen");
    assert.equal(explicit.modelProvider, "deepseek");
  });

  it("bounds read waits, falls back and ignores late snapshots", async () => {
    let finish!: (value: any) => void;
    const api = snapshotRuntime(() => new Promise(resolve => { finish = resolve; }));
    const params = { threadId: "a" };
    const result = await api.prepare("thread/resume", params, "local");
    assert.equal(result.modelProvider, "custom");
    assert.equal(Object.hasOwn(result, "model"), false);
    finish({ status: "ok", threadId: "a", model: "late", reasoningEffort: "high" });
    await Promise.resolve();
    assert.deepEqual(params, { threadId: "a" });
    assert.equal(Object.hasOwn(result, "model"), false);
    assert.equal(api.diagnostics.filter((item: any) => item.event === "pure_api_resume_model_fallback").length, 1);
  });

  it("rejects a provider change while reading the saved model", async () => {
    const api = snapshotRuntime(async ({ thread_id }) => {
      api.settings.relayProfiles[0].configContents = 'model_provider = "changed"';
      return { status: "ok", threadId: thread_id, model: "saved", reasoningEffort: "high" };
    });
    await assert.rejects(api.prepare("thread/resume", { threadId: "a" }, "local"), /配置已变化/);
  });

  it("rejects model or config changes during the metadata read", async () => {
    for (const key of ["model", "config"]) {
      const params: any = { threadId: "a" };
      const api = snapshotRuntime(async ({ thread_id }) => {
        params[key] = key === "model" ? "new-choice" : { model_reasoning_effort: "low" };
        return { status: "ok", threadId: thread_id, model: "saved", reasoningEffort: "high" };
      });
      await assert.rejects(api.prepare("thread/resume", params, "local"), /设置已变化/);
    }
  });

  it("does not downgrade concurrent setting changes into an availability fallback", async () => {
    for (const changed of ["provider", "model"]) {
      const params: any = { threadId: "a" };
      const api = snapshotRuntime(async () => {
        if (changed === "provider") api.settings.relayProfiles[0].configContents = 'model_provider = "changed"';
        else params.model = "new-choice";
        throw new Error("metadata unavailable");
      });
      await assert.rejects(api.prepare("thread/resume", params, "local"), /已变化/);
      assert.equal(api.diagnostics.length, 0);
    }
  });

  it("keeps loaded-provider guards on fallback resumes", async () => {
    const api = snapshotRuntime(async () => null);
    await assert.rejects(api.prepare("thread/resume", { threadId: "a" }, "local", false,
      async () => ({ thread: { modelProvider: "openai", status: { type: "idle" } } })), /旧供应商连接/);
    assert.equal(api.diagnostics.length, 0);
  });

  it("rejects confirmed loaded old-provider sessions and only diagnoses restored setting differences", async () => {
    for (const status of ["notLoaded", "idle", "active"]) {
      const api = snapshotRuntime();
      const calls: string[] = [];
      const rpc = async (method: string) => {
        calls.push(method);
        return { thread: { id: "a", modelProvider: "openai", status: { type: status } } };
      };
      if (status !== "notLoaded") {
        await assert.rejects(api.prepare("thread/resume", { threadId: "a" }, "local", false, rpc), /旧供应商连接/);
        assert.deepEqual(calls, ["thread/read"]);
        continue;
      }
      const patched = await api.prepare("thread/resume", { threadId: "a" }, "local", false, rpc);
      assert.deepEqual(calls, ["thread/read"]);
      api.verify("thread/resume", patched, { model: "saved-model", modelProvider: "custom", reasoningEffort: "high" }, "local");
      for (const result of [
        { model: "saved-model", modelProvider: "openai" }, { model: "global", modelProvider: "custom" },
        { model: "saved-model", modelProvider: "custom", reasoningEffort: "low" },
      ]) {
        const original = structuredClone(result);
        api.verify("thread/resume", patched, result, "local");
        assert.deepEqual(result, original);
        assert.equal(api.diagnostics.at(-1).event, "pure_api_resume_result_mismatch");
      }
      assert.equal(api.diagnostics.length, 3);
    }
  });

  it("falls back after failed, timed-out or unknown state reads without injecting saved effort", async () => {
    const rpcs = [
      async () => { throw new Error("read unavailable"); },
      async () => new Promise(() => {}),
      async () => null,
      async () => ({ thread: { modelProvider: "custom", status: { type: "unknown" } } }),
    ];
    for (const rpc of rpcs) {
      for (const effort of [undefined, "low"]) {
        const api = snapshotRuntime();
        const params = { threadId: "a", config: { model_context_window: 500000,
          ...(effort ? { model_reasoning_effort: effort } : {}) } };
        const original = structuredClone(params);
        const result = await api.prepare("thread/resume", params, "local", false, rpc);
        assert.equal(result.modelProvider, "custom");
        assert.equal(Object.hasOwn(result, "model"), false);
        assert.equal(result.config.model_reasoning_effort, effort);
        assert.equal(result.config.model_context_window, 500000);
        assert.deepEqual(params, original);
        assert.deepEqual(api.diagnostics, [{ event: "pure_api_resume_model_fallback",
          detail: { reason: "thread_state_unavailable" } }]);
      }
    }
  });

  it("ignores late state reads after fallback and still rejects concurrent changes on read errors", async () => {
    let finish!: (value: any) => void;
    const api = snapshotRuntime();
    const result = await api.prepare("thread/resume", { threadId: "a" }, "local", false,
      () => new Promise(resolve => { finish = resolve; }));
    finish({ thread: { modelProvider: "openai", status: { type: "idle" } } });
    await Promise.resolve();
    assert.equal(Object.hasOwn(result, "model"), false);
    assert.equal(api.diagnostics.length, 1);
    for (const changed of ["provider", "model"]) {
      const current = snapshotRuntime();
      const params: any = { threadId: "a" };
      await assert.rejects(current.prepare("thread/resume", params, "local", false, async () => {
        if (changed === "provider") current.settings.relayProfiles[0].configContents = 'model_provider = "changed"';
        else params.model = "new-choice";
        throw new Error("read unavailable");
      }), /已变化/);
      assert.equal(current.diagnostics.length, 0);
    }
  });

  it("prepares nested local resumes without changing the envelope", async () => {
    const api = snapshotRuntime();
    const params = { hostId: "local", method: "thread/resume", params: { threadId: "a" }, trace: "keep" };
    const result = await api.prepare("send-cli-request-for-host", params, "local");
    assert.equal(result.params.model, "saved-model");
    assert.equal(result.params.modelProvider, "custom");
    assert.equal(result.trace, "keep");
    assert.deepEqual(params.params, { threadId: "a" });
  });

  it("rejects a changed host envelope before passing a local snapshot to another host", async () => {
    const params = { hostId: "local", method: "thread/resume", params: { threadId: "a" } };
    const api = snapshotRuntime(async ({ thread_id }) => {
      params.hostId = "remote";
      return { status: "ok", threadId: thread_id, model: "saved", reasoningEffort: "high" };
    });
    const calls: any[] = [];
    await assert.rejects(api.prepare("send-cli-request-for-host", params, "local", false,
      async (...args: any[]) => { calls.push(args); }), /主机已变化/);
    assert.equal(calls.length, 0);
  });

  it("uses the snapshot in instance, prototype and host-envelope wrappers with exactly one native resume", async () => {
    const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
    const instance = catalog.slice(catalog.indexOf("  function patchAppServerModelRequestClient("), catalog.indexOf("  // issue #2177"));
    const prototype = catalog.slice(catalog.indexOf("    function installCodexAppServerClientPrototypePatch()"),
      catalog.indexOf("  const appServerModelRequestPatchMaxMisses"));
    const version = readFileSync(new URL("../../../assets/inject/renderer-inject/00-prelude.js", import.meta.url), "utf8")
      .match(/const codexAppServerModelRequestPatchVersion = "([^"]+)"/)?.[1];
    for (const [mode, scenario, optional] of ["instance", "prototype", "envelope"].flatMap(mode =>
      ["snapshot", "missing", "read-failed", "refresh-failed", "openai", "state-failed",
        "state-timeout", "state-unknown", "result-mismatch"].flatMap(scenario =>
        [{}, { model: null }, { modelProvider: "openai" },
          { config: { model: null, model_reasoning_effort: null } }].map(optional => [mode, scenario, optional] as const)))) {
      const calls: any[] = [];
      const diagnostics: string[] = [];
      let metadataReads = 0;
      class Client {
        hostId = "local";
        async sendRequest(method: string, params: any) {
          calls.push({ method, params });
          const innerMethod = method === "send-cli-request-for-host" ? params.method : method;
          const innerParams = method === "send-cli-request-for-host" ? params.params : params;
          if (innerMethod === "thread/read") {
            if (scenario === "state-failed") throw new Error("read unavailable");
            if (scenario === "state-timeout") return new Promise(() => {});
            if (scenario === "state-unknown") return { thread: { status: { type: "unknown" } } };
            return { thread: { id: "a", modelProvider: "openai", status: { type: "notLoaded" } } };
          }
          if (scenario === "result-mismatch") {
            return { model: "normalized-model", modelProvider: "normalized-provider",
              reasoningEffort: "low", thread: { id: "a" } };
          }
          return { model: innerParams.model || "native-default", modelProvider: innerParams.modelProvider,
            reasoningEffort: innerParams.config?.model_reasoning_effort, thread: { id: "a" } };
        }
      }
      const client = new Client();
      const context = vm.createContext({
        window: { __codexPlusAppServerClientClass: Client, __codexSessionDeleteBridge: () => {} }, client,
        codexAppServerModelRequestPatchVersion: version,
        codexPlusBackendSettings: { relayProfilesEnabled: true, activeRelayId: "relay", providerSyncEnabled: false,
          relayProfiles: [{ id: "relay", relayMode: "pureApi",
            configContents: `model_provider = "${scenario === "openai" ? "openai" : "custom"}"` }] },
        codexModelCatalog: {}, codexPlusBackendSettingsLoaded: true,
        codexAppServerPreparationTimeoutMs: 30,
        postJson: async (_route: string, payload: any) => {
          metadataReads++;
          if (scenario === "missing") return { status: "failed" };
          if (scenario === "read-failed") throw new Error("metadata unavailable");
          return { status: "ok", threadId: payload.thread_id, model: "saved-outside-catalog", reasoningEffort: "ultra" };
        },
        registerNativeHostClient() {}, sendCodexPlusDiagnostic(event: string) { diagnostics.push(event); },
        codexPlusPluginNativeInterceptClient: () => null,
        appServerModelRequestMethod: (method: string, params: any) => method === "send-cli-request-for-host" ? params.method : method,
        loadBackendSettingsState: async () => scenario !== "refresh-failed", loadCodexModelCatalog: async () => ({}),
        prepareCodexManagedSessionConnection: async (_method: string, params: any) => params,
        codexManagedSessionConnection: () => null, refreshCodexThreadModelBeforeTurn: async () => null,
        sendCodexManagedSessionTurn: async () => null, noteCodexManagedSessionStart() {},
        codexThreadModelRequestState: () => ({}), codexPlusModelUnlockEnabled: () => false,
        codexPluginMarketplacePatchEnabled: () => false, codexPlusSettings: () => ({}),
        setTimeout, clearTimeout,
      });
      vm.runInContext(override + readSource + instance + prototype, context);
      vm.runInContext(mode === "prototype" ? "installCodexAppServerClientPrototypePatch()" : "patchAppServerModelRequestClient(client)", context);
      const params = { threadId: "a", ...optional };
      const result = await client.sendRequest(mode === "envelope" ? "send-cli-request-for-host" : "thread/resume",
        mode === "envelope" ? { hostId: "local", method: "thread/resume", params } : params);
      const resumes = calls.filter(call => (mode === "envelope" ? call.params.method : call.method) === "thread/resume");
      assert.equal(resumes.length, 1);
      assert.equal(calls.length, scenario === "openai" ? 1 : 2);
      const received = mode === "envelope" ? resumes[0].params.params : resumes[0].params;
      if (scenario === "openai") {
        assert.deepEqual(received, params);
        assert.equal(metadataReads, 0);
        assert.equal(diagnostics.includes("pure_api_resume_model_fallback"), false);
        assert.equal(diagnostics.includes("pure_api_resume_result_mismatch"), false);
        continue;
      }
      assert.equal(received.modelProvider, "custom");
      assert.equal(metadataReads, scenario === "refresh-failed" ? 0 : 1);
      if (scenario === "snapshot" || scenario === "result-mismatch") {
        assert.equal(received.model, "saved-outside-catalog");
        assert.equal(received.config.model_reasoning_effort, "ultra");
        assert.equal(diagnostics.includes("pure_api_resume_model_fallback"), false);
        assert.equal(diagnostics.includes("pure_api_resume_result_mismatch"), scenario === "result-mismatch");
        if (scenario === "result-mismatch") assert.equal(result.model, "normalized-model");
      } else {
        assert.equal(Object.hasOwn(received, "model"), false);
        assert.equal(Object.hasOwn(received.config, "model"), false);
        assert.equal(Object.hasOwn(received.config, "model_reasoning_effort"), false);
        assert.equal(diagnostics.filter(event => event === "pure_api_resume_model_fallback").length, 1);
      }
    }
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
