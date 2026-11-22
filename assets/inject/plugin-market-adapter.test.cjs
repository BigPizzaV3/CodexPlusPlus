const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

const root = `${__dirname}/renderer-inject`;
const adapter = readFileSync(`${root}/62-plugin-market-adapter.js`, "utf8");
const model = readFileSync(`${root}/70-model-catalog.js`, "utf8");
const native = readFileSync(`${root}/60-plugin-marketplace.js`, "utf8");
const rpc = readFileSync(`${root}/91-extension-rpc.js`, "utf8");
const style = readFileSync(`${root}/10-style.js`, "utf8");
const serviceTier = readFileSync(`${root}/30-service-tier.js`, "utf8");
const sessionConnection = serviceTier.slice(serviceTier.indexOf("  function codexManagedSessionConnection("),
  serviceTier.indexOf("  function codexRemoteSessionStartedThreadId("));
const methodFunction = model.slice(model.indexOf("  function appServerModelRequestMethod("), model.indexOf("  function patchAppServerModelResult("));
const clientPatch = native.slice(native.indexOf("  function patchPluginMarketplaceRequestClient("), native.indexOf("  function patchPluginMarketplaceRequestMessage("));
const transportPatch = native.slice(native.indexOf("  function installPluginMarketplaceBridgePatch("), native.indexOf("  const pluginMarketplaceRequestPatchMaxMisses"));
const prototypePatch = model.slice(model.indexOf("    function installCodexAppServerClientPrototypePatch("), model.indexOf("  const appServerModelRequestPatchMaxMisses"));
// 原型补丁里经旁支保存（#2660）包装原生请求；辅助函数定义在截取范围外，需要一并带上。
const sideChatSend = model.slice(model.indexOf("  async function sendAppServerRequestThroughExchange("), model.indexOf("  // issue #2177："));
const flush = async () => { await new Promise((resolve) => setImmediate(resolve)); await new Promise((resolve) => setImmediate(resolve)); };
const sentinel = (source = "public") => `/fixture/.tmp/codex-plus-plugin-market/virtual-${source}/.agents/plugins/marketplace.json`;
const managedSummary = { name: "sample", id: "sample@codex-plus-plugin-market-123456789abc", codexPlusIndexId: "original-id", installed: false };
const catalog = () => ({ marketplaces: [{ name: "codex-plus-index-public", path: sentinel(), plugins: [{ ...managedSummary }] }], marketplaceLoadErrors: [], featuredPluginIds: [] });
const official = () => ({ marketplaces: [{ name: "openai-bundled", path: "/fixture/bundled.json", plugins: [{ name: "system", id: "system@openai-bundled" }] }], marketplaceLoadErrors: [], featuredPluginIds: ["system@openai-bundled"] });
const installResult = () => ({ appsNeedingAuth: [], authPolicy: "ON_USE" });

/** 本机 app-server 的替身：记下请求，emit 触发已订阅的通知。 */
function appServer(statuses) {
  const requests = [];
  let listener = null;
  return {
    requests,
    get subscribed() { return listener != null; },
    emit: (params) => listener?.({ params }),
    client: {
      async sendRequest(method, params) {
        requests.push({ method, params });
        if (method === "mcpServerStatus/list") return { data: statuses, nextCursor: null };
        if (method === "mcpServer/oauth/login") return { authorizationUrl: "https://auth.example.com/authorize?state=1", loginId: "login-1" };
        return {};
      },
      subscribe(options) {
        listener = options.listener;
        return Promise.resolve({ [Symbol.dispose]() { listener = null; } });
      },
    },
  };
}

/** 页面到宿主的 RPC 端口替身：sent 是发给宿主的帧，received 是页面最终读到的帧。每个 fixture 各用一套，补丁互不影响。 */
function rpcClasses() {
  return {
    MessagePort: class {
      constructor() { this.sent = []; this.received = []; }
      postMessage(frame) { this.sent.push(frame); }
      dispatchEvent(event) { event.currentTarget = this; this.received.push(event.data); return true; }
    },
    MessageEvent: class {
      constructor(type, init) { this.type = type; this.frame = init?.data; }
      get data() { return this.frame; }
    },
  };
}

