const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

const root = `${__dirname}/renderer-inject`;
const adapter = readFileSync(`${root}/62-plugin-market-adapter.js`, "utf8");
const model = readFileSync(`${root}/70-model-catalog.js`, "utf8");
const native = readFileSync(`${root}/60-plugin-marketplace.js`, "utf8");
const methodFunction = model.slice(model.indexOf("  function appServerModelRequestMethod("), model.indexOf("  function patchAppServerModelResult("));
const clientPatch = native.slice(native.indexOf("  function patchPluginMarketplaceRequestClient("), native.indexOf("  function patchPluginMarketplaceRequestMessage("));
const transportPatch = native.slice(native.indexOf("  function installPluginMarketplaceBridgePatch("), native.indexOf("  const pluginMarketplaceRequestPatchMaxMisses"));
const prototypePatch = model.slice(model.indexOf("    function installCodexAppServerClientPrototypePatch("), model.indexOf("  const appServerModelRequestPatchMaxMisses"));
const flush = async () => { await new Promise((resolve) => setImmediate(resolve)); await new Promise((resolve) => setImmediate(resolve)); };
const sentinel = (source = "public") => `/fixture/.tmp/codex-plus-plugin-market/virtual-${source}/.agents/plugins/marketplace.json`;
const managedSummary = { name: "sample", id: "sample@codex-plus-plugin-market-123456789abc", codexPlusIndexId: "original-id", installed: false };
const catalog = () => ({ marketplaces: [{ name: "codex-plus-index-public", path: sentinel(), plugins: [{ ...managedSummary }] }], marketplaceLoadErrors: [], featuredPluginIds: [] });
const official = () => ({ marketplaces: [{ name: "openai-bundled", path: "/fixture/bundled.json", plugins: [{ name: "system", id: "system@openai-bundled" }] }], marketplaceLoadErrors: [], featuredPluginIds: ["system@openai-bundled"] });
const installResult = () => ({ appsNeedingAuth: [], authPolicy: "ON_USE" });

