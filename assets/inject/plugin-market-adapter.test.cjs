const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

const root = `${__dirname}/renderer-inject`;
const adapter = readFileSync(`${root}/62-plugin-market-adapter.js`, "utf8");
const model = readFileSync(`${root}/70-model-catalog.js`, "utf8");
const native = readFileSync(`${root}/60-plugin-marketplace.js`, "utf8");
const serviceTier = readFileSync(`${root}/30-service-tier.js`, "utf8");
const sessionConnection = serviceTier.slice(serviceTier.indexOf("  function codexManagedSessionConnection("),
  serviceTier.indexOf("  function codexRemoteSessionStartedThreadId("));
const methodFunction = model.slice(model.indexOf("  function appServerModelRequestMethod("), model.indexOf("  function patchAppServerModelResult("));
const clientPatch = native.slice(native.indexOf("  function patchPluginMarketplaceRequestClient("), native.indexOf("  function patchPluginMarketplaceRequestMessage("));
const transportPatch = native.slice(native.indexOf("  function installPluginMarketplaceBridgePatch("), native.indexOf("  const pluginMarketplaceRequestPatchMaxMisses"));
const prototypePatch = model.slice(model.indexOf("    function installCodexAppServerClientPrototypePatch("), model.indexOf("  const appServerModelRequestPatchMaxMisses"));
// 原型补丁里经旁支保存（#2660）包装原生请求；辅助函数定义在截取范围外，需要一并带上。
const sideChatSend = model.slice(model.indexOf("  function sendAppServerRequestWithSideChat("), model.indexOf("  // issue #2177："));
const flush = async () => { await new Promise((resolve) => setImmediate(resolve)); await new Promise((resolve) => setImmediate(resolve)); };
const sentinel = (source = "public") => `/fixture/.tmp/codex-plus-plugin-market/virtual-${source}/.agents/plugins/marketplace.json`;
const managedSummary = { name: "sample", id: "sample@codex-plus-plugin-market-123456789abc", codexPlusIndexId: "original-id", installed: false };
const catalog = () => ({ marketplaces: [{ name: "codex-plus-index-public", path: sentinel(), plugins: [{ ...managedSummary }] }], marketplaceLoadErrors: [], featuredPluginIds: [] });
const official = () => ({ marketplaces: [{ name: "openai-bundled", path: "/fixture/bundled.json", plugins: [{ name: "system", id: "system@openai-bundled" }] }], marketplaceLoadErrors: [], featuredPluginIds: ["system@openai-bundled"] });
const installResult = () => ({ appsNeedingAuth: [], authPolicy: "ON_USE" });

