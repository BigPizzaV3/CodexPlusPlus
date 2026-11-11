import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

const catalog = readFileSync(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
const helpers = catalog.slice(catalog.indexOf("  function codexPerModelContextEnabled("),
  catalog.indexOf("  function patchAppServerModelRequestClient("));
const service = readFileSync(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const reload = service.slice(service.indexOf("  function codexThreadReloadParams("),
  service.indexOf("  async function reloadCodexManagedSessionConnection("));

function runtime() {
  const calls: { method: string; params: any; options: any }[] = [];
  const state = {
    relayMode: "aggregate", profileEnabled: true, hostId: "local", status: "idle", turns: 1, fail: "", unloaded: false, ephemeral: false,
    runtime: { status: "ok", models: {
      selected: { model_context_window: 131072, model_auto_compact_token_limit: 117964 },
      other: { model_context_window: 65536, model_auto_compact_token_limit: 50000 },
    } } as any,
  };
  const snapshot = {
    thread: { id: "thread", status: { type: "idle" } }, model: "selected", modelProvider: "fixture",
    cwd: "C:/project", approvalPolicy: "never", approvalsReviewer: "user", reasoningEffort: "high",
    runtimeWorkspaceRoots: ["C:/project", "C:/data"], serviceTier: "default",
    sandbox: { type: "workspaceWrite", writableRoots: ["C:/data"], networkAccess: true,
      excludeTmpdirEnvVar: true, excludeSlashTmp: false },
  };
  const send = async (method: string, params: any, options: any) => {
    calls.push({ method, params, options });
    if (state.fail === method && (method !== "thread/resume" || params.config)) throw new Error("fixture failure");
    if (method === "thread/read") return { thread: { ephemeral: state.ephemeral, status: { type: state.unloaded ? "notLoaded" : state.status } } };
    if (method === "thread/turns/list") return { data: state.turns ? [{}] : [] };
    if (method === "thread/unsubscribe") state.unloaded = true;
    if (method === "thread/resume") { state.unloaded = false; return snapshot; }
    return {};
  };
  const context = vm.createContext({
    window: { __codexSessionDeleteBridge: {} }, Map, Set,
    codexRemoteSessionActiveProfile: () => state.profileEnabled ? { relayMode: state.relayMode } : null,
    codexModelCatalog: { get runtimeContext() { return state.runtime; } },
    loadCodexModelCatalog: async () => ({ model: "selected", runtimeContext: state.runtime }),
    codexManagedSessionRecoveryOptions: () => ({ timeoutMs: 5000 }),
    applyCodexRemoteSessionProviderOverride: (_method: string, params: any) => params,
    applyCodexManagedSessionConnection: (_method: string, params: any) => params,
    modelResponseHostId: (params: any, fallback: string) => params?.hostId || fallback,
    sendCodexPlusDiagnostic() {},
  });
  vm.runInContext(reload + helpers
    + "\nglobalThis.api = { prepare: prepareCodexThreadContextStart, note: noteCodexThreadRuntimeContext, refresh: refreshCodexThreadModelBeforeTurn };", context);
  const api = context.api;
  const client = { hostId: "local" };
  const turn = { threadId: "thread", collaborationMode: { settings: { model: "selected", reasoning_effort: "max" } } };
  const refresh = () => api.refresh(client, send, "turn/start", turn, { clientUserMessageId: "user-message" });
  return { api, context, calls, state, snapshot, send, client, turn, refresh };
}

describe("runtime context refresh", () => {
  it("configures a fresh thread before its first turn without unloading it", async () => {
    const env = runtime();
    const params = await env.api.prepare("thread/start", { model: "selected", config: { unrelated: true } }, "local");
    assert.equal(params.config.unrelated, true);
    assert.equal(params.config.model_context_window, 131072);
    env.api.note("thread/start", params, { thread: { id: "thread" } }, "local");
    env.state.turns = 0;
    assert.equal(await env.refresh(), null);
    assert.equal(env.calls.length, 0);
  });

  it("applies same-model increases, decreases and compact-only changes once", async () => {
    const env = runtime();
    for (const [window, compact] of [[131072, 117964], [262144, 200000], [65536, 50000], [65536, 40000]]) {
      env.state.runtime.models.selected = { model_context_window: window, model_auto_compact_token_limit: compact };
      env.calls.length = 0;
      assert.equal(await env.refresh(), true);
      assert.deepEqual(env.calls.map(call => call.method),
        ["thread/read", "thread/turns/list", "thread/resume", "thread/unsubscribe", "thread/resume"]);
      assert.equal(env.calls.at(-1)!.params.config.model_context_window, window);
      assert.equal(env.calls.at(-1)!.params.config.model_auto_compact_token_limit, compact);
      env.calls.length = 0;
      assert.equal(await env.refresh(), null);
      assert.equal(env.calls.length, 0);
    }
  });

  it("uses collaboration-mode model selection and preserves permissions and workspace settings", async () => {
    const env = runtime();
    env.turn.collaborationMode.settings.model = "other";
    await env.refresh();
    const params = env.calls.at(-1)!.params;
    assert.equal(params.model, "other");
    assert.equal(params.modelProvider, "fixture");
    assert.equal(params.config.model_reasoning_effort, "max");
    assert.equal(params.config.model_context_window, 65536);
    assert.deepEqual(params.runtimeWorkspaceRoots, env.snapshot.runtimeWorkspaceRoots);
    assert.equal(params.cwd, env.snapshot.cwd);
    assert.equal(params.approvalPolicy, "never");
    assert.equal(params.serviceTier, "default");
    assert.equal(params.sandbox, "workspace-write");
    assert.deepEqual(Array.from(params.config.sandbox_workspace_write.writable_roots), ["C:/data"]);
    assert.equal(params.config.sandbox_workspace_write.network_access, true);
    assert.equal(env.calls[0].options.clientUserMessageId, undefined);
  });

  it("never unloads busy or fresh threads and releases its lock after errors", async () => {
    const env = runtime();
    env.state.status = "active";
    await assert.rejects(env.refresh(), /当前回复/);
    assert.deepEqual(env.calls.map(call => call.method), ["thread/read"]);
    env.state.status = "idle";
    env.state.turns = 0;
    env.calls.length = 0;
    await assert.rejects(env.refresh(), /尚无历史/);
    assert.equal(env.calls.some(call => call.method === "thread/unsubscribe"), false);
    env.state.turns = 1;
    await env.refresh();
  });

  // #3041：侧边聊天是 ephemeral 分叉，没有历史也不能卸载恢复；不报「尚无历史」，也不重建。
  it("skips ephemeral side-chat forks instead of rejecting them", async () => {
    const env = runtime();
    env.state.ephemeral = true;
    env.state.turns = 0;
    assert.equal(await env.refresh(), null);
    assert.deepEqual(env.calls.map(call => call.method), ["thread/read"]);
    env.calls.length = 0;
    assert.equal(await env.refresh(), null);
    assert.deepEqual(env.calls.map(call => call.method), [], "the fork's signature is remembered");
  });

  it("does not remember failed restores and retries on the next submission", async () => {
    const env = runtime();
    env.state.fail = "thread/resume";
    await assert.rejects(env.refresh(), /fixture failure/);
    assert.equal(env.context.window.__codexPlusThreadContexts.size, 0);
    env.state.fail = "";
    assert.equal(await env.refresh(), true);
  });

  it("propagates native unsubscribe errors before restoring or sending a turn", async () => {
    const env = runtime();
    env.state.fail = "thread/unsubscribe";
    await assert.rejects(env.refresh(), /fixture failure/);
    assert.equal(env.calls.some(call => call.method === "thread/resume" && call.params.config), false);
    assert.equal(env.context.window.__codexPlusThreadContexts.size, 0);
  });

  it("refreshes a worker's managed catalog when its provider write switch is disabled", async () => {
    const env = runtime();
    env.state.profileEnabled = false;
    const params = await env.api.prepare("thread/start", { model: "selected" }, "local");
    assert.equal(params.config.model_context_window, 131072);
    assert.equal(await env.refresh(), true);
    assert.equal(env.calls.at(-1)!.params.config.model_context_window, 131072);
  });

  it("leaves remote, official and external catalogs alone", async () => {
    const env = runtime();
    env.client.hostId = "remote";
    assert.equal(await env.refresh(), null);
    env.client.hostId = "local";
    env.state.relayMode = "official";
    env.state.runtime = null;
    assert.equal(await env.refresh(), null);
    env.state.relayMode = "aggregate";
    env.state.runtime = null;
    assert.equal(await env.refresh(), null);
    assert.equal(env.calls.length, 0);
  });

  it("surfaces catalog failures and old fixed ceilings before sending anything", async () => {
    const env = runtime();
    for (const status of ["failed", "unsupported"]) {
      env.state.runtime = { status, message: "fixture catalog error" };
      await assert.rejects(env.refresh(), /fixture catalog error/);
    }
    assert.equal(env.calls.length, 0);
  });
});