function fixture(handler, { enabled = true } = {}) {
  const calls = [], originalMessages = [], delivered = [], toasts = [], handlers = new Map(), timers = new Map();
  let timerId = 0;
  const window = {
    location: { origin: "app://-" },
    addEventListener(type, callback) { const list = handlers.get(type) || []; list.push(callback); handlers.set(type, list); },
    removeEventListener(type, callback) { handlers.set(type, (handlers.get(type) || []).filter((item) => item !== callback)); },
    dispatchEvent(event) {
      for (const callback of handlers.get(event.type) || []) { callback(event); if (event.stopped) break; }
      if (!event.stopped) delivered.push(event);
      return true;
    },
    electronBridge: { sendMessageFromView: async (message) => { originalMessages.push(message); } },
  };
  class MessageEvent { constructor(type, init) { Object.assign(this, { type }, init); } stopImmediatePropagation() { this.stopped = true; } }
  class NativeClient { constructor(hostId = "local") { this.hostId = hostId; this.calls = []; } async sendRequest(method, params) { this.calls.push({ method, params }); return official(); } }
  const context = vm.createContext({
    window, MessageEvent, Error, console, Date,
    codexPlusIsWindowsPlatform: false, codexAppServerModelRequestPatchVersion: "11", codexPluginMarketplaceUnlockVersion: "17",
    codexPlusBackendSettingsLoaded: true,
    setTimeout: (callback, delay) => { const id = ++timerId; timers.set(id, { callback, delay }); return id; },
    clearTimeout: (id) => timers.delete(id),
    codexPluginMarketplacePatchEnabled: () => enabled,
    clearPluginMarketplaceQueryCache: () => {}, showToast: (message) => toasts.push(message),
    sendCodexPlusDiagnostic: () => {},
    restorePluginMarketplaceRequestParams: (params) => params,
    pluginMarketplaceRequestProfile: () => ({ remoteOnly: false }),
    patchPluginMarketplaceRequestParams: (_, params) => params,
    patchPluginMarketplaceResult: (_, result) => result,
    patchPluginMarketplaceRequestMessage: (message) => message,
    patchPluginMarketplaceResponseData: () => false,
    pluginMarketplaceRemoteAuthError: (error) => /chatgpt authentication required for remote plugin catalog.*api key auth is not supported/i.test(error?.message || error?.error?.message || String(error)),
    markPluginMarketplaceRemoteCatalogUnavailable: () => {},
    remoteOnlyPluginMarketplaceFallbackResult: () => ({ marketplaces: [] }), localPluginMarketplaceFallbackResult: () => ({ marketplaces: [] }),
    codexPlusModelUnlockEnabled: () => false, codexRemoteSessionProviderPatchEnabled: () => false,
    registerNativeHostClient: () => {},
    codexPlusSettings: () => ({ serviceTierControls: false }),
    codexRemoteSessionProviderRequestMethod: () => false, codexRemoteSessionProviderOverrideEnabled: () => false,
    applyCodexRemoteSessionProviderOverride: (_, params) => params, applyCodexServiceTierRequestOnly: (_, params) => params,
    refreshCodexThreadModelBeforeTurn: async () => null,
    codexThreadModelRequestState: () => ({ threadId: "", model: "" }),
    NativeClient,
    postJson: async (path, payload) => {
      calls.push({ path, payload });
      if (handler) return await handler(path, payload);
      if (payload.method === "list-plugins" || payload.method === "installed-plugins") return catalog();
      if (payload.method === "read-plugin") return { plugin: { summary: { ...managedSummary }, skills: [] } };
      if (payload.method === "install-plugin") return installResult();
      throw new Error(`Unexpected request: ${path}`);
    },
  });
  vm.runInContext(`${methodFunction}\n${adapter}\n${clientPatch}\n${transportPatch}\n${prototypePatch}\nglobalThis.api = {
    intercept: codexPlusPluginNativeInterceptClient, operation: codexPlusPluginNativeOperation,
    outgoing: codexPlusPluginNativeInterceptOutgoing, incoming: codexPlusPluginNativeInterceptIncoming,
    patchClient: patchPluginMarketplaceRequestClient, patchBridge: installPluginMarketplaceBridgePatch,
    patchPrototype: installCodexAppServerClientPrototypePatch, normalize: appServerModelRequestMethod,
  };`, context);
  return {
    api: context.api, context, window, calls, originalMessages, delivered, timers, toasts,
    response: (data) => window.dispatchEvent(new MessageEvent("message", { data, source: window })),
    outboundEvent: (detail, forwarded = false) => window.dispatchEvent({ type: "codex-message-from-view", detail, __codexForwardedViaBridge: forwarded }),
    runPoll: async () => { const [id, timer] = [...timers].find(([, timer]) => timer.delay === 1500); timers.delete(id); timer.callback(); await flush(); },
  };
}

test("原生 client 列表保留系统插件，托管详情/安装不调用原接口，其他插件与模型透传", async () => {
  const f = fixture();
  const client = new f.context.NativeClient();
  f.api.patchClient(client);
  const list = await client.sendRequest("plugin/list", {});
  assert.equal(list.marketplaces.length, 2);
  assert.equal(list.marketplaces[0].plugins[0].name, "system");
  const params = { marketplacePath: sentinel(), pluginName: "sample" };
  const read = await client.sendRequest("plugin/read", params);
  assert.equal(read.plugin.summary.codexPlusIndexId, "original-id");
  const installed = await client.sendRequest("plugin/install", params);
  assert.equal(installed.authPolicy, "ON_USE");
  assert.equal(client.calls.length, 1, "managed read/install never enter native sendRequest");
  await client.sendRequest("plugin/install", { marketplacePath: "/official/marketplace.json", pluginName: "ordinary" });
  await client.sendRequest("plugin/uninstall", { pluginId: "sample@hash-market" });
  await client.sendRequest("model/list", {});
  assert.deepEqual(client.calls.slice(1).map((call) => call.method), ["plugin/install", "plugin/uninstall", "model/list"]);
  assert.match(f.toasts[0], /重启 Codex/);
});