function fixture(handler, { enabled = true } = {}) {
  const calls = [], originalMessages = [], delivered = [], toasts = [], retainedRequests = [], handlers = new Map(), timers = new Map();
  let timerId = 0;
  const modelCatalogRuntime = {};
  const window = {
    location: { origin: "app://-" },
    __codexPlusModelCatalogRuntime: modelCatalogRuntime,
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
  class NativeClient { constructor(hostId = "local") { this.hostId = hostId; this.calls = []; this.options = []; } async sendRequest(method, params, options) { this.calls.push({ method, params }); this.options.push(options); return official(); } }
  const context = vm.createContext({
    window, MessageEvent, Error, console, Date,
    codexModelCatalogRuntime: modelCatalogRuntime,
    codexPlusIsWindowsPlatform: false, codexAppServerModelRequestPatchVersion: "11", codexPluginMarketplaceUnlockVersion: "17",
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: {},
    codexRemoteSessionActiveProfile: () => null,
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
    sendCodexRetainedModelRequest: (client, send, method, params, options) => {
      retainedRequests.push({ client, method, params, options });
      return send(method, params, options);
    },
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
  vm.runInContext(`${methodFunction}\n${sessionConnection}\n${adapter}\n${clientPatch}\n${transportPatch}\n${sideChatSend}\n${prototypePatch}\nglobalThis.api = {
    intercept: codexPlusPluginNativeInterceptClient, operation: codexPlusPluginNativeOperation,
    outgoing: codexPlusPluginNativeInterceptOutgoing, incoming: codexPlusPluginNativeInterceptIncoming,
    patchClient: patchPluginMarketplaceRequestClient, patchBridge: installPluginMarketplaceBridgePatch,
    patchPrototype: installCodexAppServerClientPrototypePatch, normalize: appServerModelRequestMethod,
  };`, context);
  return {
    api: context.api, context, window, calls, originalMessages, delivered, timers, toasts, retainedRequests,
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

test("未启用聚合保护时，模型与插件原型包装共同保留原生请求", async () => {
  const f = fixture();
  f.window.__codexPlusAppServerClientClass = f.context.NativeClient;
  assert.equal(f.api.patchPrototype(), true);
  const client = new f.context.NativeClient();
  f.api.patchClient(client);
  const modelParams = { includeHidden: false };
  const turnParams = { threadId: "fixture-thread", model: "fixture-model", input: [] };
  const options = { priority: "fixture" };
  await client.sendRequest("model/list", modelParams, options);
  await client.sendRequest("turn/start", turnParams, options);
  assert.deepEqual(client.calls, [
    { method: "model/list", params: modelParams },
    { method: "turn/start", params: turnParams },
  ]);
  assert.deepEqual(client.options, [options, options]);
  assert.deepEqual(f.retainedRequests.map(({ method }) => method), ["model/list", "turn/start"]);
  assert.ok(f.retainedRequests.every((request) => request.client === client && request.options === options));
  assert.equal(f.calls.length, 0, "无关请求不访问插件索引");
});

test("过期模型目录运行时不能安装原型补丁，当前运行时安装保持幂等", () => {
  const f = fixture();
  f.window.__codexPlusAppServerClientClass = f.context.NativeClient;
  const original = f.context.NativeClient.prototype.sendRequest;
  f.window.__codexPlusModelCatalogRuntime = {};
  assert.equal(f.api.patchPrototype(), false);
  assert.equal(f.context.NativeClient.prototype.sendRequest, original);
  f.window.__codexPlusModelCatalogRuntime = f.context.codexModelCatalogRuntime;
  assert.equal(f.api.patchPrototype(), true);
  const patched = f.context.NativeClient.prototype.sendRequest;
  assert.notEqual(patched, original);
  assert.equal(f.api.patchPrototype(), true);
  assert.equal(f.context.NativeClient.prototype.sendRequest, patched);
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

test("配置读取超时与插件传输同时安装时只取消读取并透传原生错误", async () => {
  for (const guardFirst of [true, false]) {
    const f = fixture();
    f.window.setTimeout = f.context.setTimeout;
    f.window.clearTimeout = f.context.clearTimeout;
    const installGuard = () => vm.runInContext(
      readFileSync(`${root}/31-config-read-deadline.js`, "utf8") + "\ninstallCodexConfigReadDeadline();", f.context);
    if (guardFirst) installGuard();
    f.api.patchBridge();
    if (!guardFirst) installGuard();
    const config = { type: "fetch", method: "POST", requestId: "config",
      url: "vscode://codex/mcp-codex-config", body: '{"cwd":"/fixture"}' };
    await f.window.electronBridge.sendMessageFromView(config);
    f.outboundEvent(config, true);
    const turn = { type: "mcp-request", request: { id: "turn", method: "turn/start", params: {} } };
    await f.window.electronBridge.sendMessageFromView(turn);
    f.outboundEvent(turn, true);
    const [timerId, timer] = [...f.timers].find(([, timer]) => timer.delay === 15_000);
    f.timers.delete(timerId);
    timer.callback();
    await flush();
    const replies = f.delivered.filter((event) => event.data?.requestId === "config");
    assert.equal(replies.length, 1);
    assert.equal(replies[0].data.responseType, "error");
    assert.equal(replies[0].data.status, 408);
    assert.deepEqual(f.originalMessages.map(({ type }) => type), ["fetch", "mcp-request", "cancel-fetch"]);
    assert.equal(f.calls.length, 0, "不访问插件索引或重发用户请求");
  }
});

// #2790：新版 Codex 插件页输入关键词后改走服务端 plugin/search，不再本地过滤 plugin/list。
const gmailSummary = { name: "gmail", id: "gmail@codex-plus-plugin-market-0123456789ab", codexPlusIndexId: "gmail", installed: false,
  keywords: ["email", "google workspace"], interface: { displayName: "Gmail", shortDescription: "Work with Gmail using the configured Gmail app connector." } };
const mailToolSummary = { name: "mail-tool", id: "mail-tool@codex-plus-plugin-market-ba9876543210", codexPlusIndexId: "mail-tool", installed: false,
  keywords: [], interface: { displayName: "Mail Tool", shortDescription: "Drafts replies for Gmail threads." } };
const searchCatalog = () => ({ marketplaces: [{ name: "codex-plus-index-public", path: sentinel(), plugins: [{ ...mailToolSummary }, { ...gmailSummary }, { ...managedSummary }] }], marketplaceLoadErrors: [], featuredPluginIds: [] });
// vm 上下文里的数组与测试进程不同 realm，先展开再比较。
const pluginNames = (result) => [...result.data].map((entry) => entry.plugin.name);
const searchHandler = async (_, payload) => {
  if (payload.method === "list-plugins") return searchCatalog();
  if (payload.method === "read-plugin") return { plugin: { summary: { ...gmailSummary }, skills: [] } };
  return installResult();
};

test("plugin/search 首页结果补入索引插件：名称命中优先、保留原生结果与游标、可直接安装", async () => {
  const f = fixture(searchHandler);
  const client = new f.context.NativeClient();
  client.sendRequest = async (method, params) => {
    client.calls.push({ method, params });
    return { data: [{ marketplaceName: "openai-bundled", marketplacePath: "/fixture/bundled.json", plugin: { name: "system", id: "system@openai-bundled" } }], nextCursor: "next" };
  };
  f.api.patchClient(client);
  assert.equal(f.api.normalize("plugin/search", {}), "search-plugins");
  const result = await client.sendRequest("plugin/search", { searchTerm: " GMAIL ", limit: 50 });
  assert.deepEqual(pluginNames(result), ["system", "gmail", "mail-tool"]);
  assert.equal(result.nextCursor, "next", "保留原生翻页游标");
  assert.equal(result.data[1].marketplacePath, sentinel());
  assert.equal(result.data[1].marketplaceName, "codex-plus-index-public");
  assert.equal(client.calls.length, 1, "原生 plugin/search 照常执行");
  // 再搜一次走目录缓存，不重复拉索引。
  await client.sendRequest("plugin/search", { searchTerm: "google workspace" });
  assert.equal(f.calls.filter((call) => call.payload.method === "list-plugins").length, 1);
  // 搜索结果里的托管插件安装仍由本层接管，不进原生安装器。
  const installed = await client.sendRequest("plugin/install", { marketplacePath: sentinel(), pluginName: "gmail" });
  assert.equal(installed.authPolicy, "ON_USE");
  assert.equal(f.calls.find((call) => call.payload.method === "install-plugin").payload.params.pluginName, "gmail");
  assert.equal(client.calls.length, 2);
});

test("plugin/search：原生同 id 不重复；翻页、个人目录、远程 host、空关键词不混入；遵守 limit", async () => {
  const f = fixture(searchHandler);
  const native = async () => ({ data: [{ marketplaceName: "codex-plus-plugin-market-0123456789ab", marketplacePath: "/fixture/installed.json", plugin: { ...gmailSummary, installed: true } }], nextCursor: null });
  const deduped = await f.api.intercept("plugin/search", { searchTerm: "gmail" }, {}, native, "local");
  assert.equal(deduped.data.filter((entry) => entry.plugin.id === gmailSummary.id).length, 1);
  assert.equal(deduped.data[0].plugin.installed, true);
  assert.equal(f.api.intercept("plugin/search", { searchTerm: "gmail", cursor: "page-2" }, {}, native, "local"), null);
  assert.equal(f.api.intercept("plugin/search", { searchTerm: "gmail", scope: "personal" }, {}, native, "local"), null);
  assert.equal(f.api.intercept("plugin/search", { searchTerm: "gmail" }, {}, native, "ssh-host"), null);
  const empty = async () => ({ data: [], nextCursor: null });
  const global = await f.api.intercept("plugin/search", { searchTerm: "mail tool", scope: "global" }, {}, empty, "local");
  assert.deepEqual(pluginNames(global), ["mail-tool"]);
  const blank = await f.api.intercept("plugin/search", { searchTerm: "  " }, {}, empty, "local");
  assert.deepEqual(pluginNames(blank), []);
  const limited = await f.api.intercept("plugin/search", { searchTerm: "gmail", limit: 1 }, {}, empty, "local");
  assert.deepEqual(pluginNames(limited), ["gmail"]);
});

test("plugin/search：API Key 远程鉴权错误兜底为索引结果，其他原生错误透传，索引离线时保留原生结果", async () => {
  const f = fixture(searchHandler);
  const authFailed = await f.api.intercept("plugin/search", { searchTerm: "gmail" }, {},
    async () => { throw new Error("chatgpt authentication required for remote plugin catalog; api key auth is not supported"); }, "local");
  assert.deepEqual(pluginNames(authFailed), ["gmail", "mail-tool"]);
  assert.equal(authFailed.nextCursor, null);
  await assert.rejects(f.api.intercept("plugin/search", { searchTerm: "gmail" }, {}, async () => { throw new Error("protocol malformed"); }, "local"), /protocol malformed/);
  const offline = fixture(async () => ({ status: "failed", message: "后端未连接" }));
  const kept = await offline.api.intercept("plugin/search", { searchTerm: "system" }, {},
    async () => ({ data: [{ marketplaceName: "openai-bundled", marketplacePath: "/fixture/bundled.json", plugin: { name: "system", id: "system@openai-bundled" } }], nextCursor: null }), "local");
  assert.deepEqual(pluginNames(kept), ["system"]);
});

test("plugin/search 走 MCP 消息传输时也合并索引结果", async () => {
  const f = fixture(searchHandler);
  f.api.patchBridge();
  await f.window.electronBridge.sendMessageFromView({ type: "mcp-request", hostId: "local", request: { id: "plugin/search:1", method: "plugin/search", params: { searchTerm: "gmail", limit: 50 } } });
  assert.equal(f.originalMessages.length, 1, "原生搜索请求照常发出");
  f.response({ type: "mcp-response", hostId: "local", message: { id: "plugin/search:1", result: { data: [], nextCursor: null } } });
  await flush();
  const reply = f.delivered.find((event) => event.data?.message?.id === "plugin/search:1").data;
  assert.deepEqual(pluginNames(reply.message.result), ["gmail", "mail-tool"]);
});

test("装完插件刷新原生插件列表：新版 Codex 没有全局 queryClient 时从 React 树找到并按 plugins 前缀失效", () => {
  const conversation = readFileSync(`${root}/95-conversation-view.js`, "utf8");
  const clear = native.slice(native.indexOf("  function clearPluginMarketplaceQueryCache("), native.indexOf("  function installPluginMarketplaceBridgePatch("));
  const finder = conversation.slice(conversation.indexOf("  function looksLikeQueryClient("), conversation.indexOf("  function mainRateLimitQueries("));
  const invalidated = [], diagnostics = [];
  const client = { getQueryCache() { return {}; }, setQueryData() {}, invalidateQueries(filters) { invalidated.push(filters.queryKey); return Promise.resolve(); } };
  // React 根容器 → QueryClientProvider（props.client），没有 window.__REACT_QUERY_CLIENT__。
  const rootNode = { "__reactContainer$x": { stateNode: { current: { child: { memoizedProps: {}, sibling: { memoizedProps: { client } } } } } } };
  const run = (documentRoot) => {
    const context = {
      window: {}, document: { getElementById: () => documentRoot, body: null, documentElement: null },
      clearPluginMarketplaceRemoteCatalogUnavailable() {},
      sendCodexPlusDiagnostic(event) { diagnostics.push(event); },
    };
    vm.runInNewContext(`${finder}\n${clear}\nclearPluginMarketplaceQueryCache(); clearPluginMarketplaceQueryCache();`, context);
  };
  run(rootNode);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidated)), [["plugins"], ["plugins"]]);
  assert.deepEqual(diagnostics, []);
  // 找不到时只报一次诊断，不抛错。
  run(null);
  assert.deepEqual(diagnostics, ["plugin_marketplace_query_client_missing"]);
});