function fixture(handler, { enabled = true, server = null, withRpc = false } = {}) {
  const calls = [], originalMessages = [], delivered = [], toasts = [], toastOptions = [], opened = [], handlers = new Map(), timers = new Map();
  let timerId = 0;
  // 页面根节点上的属性，和 document 上挂的监听。
  const rootAttributes = new Set(), documentHandlers = new Map();
  // 注入层自己加的 <style>：id → 元素。
  const styles = new Map();
  const document = {
    documentElement: {
      toggleAttribute: (name, on) => { if (on) rootAttributes.add(name); else rootAttributes.delete(name); },
      hasAttribute: (name) => rootAttributes.has(name),
    },
    head: { appendChild: (element) => { styles.set(element.id, element); } },
    createElement: () => { const element = { textContent: "", remove: () => { styles.delete(element.id); } }; return element; },
    getElementById: (id) => styles.get(id) ?? null,
    addEventListener(type, callback) { documentHandlers.set(type, [...(documentHandlers.get(type) || []), callback]); },
    removeEventListener(type, callback) { documentHandlers.set(type, (documentHandlers.get(type) || []).filter((item) => item !== callback)); },
  };
  /** 页面元素的替身：closest 只认测试给的那个选择器结果。 */
  class Element { constructor(closest) { this.closest = closest; } }
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
    open: (...args) => { opened.push(args); },
    ...(withRpc ? rpcClasses() : {}),
  };
  class MessageEvent { constructor(type, init) { Object.assign(this, { type }, init); } stopImmediatePropagation() { this.stopped = true; } }
  class NativeClient { constructor(hostId = "local") { this.hostId = hostId; this.calls = []; } async sendRequest(method, params) { this.calls.push({ method, params }); return official(); } }
  const context = vm.createContext({
    window, document, Element, MessageEvent, Error, console, Date, AbortController, queueMicrotask,
    codexPlusIsWindowsPlatform: false, codexAppServerModelRequestPatchVersion: "11", codexPluginMarketplaceUnlockVersion: "17",
    codexPlusMarkExtensionFailure: (_key, message) => toasts.push(`[failure] ${message}`),
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: {},
    codexRemoteSessionActiveProfile: () => null,
    setTimeout: (callback, delay) => { const id = ++timerId; timers.set(id, { callback, delay }); return id; },
    clearTimeout: (id) => timers.delete(id),
    codexPluginMarketplacePatchEnabled: () => enabled,
    clearPluginMarketplaceQueryCache: () => {},
    showToast: (message, options) => { toasts.push(message); toastOptions.push(options || {}); },
    // 真实的页面里由 91-extension-runtime 提供；这里按英文取。
    codexPlusResolveText: (value) => typeof value === "string" ? value : String(value?.en ?? ""),
    URL,
    codexPlusAppServerClient: async () => { if (!server) throw new Error("Codex 界面还没准备好，稍后再试"); return server.client; },
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
    prepareCodexPureApiResume: async (_, params) => params,
    prepareCodexGatewayReconnect: async () => {},
    noteCodexGatewayThreadStart: () => {},
    verifyCodexPureApiResumeResult: () => {},
    prepareCodexThreadContextStart: async (_, params) => params,
    noteCodexThreadRuntimeContext: () => {},
    // 拓展的 codex/request 中间件：这里没有拓展，原样放行。
    codexPlusApplyExtensionRequestMiddlewareAsync: async (_, params) => params, codexPlusExtensionWantsRequestMiddleware: () => false,
    refreshCodexThreadModelBeforeTurn: async () => null,
    codexThreadModelRequestState: () => ({ threadId: "", model: "" }),
    NativeClient,
    postJson: async (path, payload) => {
      calls.push({ path, payload });
      const result = await backend(path, payload);
      // 真实的后端结果是页面里 JSON.parse 出来的；RPC 编解码按原型认普通对象，这里也给页面这一侧的对象。
      return withRpc ? revive(result) : result;
    },
  });
  const backend = async (path, payload) => {
    if (handler) return await handler(path, payload);
    if (payload.method === "list-plugins" || payload.method === "installed-plugins") return catalog();
    if (payload.method === "read-plugin") return { plugin: { summary: { ...managedSummary }, skills: [] } };
    if (payload.method === "install-plugin") return installResult();
    throw new Error(`Unexpected request: ${path}`);
  };
  // Codex 的 Chromium 自带 Symbol.dispose；Node 22（CI）只给主上下文补了它，新建的 vm 上下文里没有。
  const contextSymbol = vm.runInContext("Symbol", context);
  if (!contextSymbol.dispose) Object.defineProperty(contextSymbol, "dispose", { value: Symbol.dispose });
  const parse = vm.runInContext("(text) => JSON.parse(text)", context);
  const revive = (value) => parse(JSON.stringify(value));
  vm.runInContext(`${methodFunction}\n${sessionConnection}\n${adapter}\n${clientPatch}\n${transportPatch}\n${sideChatSend}\n${prototypePatch}\n${withRpc ? rpc : ""}\nglobalThis.api = {
    intercept: codexPlusPluginNativeInterceptClient, operation: codexPlusPluginNativeOperation,
    outgoing: codexPlusPluginNativeInterceptOutgoing, incoming: codexPlusPluginNativeInterceptIncoming,
    patchClient: patchPluginMarketplaceRequestClient, patchBridge: installPluginMarketplaceBridgePatch,
    patchPrototype: installCodexAppServerClientPrototypePatch, normalize: appServerModelRequestMethod,
    installCategoryLinks: installCodexPlusPluginCategoryLinks, categoryTitle: codexPlusPluginCategoryTitle,
  };`, context);
  return {
    api: context.api, context, window, calls, originalMessages, delivered, timers, toasts, toastOptions, opened, revive, rootAttributes, styles, Element,
    click: (target, init) => { for (const callback of documentHandlers.get("click") || []) callback({ button: 0, defaultPrevented: false, target, ...init }); },
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

test("和官方插件同在「公开」页签：官方已有的同名插件不重复列，已从索引装上的和个人市场的同名不受影响", async () => {
  const index = (installed) => ({
    marketplaces: [{ name: "codex-plus-index-full", path: sentinel("full"), interface: { displayName: "Codex official 目录快照 · 完整库" },
      plugins: [{ ...managedSummary, name: "notion", id: "notion@codex-plus-plugin-market-123456789abc", installed }, { ...managedSummary, name: "mine", id: "mine@codex-plus-plugin-market-0123456789ab" }, { ...managedSummary }] }],
    marketplaceLoadErrors: [], featuredPluginIds: [],
  });
  const native = async () => ({
    marketplaces: [
      { name: "openai-api-curated", path: "/fixture/api.json", interface: { displayName: "Codex official" }, plugins: [{ name: "notion", id: "notion@openai-api-curated" }] },
      { name: "my-market", path: "/fixture/mine.json", plugins: [{ name: "mine", id: "mine@my-market" }] },
    ], marketplaceLoadErrors: [], featuredPluginIds: [],
  });
  // vm 上下文里的数组与测试进程不同 realm，先展开再比较。
  const names = (result) => [...result.marketplaces.find((market) => market.name === "codex-plus-index-full").plugins].map((plugin) => plugin.name);
  const listed = await fixture(async () => index(false)).api.intercept("plugin/list", {}, {}, native, "local");
  assert.deepEqual(names(listed), ["mine", "sample"]);
  assert.equal(listed.marketplaces[0].plugins[0].id, "notion@openai-api-curated", "官方的那一个原样保留");
  const kept = await fixture(async () => index(true)).api.intercept("plugin/list", {}, {}, native, "local");
  assert.deepEqual(names(kept), ["notion", "mine", "sample"]);
});

test("索引目录并进插件页后分类标题点了打开这一类的全部；没并入、点在别处都不动", async () => {
  const f = fixture();
  f.api.installCategoryLinks();
  f.api.installCategoryLinks();
  const title = (id) => new f.Element((selector) => selector === f.api.categoryTitle ? { closest: (inner) => inner === "section" ? { id } : null } : null);
  f.click(title("plugins-developer-tools"));
  assert.equal(f.delivered.length, 0, "目录还没并进页面");
  await f.api.intercept("plugin/list", {}, {}, async () => official(), "local");
  assert.equal(f.rootAttributes.has("data-codex-plus-plugin-categories"), true);
  f.click(title("plugins-data-&-analytics"));
  assert.equal(f.delivered.length, 1, "重复安装只留一个监听");
  const navigation = f.delivered[0];
  assert.deepEqual(JSON.parse(JSON.stringify(navigation.data)), { type: "navigate-to-route", path: "/plugins?category=data-%26-analytics" });
  assert.equal(navigation.source, f.window);
  assert.equal(navigation.origin, "app://-");
  f.click(new f.Element(() => null));
  f.click(title("plugins-security"), { button: 1 });
  f.click({});
  assert.equal(f.delivered.length, 1);
  // 「个人」页签的市场分组不算分类；样式和点击认同一个选择器。
  assert.match(f.api.categoryTitle, /:not\(\[id\^="plugins-marketplace-"\], #plugins-created-by-you, #plugins-shared-with-you\)/);
  assert.ok(style.includes(`html[data-codex-plus-plugin-categories] ${f.api.categoryTitle} { cursor: pointer; }`));

  // 有 ChatGPT 登录时后端不给索引目录，标题交回原生处理。
  const signedIn = fixture(async () => ({ marketplaces: [], marketplaceLoadErrors: [], featuredPluginIds: [] }));
  signedIn.rootAttributes.add("data-codex-plus-plugin-categories");
  await signedIn.api.intercept("plugin/list", {}, {}, async () => official(), "local");
  assert.equal(signedIn.rootAttributes.has("data-codex-plus-plugin-categories"), false);
});

test("官方分组：分类换成官方组名、组内按官方顺序和官方插件穿插着排，合集另列一份", async () => {
  const mine = (name, extra = {}) => ({
    ...managedSummary, name, id: `${name}@codex-plus-plugin-market-123456789abc`, codexPlusIndexId: `p-${name}`,
    interface: { displayName: name, shortDescription: "x", category: "Productivity" }, ...extra,
  });
  const f = fixture(async (_, payload) => payload.method === "list-plugins" ? {
    marketplaces: [{ name: "codex-plus-index-full", path: sentinel("full"), plugins: [
      mine("plain"), mine("docs"), mine("mail"), mine("pinned", { installed: true }), mine("notion"),
      mine("loose", { interface: { category: "Security" } }), mine("odd", { interface: { category: "Unheard Of" } }),
    ] }],
    marketplaceLoadErrors: [], featuredPluginIds: [],
    codexPlusSections: [
      { slug: "featured", title: { en: "Featured", zh: "热门" }, curated: true, plugins: ["p-mail", "p-notion", "p-pinned"] },
      { slug: "business-and-operations", title: { en: "Business & Operations", zh: "业务与运营" }, plugins: ["p-notion", "p-docs", "p-plain", "p-pinned", "p-mail"] },
      { slug: "security", title: { en: "Security" }, plugins: ["p-gone"] },
      { slug: "other", title: { en: "Other" }, plugins: ["p-gone"] },
    ],
  } : installResult());
  const native = async () => ({
    marketplaces: [
      { name: "openai-api-curated", path: "/fixture/api.json", interface: { displayName: "Codex official" }, plugins: [
        { name: "native-one", id: "native-one@openai-api-curated", interface: { category: "Business & Operations" } },
        { name: "notion", id: "notion@openai-api-curated", interface: { category: "Productivity" } },
      ] },
      { name: "my-market", path: "/fixture/mine.json", plugins: [{ name: "kept", id: "kept@my-market" }] },
    ], marketplaceLoadErrors: [], featuredPluginIds: ["native-one@openai-api-curated"],
  });
  const result = await f.api.intercept("plugin/list", {}, {}, native, "local");
  const rows = [...result.marketplaces].flatMap((market) => [...market.plugins].map((plugin) => `${market.name}:${plugin.id.replace("@codex-plus-plugin-market-123456789abc", "")}:${plugin.interface?.category ?? ""}`));
  assert.deepEqual(rows, [
    // 合集：另列一份，id 另起、分类写合集名；装好的 pinned 不在合集里重复。官方的 Notion 顶掉了索引里的同名条目，排在它的位置。
    "codex-plus-index-full:mail@codex-plus-index-full-featured:Featured",
    "openai-api-curated:notion@openai-api-curated-featured:Featured",
    // 分类：官方顺序；分类文字换成这一组的组名。
    "openai-api-curated:notion@openai-api-curated:Business & Operations",
    "codex-plus-index-full:docs:Business & Operations",
    "codex-plus-index-full:plain:Business & Operations",
    "codex-plus-index-full:pinned:Business & Operations",
    "codex-plus-index-full:mail:Business & Operations",
    // 官方没列的按自己的分类接在后面；没有这个分类的归到 Other。
    "openai-api-curated:native-one@openai-api-curated:Business & Operations",
    "codex-plus-index-full:loose:Security",
    "codex-plus-index-full:odd:Other",
    "my-market:kept@my-market:",
  ]);
  // 同一个市场拆成几段穿插着排，名字和路径不变。
  assert.deepEqual([...result.marketplaces].map((market) => market.name),
    ["codex-plus-index-full", "openai-api-curated", "codex-plus-index-full", "openai-api-curated", "codex-plus-index-full", "my-market"]);
  assert.equal(result.marketplaces[2].path, sentinel("full"));
  // 合集由分组给出，不再用原生的 Featured；排版用的数据不带给页面。
  assert.deepEqual([...result.featuredPluginIds], []);
  assert.equal("codexPlusSections" in result, false);
  assert.equal("codexPlusOfficialTwins" in result, false);
  // 页面按组名排分组，样式按官方顺序纠正。
  assert.equal(f.styles.get("codex-plus-plugin-section-order").textContent, [
    'section[id="plugins-featured"] { order: 0; }', 'section[id="plugins-business-&-operations"] { order: 1; }',
    'section[id="plugins-security"] { order: 2; }', 'section[id="plugins-other"] { order: 3; }',
  ].join("\n"));

  // 有 ChatGPT 登录时后端不给索引目录：撤掉分组顺序。
  const signedIn = fixture(async () => ({ marketplaces: [], marketplaceLoadErrors: [], featuredPluginIds: [], codexPlusSections: [] }));
  signedIn.styles.set("codex-plus-plugin-section-order", { remove: () => signedIn.styles.delete("codex-plus-plugin-section-order") });
  const plain = await signedIn.api.intercept("plugin/list", {}, {}, native, "local");
  assert.equal(signedIn.styles.size, 0);
  assert.deepEqual([...plain.marketplaces].map((market) => market.name), ["openai-api-curated", "my-market"]);
  assert.deepEqual([...plain.featuredPluginIds], ["native-one@openai-api-curated"]);
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
  await client.sendRequest("model/list", modelParams);
  await client.sendRequest("turn/start", turnParams);
  assert.deepEqual(client.calls, [
    { method: "model/list", params: modelParams },
    { method: "turn/start", params: turnParams },
  ]);
  assert.equal(f.calls.length, 0, "无关请求不访问插件索引");
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

test("plugin/search：原生结果里已有同名插件时不重复列索引里的那一个", async () => {
  const f = fixture(searchHandler);
  const native = async () => ({ data: [{ marketplaceName: "openai-curated", marketplacePath: "/fixture/curated.json", plugin: { name: "gmail", id: "gmail@openai-curated" } }], nextCursor: null });
  const result = await f.api.intercept("plugin/search", { searchTerm: "gmail" }, {}, native, "local");
  assert.deepEqual(pluginNames(result), ["gmail", "mail-tool"]);
  assert.equal(result.data[0].plugin.id, "gmail@openai-curated");
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

test("回包处理超时把原始响应原样投回，宿主请求不会永久 pending（issue #3098）", async () => {
  const f = fixture((_path, payload) => payload.method === "list-plugins" ? new Promise(() => {}) : catalog());
  f.api.patchBridge();
  await f.window.electronBridge.sendMessageFromView({ type: "fetch", url: "vscode://codex/list-plugins", requestId: "slow", body: "{}" });
  const originalResponse = { type: "fetch-response", requestId: "slow", responseType: "success", status: 200, headers: {}, bodyJsonString: JSON.stringify(official()) };
  f.response(originalResponse);
  assert.ok(!f.delivered.some((event) => event.data?.type === "fetch-response"), "回包已摘下等待合并");
  const timer = [...f.timers.entries()].find(([, t]) => t.delay === 15000);
  assert.ok(timer, "回包处理必须有 15s 兜底定时器");
  timer[1].callback();
  await flush();
  const delivered = f.delivered.find((event) => event.data?.type === "fetch-response");
  assert.equal(delivered.data, originalResponse, "超时后必须把原始回包原样投回渲染层");
  assert.ok(f.toasts.some((message) => message.includes("超时")));
});

const managedParams = { marketplacePath: sentinel(), pluginName: "sample" };
const installWith = (extra) => async (_, payload) => payload.method === "install-plugin"
  ? { ...installResult(), ...extra }
  : { plugin: { summary: { ...managedSummary }, skills: [] } };

test("装完带 MCP 连接器的插件：提示登录，点了才发起 OAuth，完成后重连", async () => {
  const server = appServer([{ name: "demo", authStatus: "notLoggedIn" }, { name: "other", authStatus: "notLoggedIn" }]);
  const f = fixture(installWith({ codexPlusMcpServers: ["demo"], codexPlusHostedOnly: false }), { server });
  const result = await f.api.intercept("plugin/install", managedParams, {}, async () => { throw new Error("native install must not run"); });
  assert.equal(result.authPolicy, "ON_USE");
  await flush();
  assert.match(f.toasts[0], /连接器 demo 要登录/);
  assert.equal(f.toastOptions[0].actionLabel, "登录");
  assert.ok(f.toastOptions[0].durationMs > 10_000, "带按钮的提示要留够时间");
  assert.deepEqual(server.requests.map((request) => request.method), ["mcpServerStatus/list"], "不点就不发起登录");
  assert.equal(f.opened.length, 0);

  await f.toastOptions[0].onAction();
  assert.deepEqual(JSON.parse(JSON.stringify(server.requests.at(-1))), { method: "mcpServer/oauth/login", params: { name: "demo" } });
  assert.deepEqual(f.opened, [["https://auth.example.com/authorize?state=1", "_blank", "noopener,noreferrer"]]);

  server.emit({ name: "other", loginId: "login-9", success: true });
  server.emit({ name: "demo", loginId: "login-9", success: true });
  assert.equal(f.toasts.length, 1, "别的服务、别的登录的通知不算");
  server.emit({ name: "demo", loginId: "login-1", success: true });
  assert.equal(server.requests.at(-1).method, "config/mcpServer/reload");
  assert.match(f.toasts[1], /连接器 demo 已登录/);
  assert.equal(server.subscribed, false, "完成后退订");
});

test("MCP 登录没完成或授权地址不对时说明原因，不打开页面", async () => {
  const server = appServer([{ name: "demo", authStatus: "notLoggedIn" }]);
  const f = fixture(installWith({ codexPlusMcpServers: ["demo"] }), { server });
  await f.api.intercept("plugin/install", managedParams, {}, async () => null);
  await flush();
  await f.toastOptions[0].onAction();
  server.emit({ name: "demo", loginId: "login-1", success: false, error: "access_denied" });
  assert.match(f.toasts[1], /没有完成：access_denied/);
  assert.equal(f.toastOptions[1].type, "warn");

  server.client.sendRequest = async () => ({ authorizationUrl: "javascript:alert(1)" });
  await assert.rejects(f.toastOptions[0].onAction(), /登录没能开始/);
  assert.equal(f.opened.length, 1);
  assert.equal(server.subscribed, false, "没开始的登录不留订阅");
});

test("只有 ChatGPT 托管连接器、且没有 ChatGPT 登录：提示去添加账号", async () => {
  const f = fixture(installWith({ codexPlusMcpServers: [], codexPlusHostedOnly: true }));
  await f.api.intercept("plugin/install", managedParams, {}, async () => null);
  await flush();
  assert.match(f.toasts[0], /要有 ChatGPT 登录才能连接/);
  assert.equal(f.toastOptions[0].actionLabel, "添加 ChatGPT 账号");
  await f.toastOptions[0].onAction();
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls.at(-1))), { path: "/manager/open", payload: { page: "relay" } });
});

test("MCP 连接器已登录时不打扰；还没加载出来时指到原生 MCP 设置", async () => {
  const ready = fixture(installWith({ codexPlusMcpServers: ["demo"] }), { server: appServer([{ name: "demo", authStatus: "oAuth" }]) });
  await ready.api.intercept("plugin/install", managedParams, {}, async () => null);
  await flush();
  assert.match(ready.toasts[0], /重启 Codex 后加载/);
  assert.equal(ready.toastOptions[0].actionLabel, undefined);

  // 拿不到 app-server（界面没准备好）和服务没列出来是同一种处理。
  for (const server of [null, appServer([])]) {
    const f = fixture(installWith({ codexPlusMcpServers: ["demo"] }), { server });
    await f.api.intercept("plugin/install", managedParams, {}, async () => null);
    await flush();
    assert.match(f.toasts[0], /连接器 demo 还没加载/);
    assert.equal(f.toastOptions[0].actionLabel, "打开 MCP 设置");
    f.toastOptions[0].onAction();
    const navigation = f.delivered.at(-1);
    assert.deepEqual(JSON.parse(JSON.stringify(navigation.data)), { type: "navigate-to-route", path: "/settings/mcp-settings" });
    // Codex 26.1117 实测：缺 source 或 origin 的宿主消息不会跳转。
    assert.equal(navigation.source, f.window);
    assert.equal(navigation.origin, "app://-");
  }
});

/** RPC 传输下的一次往返：页面发帧，宿主回帧，返回页面最终读到的那一帧。 */
function rpcFixture(handler, options) {
  const f = fixture(handler, { ...options, withRpc: true });
  const port = new f.window.MessagePort();
  // 帧都是页面这一侧的对象，和真实的 capnweb 一样。
  const send = (frame) => port.postMessage(f.revive(frame));
  const request = (method, params, receiver = 0) => send(["push", ["pipeline", receiver, ["sendRequest"], [method, params]]]);
  const subscribe = (hostId = "local", receiver = 0) => send(["push", ["pipeline", receiver, ["subscribe"],
    [{ type: "event", key: { hostId }, event: "auth-status", listener: ["export", -1] }]]]);
  /** 宿主回一帧；返回页面先读到的和最后读到的（没被摘下时是同一帧）。 */
  const reply = async (frame) => {
    const before = port.received.length;
    port.dispatchEvent(new f.window.MessageEvent("message", { data: f.revive(frame) }));
    await flush();
    return { first: plain(port.received[before]), last: port.received.at(-1), count: port.received.length - before };
  };
  return { ...f, port, send, request, subscribe, reply, noop: ["resolve", 0.5, ["undefined"]] };
}
const plain = (value) => JSON.parse(JSON.stringify(value));
const wireOfficial = () => ({
  marketplaces: [[{ name: "openai-bundled", path: "/fixture/bundled.json", plugins: [[{ name: "system", id: "system@openai-bundled" }]] }]],
  marketplaceLoadErrors: [[]], featuredPluginIds: [["system@openai-bundled"]],
});

test("RPC 传输：列表回包按序号对上后补进索引目录，请求帧原样发出", async () => {
  const f = rpcFixture();
  // 注入晚于会话开始：前面已经发过 40 个 push，序号只能靠 pull 校准。
  f.subscribe();
  f.request("plugin/list", { cwds: [["/fixture"]], marketplaceKinds: [["local"]] });
  f.send(["pull", 41]);
  f.send(["pull", 42]);
  assert.equal(f.port.sent.length, 4);
  assert.deepEqual(plain(f.port.sent[1]), ["push", ["pipeline", 0, ["sendRequest"], ["plugin/list", { cwds: [["/fixture"]], marketplaceKinds: [["local"]] }]]]);

  const subscribed = await f.reply(["resolve", 41, ["export", -62]]);
  assert.deepEqual([subscribed.count, subscribed.first], [1, ["resolve", 41, ["export", -62]]], "别的请求的回包不动");
  const listed = await f.reply(["resolve", 42, wireOfficial()]);
  assert.deepEqual([listed.count, listed.first], [2, f.noop], "原回包先摘下，补好再投回");
  const merged = listed.last;
  assert.equal(merged[0], "resolve");
  assert.equal(merged[1], 42);
  const markets = merged[2].marketplaces[0];
  assert.deepEqual(plain(markets.map((market) => market.name)), ["openai-bundled", "codex-plus-index-public"]);
  assert.equal(markets[1].plugins[0][0].codexPlusIndexId, "original-id");
  assert.deepEqual(plain(merged[2].featuredPluginIds), [["system@openai-bundled"]]);
  assert.deepEqual(plain(f.calls), [{ path: "/plugin-market/native", payload: { method: "list-plugins", params: { cwds: ["/fixture"], marketplaceKinds: ["local"] } } }]);
  assert.deepEqual(f.toasts, []);
});

test("RPC 传输：索引里的详情和安装由本层作答，原生不执行", async () => {
  const f = rpcFixture();
  f.subscribe();
  f.request("plugin/read", { marketplacePath: sentinel(), pluginName: "sample" });
  await flush();
  const [, push, reject] = plain(f.port.sent);
  const marker = reject[2][2];
  assert.deepEqual(push, ["push", ["pipeline", 0, ["sendRequest"],
    ["plugin/read", { marketplacePath: sentinel(), pluginName: "sample" }, ["promise", -0.5]]]]);
  assert.deepEqual(reject, ["reject", -0.5, ["error", "Error", marker]]);
  assert.match(marker, /^Codex\+\+ answers plugin\/read itself/);
  // 没有任何 pull 校准过序号：靠拒绝理由认回包。
  const read = await f.reply(["reject", 309, ["error", "Error", marker]]);
  assert.deepEqual(read.first, f.noop);
  assert.deepEqual(plain(read.last), ["resolve", 309, { plugin: { summary: { ...managedSummary }, skills: [[]] } }]);
  assert.deepEqual((await f.reply(["release", -0.5, 1])).first, f.noop);
  assert.deepEqual(f.calls.map((call) => call.payload.method), ["read-plugin"]);

  // 安装用的索引标识来自列表；上面那次作答已经把序号对准（309 是第 2 个 push）。
  f.request("plugin/list", {});
  assert.equal((await f.reply(["resolve", 310, wireOfficial()])).count, 2);
  f.request("plugin/install", { marketplacePath: sentinel(), pluginName: "sample" });
  await flush();
  const installed = await f.reply(["reject", 311, plain(f.port.sent.at(-1))[2]]);
  assert.deepEqual(plain(installed.last), ["resolve", 311, { appsNeedingAuth: [[]], authPolicy: "ON_USE" }]);
  assert.deepEqual(f.calls.map((call) => call.payload.method), ["read-plugin", "list-plugins", "install-plugin"]);
  assert.deepEqual(f.toasts, ["插件已安装，新开聊天或重启 Codex 后加载。"]);

  const failing = rpcFixture(async () => ({ status: "failed", message: "下载失败" }));
  failing.subscribe();
  failing.request("plugin/read", { marketplacePath: sentinel(), pluginName: "sample" });
  await flush();
  const failed = await failing.reply(["reject", 7, plain(failing.port.sent[2])[2]]);
  assert.deepEqual(plain(failed.last), ["reject", 7, ["error", "Error", "下载失败"]]);
});

test("RPC 传输：关掉开关、别的主机、对不上形状或原生出错时原样放行", async () => {
  const off = rpcFixture(null, { enabled: false });
  off.subscribe();
  off.request("plugin/list", {});
  off.request("plugin/read", { marketplacePath: sentinel(), pluginName: "sample" });
  const untouched = async (fixture, frame, label) => {
    const reply = await fixture.reply(frame);
    assert.deepEqual([reply.count, reply.first], [1, plain(frame)], label);
  };
  await untouched(off, ["resolve", 2, wireOfficial()]);
  assert.deepEqual(plain(off.port.sent[2]), ["push", ["pipeline", 0, ["sendRequest"],
    ["plugin/read", { marketplacePath: sentinel(), pluginName: "sample" }]]]);
  assert.equal(off.port.sent.length, 3);
  assert.deepEqual(off.calls, []);

  const f = rpcFixture();
  f.subscribe("remote-host", 5);
  f.request("plugin/list", {}, 5);
  f.request("plugin/list", {}, 9);
  f.subscribe();
  f.request("plugin/list", { marketplaceKinds: [["remote"]] });
  f.request("model/list", {});
  for (const id of [2, 3, 5, 6]) await untouched(f, ["resolve", id, wireOfficial()], `push ${id}`);
  // 序号对到了别的结果上（形状不是插件列表）、原生报了别的错：都不改。
  f.request("plugin/list", {});
  await untouched(f, ["resolve", 7, { data: [[]] }]);
  f.request("plugin/installed", {});
  await untouched(f, ["reject", 8, ["error", "Error", "marketplace file is corrupt"]]);
  assert.deepEqual(f.calls, []);
});

test("RPC 传输：原生因没有 ChatGPT 登录拒绝目录时，仍给出索引目录", async () => {
  const f = rpcFixture();
  f.subscribe();
  f.request("plugin/list", {});
  const merged = (await f.reply(["reject", 2, ["error", "Error",
    "ChatGPT authentication required for remote plugin catalog; API key auth is not supported"]])).last;
  assert.deepEqual([merged[0], merged[1]], ["resolve", 2]);
  assert.deepEqual(plain(merged[2].marketplaces[0].map((market) => market.name)), ["codex-plus-index-public"]);
});