test("public可见、full权限错误不清空原生列表；非鉴权原生错误不被吞掉", async () => {
  const f = fixture(async (_, payload) => payload.method === "list-plugins"
    ? { ...catalog(), marketplaceLoadErrors: [{ marketplacePath: sentinel("full"), message: "需要 GitHub 访问权限" }] } : installResult());
  const original = async () => official();
  const result = await f.api.intercept("list-plugins", {}, {}, original, "local");
  assert.equal(result.marketplaces.length, 2);
  assert.match(result.marketplaceLoadErrors[0].message, /GitHub/);
  await assert.rejects(f.api.intercept("plugin/list", {}, {}, async () => { throw new Error("protocol malformed"); }, "local"), /protocol malformed/);
  const failed = fixture(async () => ({ status: "failed", message: "后端未连接" }));
  const kept = await failed.api.intercept("plugin/list", {}, {}, original, "local");
  assert.equal(kept.marketplaces[0].plugins[0].name, "system");
  assert.match(kept.marketplaceLoadErrors[0].message, /后端未连接/);
});

test("原生远程鉴权错误允许索引兜底，远程专用市场不混入本机索引", async () => {
  const f = fixture();
  const result = await f.api.intercept("plugin/list", {}, {}, async () => { throw new Error("chatgpt authentication required for remote plugin catalog; api key auth is not supported"); }, "local");
  assert.equal(result.marketplaces[0].name, "codex-plus-index-public");
  assert.equal(f.api.intercept("plugin/list", { marketplaceKinds: ["shared-with-me"] }, {}, async () => official(), "local"), null);
});

test("legacy fetch列表 envelope 与 MCP托管read/install响应保持真实原生字段", async () => {
  const f = fixture(); f.api.patchBridge();
  await f.window.electronBridge.sendMessageFromView({ type: "fetch", url: "vscode://codex/list-plugins", requestId: "fetch-one", body: "{}" });
  assert.equal(f.originalMessages.length, 1);
  f.response({ type: "fetch-response", requestId: "fetch-one", responseType: "success", status: 200, headers: {}, bodyJsonString: JSON.stringify(official()) });
  await flush();
  const fetchResponse = f.delivered.find((event) => event.data?.type === "fetch-response").data;
  assert.equal(fetchResponse.requestId, "fetch-one");
  assert.equal(fetchResponse.responseType, "success");
  assert.equal(JSON.parse(fetchResponse.bodyJsonString).marketplaces.length, 2);
  const params = { marketplacePath: sentinel(), pluginName: "sample" };
  await f.window.electronBridge.sendMessageFromView({ type: "mcp-request", hostId: "local", request: { id: 7, method: "plugin/read", params } });
  await flush();
  const readResponse = f.delivered.find((event) => event.data?.message?.id === 7).data;
  assert.equal(readResponse.hostId, "local");
  assert.equal(readResponse.message.result.plugin.summary.codexPlusIndexId, "original-id");
  await f.window.electronBridge.sendMessageFromView({ type: "mcp-request", hostId: "local", request: { id: 8, method: "plugin/install", params } });
  await flush();
  assert.equal(f.delivered.find((event) => event.data?.message?.id === 8).data.message.result.authPolicy, "ON_USE");
  assert.equal(f.originalMessages.length, 1, "managed traffic never reaches the original electron bridge");
  await f.window.electronBridge.sendMessageFromView({ type: "fetch", url: "vscode://codex/plugin/install", requestId: "install-fetch", body: JSON.stringify(params) });
  await flush();
  assert.equal(JSON.parse(f.delivered.find((event) => event.data?.requestId === "install-fetch").data.bodyJsonString).authPolicy, "ON_USE");
  assert.equal(f.originalMessages.length, 1, "managed legacy fetch install also bypasses native IPC");
});

test("fallback事件接管托管安装，已桥接的同一事件不重复提交；原生卸载仍发送", async () => {
  const f = fixture(); f.api.patchBridge();
  const request = { type: "mcp-request", hostId: "local", request: { id: "evt", method: "plugin/install", params: { marketplacePath: sentinel(), pluginName: "sample" } } };
  f.outboundEvent(request); await flush();
  f.outboundEvent(request, true); await flush();
  assert.equal(f.calls.filter((call) => call.payload.method === "install-plugin").length, 1);
  assert.ok(!f.delivered.some((event) => event.type === "codex-message-from-view"));
  await f.window.electronBridge.sendMessageFromView({ type: "mcp-request", hostId: "local", request: { id: "remove", method: "plugin/uninstall", params: { pluginId: managedSummary.id } } });
  assert.equal(f.originalMessages.length, 1);
});

test("安装桥接超时保留 Promise 并查询原任务，绝不重新提交或走原生安装", async () => {
  let polls = 0;
  const f = fixture(async (path, payload) => {
    if (path === "/plugin-market/install-status") return ++polls === 1 ? { status: "ok", busy: true, stage: "fetching" } : { status: "ok", busy: false, stage: "complete", nativeResult: installResult() };
    if (payload.method === "read-plugin") return { plugin: { summary: managedSummary } };
    if (payload.method === "install-plugin") throw new Error("bridge request timeout after 26 seconds");
  });
  let settled = false;
  const params = { marketplacePath: sentinel(), pluginName: "sample" };
  const promise = f.api.intercept("plugin/install", params, {}, async () => { throw new Error("original must not execute"); }, "local").then((result) => { settled = true; return result; });
  await flush();
  assert.equal(settled, false);
  await f.runPoll(); assert.equal(settled, false);
  await f.runPoll();
  assert.equal((await promise).authPolicy, "ON_USE");
  assert.equal(f.calls.filter((call) => call.payload.method === "install-plugin").length, 1);
  assert.equal(f.calls.filter((call) => call.path === "/plugin-market/install-status").length, 2);
});

test("远程host列表不混入本机市场，托管read/install明确拒绝且不写本机", async () => {
  const f = fixture();
  assert.equal(f.api.intercept("plugin/list", {}, {}, async () => official(), "ssh-host"), null);
  for (const method of ["plugin/read", "plugin/install"]) {
    await assert.rejects(f.api.intercept(method, { marketplacePath: sentinel(), pluginName: "sample" }, {}, async () => { throw new Error("native must not receive virtual path"); }, "ssh-host"), /仅支持本机/);
  }
  assert.equal(f.api.intercept("send-cli-request-for-host", { hostId: "ssh-host", method: "plugin/list", params: {} }, {}, async () => official(), "local"), null);
  await assert.rejects(f.api.intercept("send-cli-request-for-host", { hostId: "local", method: "plugin/install", params: { marketplacePath: sentinel(), pluginName: "sample" } }, {}, async () => {}, "ssh-host"), /仅支持本机/);
  assert.equal(f.calls.length, 0);
});

test("原生已注册的托管 hashmarket 与virtual组同id只显示一次，官方同名插件仍保留", async () => {
  const f = fixture();
  const base = official();
  base.marketplaces.push({ name: "codex-plus-plugin-market-123456789abc", path: "/fixture/installed/marketplace.json", plugins: [{ ...managedSummary, installed: true }] });
  base.marketplaces[0].plugins.push({ name: "sample", id: "sample@openai-bundled" });
  const result = await f.api.intercept("plugin/list", {}, {}, async () => base, "local");
  const summaries = result.marketplaces.flatMap((market) => market.plugins);
  assert.equal(summaries.filter((plugin) => plugin.id === managedSummary.id).length, 1);
  assert.equal(summaries.filter((plugin) => plugin.id === "sample@openai-bundled").length, 1);
});

test("安装状态连续通信失败后停止等待，仍不重发安装", async () => {
  const f = fixture(async (path, payload) => {
    if (path === "/plugin-market/install-status") throw new Error("bridge disconnected");
    if (payload.method === "read-plugin") return { plugin: { summary: managedSummary } };
    if (payload.method === "install-plugin") throw new Error("bridge timeout");
  });
  const promise = f.api.intercept("plugin/install", { marketplacePath: sentinel(), pluginName: "sample" }, {}, async () => {}, "local");
  const failed = assert.rejects(promise, /暂时无法确认安装结果/);
  await flush();
  for (let index = 0; index < 5; index += 1) await f.runPoll();
  await failed;
  assert.equal(f.calls.filter((call) => call.payload.method === "install-plugin").length, 1);
  assert.equal([...f.timers].filter(([, timer]) => timer.delay === 1500).length, 0);
});

test("捕获原型 client 以及嵌套send-cli请求也接管；多层适配不重复取索引", async () => {
  const f = fixture();
  f.window.__codexPlusAppServerClientClass = f.context.NativeClient;
  assert.equal(f.api.patchPrototype(), true);
  const client = new f.context.NativeClient();
  f.api.patchClient(client);
  const list = await client.sendRequest("send-cli-request-for-host", { hostId: "local", method: "plugin/list", params: {} });
  assert.equal(list.marketplaces.length, 2);
  assert.equal(f.calls.filter((call) => call.payload.method === "list-plugins").length, 1);
  assert.equal(client.calls.length, 1);
  const read = await client.sendRequest("send-cli-request-for-host", { hostId: "local", method: "plugin/read", params: { marketplacePath: sentinel(), pluginName: "sample" } });
  assert.equal(read.plugin.summary.codexPlusIndexId, "original-id");
  assert.equal(client.calls.length, 1);
  assert.equal(f.api.normalize("send-cli-request-for-host", { method: "plugin/installed", params: {} }), "installed-plugins");
});

test("fetch JSONRPC封装、原生非鉴权错误和无关message分别保留", async () => {
  const f = fixture(); f.api.patchBridge();
  await f.window.electronBridge.sendMessageFromView({ type: "fetch", url: "vscode://codex/list-plugins", requestId: "rpc", body: "{}" });
  f.response({ type: "fetch-response", requestId: "rpc", responseType: "success", status: 200, bodyJsonString: JSON.stringify({ id: "inner", result: official() }) });
  await flush();
  const rpc = JSON.parse(f.delivered.find((event) => event.data?.requestId === "rpc").data.bodyJsonString);
  assert.equal(rpc.id, "inner"); assert.equal(rpc.result.marketplaces.length, 2);
  await f.window.electronBridge.sendMessageFromView({ type: "mcp-request", hostId: "local", request: { id: 99, method: "plugin/list", params: {} } });
  const originalError = { type: "mcp-response", hostId: "local", message: { id: 99, error: { code: -1, message: "config broken" } } };
  f.response(originalError); await flush();
  assert.equal(f.delivered.find((event) => event.data?.message?.id === 99).data, originalError);
  assert.equal(f.api.incoming({ type: "mcp-notification", params: {} }), false);
});

test("已有旧版桥接wrapper的当前窗口重新注入后也先截获托管请求", async () => {
  const f = fixture();
  f.window.electronBridge.__codexPluginMarketplaceOriginalSendMessageFromView = f.window.electronBridge.sendMessageFromView.bind(f.window.electronBridge);
  f.window.electronBridge.__codexPluginMarketplaceBridgePatch = "16";
  f.window.__codexPluginMarketplaceOriginalDispatchEvent = f.window.dispatchEvent;
  f.window.__codexPluginMarketplaceWindowEventPatch = "16";
  f.window.__codexPluginMarketplaceBridgePatch = "16";
  f.api.patchBridge();
  const params = { marketplacePath: sentinel(), pluginName: "sample" };
  await f.window.electronBridge.sendMessageFromView({ type: "mcp-request", hostId: "local", request: { id: 88, method: "plugin/install", params } });
  await flush();
  assert.equal(f.originalMessages.length, 0);
  assert.equal(f.delivered.find((event) => event.data?.message?.id === 88).data.message.result.authPolicy, "ON_USE");
  const original = f.window.electronBridge.sendMessageFromView;
  f.api.patchBridge();
  assert.equal(f.window.electronBridge.sendMessageFromView, original, "scans must not accumulate wrappers");
});
