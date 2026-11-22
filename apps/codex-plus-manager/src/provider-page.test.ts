import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

// Codex 页面「供应商」页：42 分片（与管理器共用的编辑函数打包）+ 43 分片（页面）。
const root = new URL("../../../", import.meta.url);
const fragments = new URL("assets/inject/renderer-inject/", root);
const editingSource = await readFile(new URL("42-provider-editing.generated.js", fragments), "utf8");
const pageSource = await readFile(new URL("43-provider-page.js", fragments), "utf8");
const gatewaySource = await readFile(new URL("44-provider-gateway.js", fragments), "utf8");
const renderer = await readFile(new URL("assets/inject/renderer-inject.js", root), "utf8");

function loadEditing(): any {
  return new Function(`${editingSource}\nreturn codexPlusProviderEditing;`)();
}

function pureProfile(id: string, baseUrl: string, key: string) {
  return {
    id, name: id.toUpperCase(), relayMode: "pureApi", protocol: "responses", sessionProvider: "custom",
    officialMixApiKey: false, noAuth: false, modelRoutes: [], customHeaders: [], apiKeys: [{ id: "k", name: "K", apiKey: "", stored: true, preview: "sk-…abcd" }],
    activeApiKeyId: "k", modelList: "", modelWindows: "", useCommonConfig: true,
    configContents: `model_provider = "custom"\n\n[model_providers.custom]\nname = "custom"\nwire_api = "responses"\nrequires_openai_auth = true\nbase_url = "${baseUrl}"\nexperimental_bearer_token = "${key}"\n`,
    authContents: `{"OPENAI_API_KEY":"${key}"}`,
  };
}

describe("供应商编辑函数打包（42 分片）", () => {
  it("与 relay-profile-editing.ts 保持一致", () => {
    // 用 fileURLToPath 而不是 .pathname：Windows 上 pathname 是 /D:/…，node 找不到脚本。
    execFileSync(process.execPath, [fileURLToPath(new URL("scripts/assemble-provider-editing.mjs", root)), "--check"], { stdio: "pipe" });
  });

  it("不挂全局、不访问浏览器存储", () => {
    assert.match(editingSource, /^var codexPlusProviderEditing = \(\(\) => \{/m);
    assert.doesNotMatch(editingSource, /localStorage|document\.|\bwindow\.(?:document|location|addEventListener|__)/);
    // 行首的 `})();` 会被误认成主 IIFE 收尾。
    assert.doesNotMatch(editingSource, /\n\}\)\(\);\n/);
  });

  it("字段改动按管理器的规则同步进 config.toml", () => {
    const api = loadEditing();
    // 与管理器读取时一样先 normalizeRelayProfile（补齐服务端不序列化的派生字段）。
    const draft = api.normalizeRelayProfile(pureProfile("a", "https://a.example/v1", "sk-live-aaaa1111"));
    assert.equal(draft.baseUrl, "https://a.example/v1");
    // 命名 Key 是打码的：派生出的 apiKey 为空，但改地址不能动文件里现有的 bearer（服务端保存时再按真值重写）。
    assert.equal(draft.apiKey, "");
    const next = api.applyRelayProfilePatchToFiles(draft, { baseUrl: "https://a2.example/v1" });
    assert.match(next.configContents, /base_url = "https:\/\/a2\.example\/v1"/);
    assert.match(next.configContents, /experimental_bearer_token = "sk-live-aaaa1111"/);
    assert.deepEqual(api.parseModelSuffix("deepseek-v4-pro[1M]"), { slug: "deepseek-v4-pro", window: 1000000 });
  });

  it("删除守卫与中文文案（i18n 换成原文直出）", () => {
    const api = loadEditing();
    const settings = { activeRelayId: "a", relayProfiles: [pureProfile("a", "https://a", "sk-1"), pureProfile("b", "https://b", "sk-2")].map((profile) => api.normalizeRelayProfile(profile)), aggregateRelayProfiles: [], activeAggregateRelayId: "" };
    assert.match(String(api.relayProfileDeleteIssue(settings, "a")), /当前|使用/);
    assert.equal(api.relayProfileDeleteIssue(settings, "b"), null);
    assert.deepEqual(api.removeRelayProfile(settings, "b").relayProfiles.map((profile: any) => profile.id), ["a"]);
  });

  it("网关接管期间保存直连档案，切回混合模式恢复原来的配置（#3190）", () => {
    const api = loadEditing();
    const gateway = api.normalizeRelayProfile(pureProfile("codex-plus-gateway", "https://gateway.test", "fixture"));
    const mixed = api.normalizeRelayProfile({ ...pureProfile("direct", "https://original.test", "fixture"),
      relayMode: "official", officialMixApiKey: true, authContents: "", modelRoutes: [] });
    const settings = { activeRelayId: gateway.id, relayProfiles: [gateway], aggregateRelayProfiles: [], activeAggregateRelayId: "" };
    const official = api.applyRelayProfilePatchToFiles(mixed, { officialMixApiKey: false });
    assert.equal(official.configContents, "");
    const saved = api.addRelayProfile(settings, official);
    assert.equal(saved.activeRelayId, gateway.id);
    const loaded = saved.relayProfiles.find((profile: any) => profile.id === mixed.id);
    const restored = api.applyRelayProfilePatchToFiles({ ...loaded, upstreamBaseUrl: "", baseUrl: "", apiKey: "" }, { officialMixApiKey: true });
    assert.equal(restored.configContents, mixed.configContents);
    assert.equal(restored.upstreamBaseUrl, "https://original.test");
    assert.equal(restored.inactiveMixedApi, undefined);
  });
});

/** 在桩环境里执行 43 分片，拿到内部函数。 */
function loadPage(stubs: { postJson: (path: string, payload: any) => any; ask?: (question: any) => any }) {
  const calls: Array<{ path: string; payload: any }> = [];
  const toasts: string[] = [];
  const document = { querySelector: () => null, querySelectorAll: () => [], activeElement: null, hidden: false };
  const api = new Function(
    "codexPlusProviderEditing", "postJson", "codexPlusAsk", "showToast", "escapeHtml", "document", "codexPlusMenuIcons",
    `${pageSource}\n${gatewaySource}\nreturn { state: codexPlusProviderPageState, applyList: codexPlusProviderApplyList, commit: codexPlusProviderCommit,
      use: codexPlusProviderUse, selectKey: codexPlusProviderSelectKey, applyModelLines: codexPlusProviderApplyModelLines,
      modelLines: codexPlusProviderModelLines, duplicate: codexPlusProviderDuplicate, renderPage: renderCodexPlusProviderPage,
      gw: codexPlusGatewayPageState, gwApply: codexPlusGatewayApplyResult, gwSelect: codexPlusGatewaySelect, gwSave: codexPlusGatewaySave,
      gwEditable: codexPlusGatewayEditable, gwConnect: codexPlusGatewayConnect, gwOpenDisconnect: codexPlusGatewayOpenDisconnect,
      gwApplyDisconnect: codexPlusGatewayApplyDisconnect, gwInput: codexPlusGatewayHandleInput, gwStartNew: codexPlusGatewayStartNew,
      modelEditor: codexPlusProviderModelEditor, windowLabel: codexPlusProviderWindowLabel,
      detail: renderCodexPlusProviderDetail, gwDetail: renderCodexPlusGatewayDetail, menuChanged: codexPlusProviderMenuChanged };`,
  )(
    loadEditing(),
    async (path: string, payload: any) => {
      calls.push({ path, payload: JSON.parse(JSON.stringify(payload)) });
      return stubs.postJson(path, payload);
    },
    async (question: any) => stubs.ask?.(question) ?? null,
    (message: string) => toasts.push(message),
    (value: unknown) => String(value ?? ""),
    document,
    // 41 分片的菜单图标，桩里不加载 41。
    { chevron: "<svg data-icon=\"chevron\"></svg>", check: "<svg data-icon=\"check\"></svg>" },
  );
  return { ...api, calls, toasts };
}

function listResult(revision = "r1", activeRelayId = "a") {
  return {
    status: "ok", revision, gatewayRelayId: "codex-plus-gateway",
    settings: {
      activeRelayId, relayProfilesEnabled: true, aggregateRelayProfiles: [], activeAggregateRelayId: "",
      relayProfiles: [pureProfile("a", "https://a.example/v1", "sk-a-live-0000"), pureProfile("b", "https://b.example/v1", "sk-b-live-1111")],
    },
    defaults: { relayBaseUrl: "https://default.example/v1", relayProfiles: [] },
  };
}

describe("供应商页（43 分片）", () => {
  it("切换遇到 live 冲突时问用户，再带着选择重提", async () => {
    let commits = 0;
    const page = loadPage({
      postJson: (path) => {
        if (path !== "/providers/commit") return listResult();
        commits += 1;
        return commits === 1
          ? { status: "conflict", liveConflict: { profileId: "a", profileName: "A" }, message: "冲突" }
          : { ...listResult("r2", "b"), switched: true, message: "供应商已切换。" };
      },
      ask: () => ({ value: "discardLiveChanges" }),
    });
    page.applyList(listResult());
    await page.use("b");
    const sent = page.calls.filter((call: { path: string; payload: any }) => call.path === "/providers/commit");
    assert.equal(sent.length, 2);
    assert.equal(sent[0].payload.activate, "b");
    assert.equal(sent[0].payload.revision, "r1");
    assert.equal(sent[0].payload.conflictResolution, "stop");
    assert.equal(sent[1].payload.conflictResolution, "discardLiveChanges");
    assert.equal(page.state.settings.activeRelayId, "b");
    assert.match(page.state.status, /新对话立即生效/);
  });

  it("用户取消冲突弹窗时不再提交", async () => {
    const page = loadPage({
      postJson: (path) => (path === "/providers/commit" ? { status: "conflict", liveConflict: { profileId: "a", profileName: "A" } } : listResult()),
      ask: () => null,
    });
    page.applyList(listResult());
    await page.use("b");
    assert.equal(page.calls.filter((call: { path: string; payload: any }) => call.path === "/providers/commit").length, 1);
    assert.equal(page.state.settings.activeRelayId, "a");
    assert.match(page.state.status, /已取消切换/);
  });

  it("选中打码的已存 Key 不改文件，交给服务端按真值重写", () => {
    const page = loadPage({ postJson: () => listResult() });
    const result = listResult();
    result.settings.relayProfiles[0].apiKeys.push({ id: "k2", name: "K2", apiKey: "", stored: true, preview: "sk-…2222" });
    page.applyList(result);
    const before = page.state.draft.configContents;
    page.selectKey("k2");
    assert.equal(page.state.draft.activeApiKeyId, "k2");
    assert.equal(page.state.draft.configContents, before);
    assert.equal(page.state.dirty, true);
  });

  it("模型列表按 [1M] 后缀拆出窗口，回显时拼回去", () => {
    const page = loadPage({ postJson: () => listResult() });
    page.applyList(listResult());
    page.applyModelLines("deepseek-v4-pro[1M]\nqwen3\nqwen3\n");
    assert.equal(page.state.draft.modelList, "deepseek-v4-pro\nqwen3");
    assert.deepEqual(JSON.parse(page.state.draft.modelWindows), { "deepseek-v4-pro": "1000000" });
    assert.equal(page.modelLines(page.state.draft), "deepseek-v4-pro[1000000]\nqwen3");
  });

  it("模型行编辑器：一行一个，星标跟默认模型，窗口回显成下拉", () => {
    const page = loadPage({ postJson: () => listResult() });
    page.applyList(listResult());
    page.applyModelLines("deepseek-v4-pro[1M]\nqwen3[272000]\nplain");
    const html = page.modelEditor({ ...page.state.draft, model: "qwen3" });
    assert.equal((html.match(/data-codex-provider-model-row/g) || []).length, 3);
    assert.match(html, /data-active="true" data-codex-provider-model-default="qwen3"/);
    assert.match(html, /data-active="false" data-codex-provider-model-default="plain"/);
    // 非预设窗口（272000）也要作为选项保留，否则回显会丢。
    assert.match(html, /data-codex-provider-menu-toggle="model-window" data-value="272000"[^>]*><span class="codex-plus-provider-menu-label">272K<\/span>/);
    assert.match(html, /data-codex-provider-menu-toggle="model-window" data-value="1000000"[^>]*><span class="codex-plus-provider-menu-label">1M<\/span>/);
    assert.match(html, /\["272000","272K"\]/, "非预设窗口也在菜单选项里");
    assert.equal(page.windowLabel("1000000"), "1M");
  });

  it("下拉都用 Codex 原生样式的菜单，不用原生 select；选中后写回草稿", () => {
    const page = loadPage({ postJson: () => listResult() });
    page.applyList(listResult());
    const direct = page.detail();
    assert.doesNotMatch(direct, /<select|<option/);
    assert.match(direct, /class="codex-plus-menu-trigger" data-codex-provider-menu-toggle="mode" data-value="pureApi"/);
    assert.match(direct, /data-codex-provider-menu-toggle="protocol" data-value="responses"/);
    // 有下拉的行是 div 不是 label：label 会把整行的点击转给按钮，点空白处也会弹菜单。
    assert.doesNotMatch(direct, /<label class="codex-plus-provider-line"[^>]*>(?:(?!<\/label>)[\s\S])*data-codex-provider-menu-toggle/);
    page.menuChanged("protocol", "chatCompletions");
    assert.equal(page.state.draft.protocol, "chatCompletions");
    page.menuChanged("mode", "mixedApi");
    assert.equal(page.state.draft.officialMixApiKey, true);
    assert.equal(page.state.dirty, true);
    page.gwApply(gatewayResult());
    page.gwSelect("gw-a");
    const gateway = page.gwDetail();
    assert.doesNotMatch(gateway, /<select|<option/);
    assert.match(gateway, /data-codex-provider-menu-toggle="gw-test-model" data-value=""[^>]*data-codex-gateway-test-model="true"/);
    page.menuChanged("gw-protocol", "anthropicMessages");
    assert.equal(page.gw.draft.protocol, "anthropicMessages");
    assert.equal(page.gw.dirty, true);
  });

  it("复制供应商时让服务端按来源还原 Key", async () => {
    const page = loadPage({ postJson: (path) => (path === "/providers/commit" ? listResult("r2") : listResult()) });
    page.applyList(listResult());
    await page.duplicate("b");
    const sent = page.calls.find((call: { path: string; payload: any }) => call.path === "/providers/commit")!;
    const copy = sent.payload.relayProfiles.find((profile: any) => !["a", "b"].includes(profile.id));
    assert.ok(copy, "应提交复制出的新供应商");
    assert.equal(sent.payload.keySources[copy.id], "b");
  });

  it("没改过的供应商原样回传服务端版本，改过的才提交归一化结果", async () => {
    const page = loadPage({ postJson: (path) => (path === "/providers/commit" ? listResult("r2") : listResult()) });
    const raw = listResult();
    page.applyList(raw);
    await page.duplicate("b");
    const sent = page.calls.find((call: { path: string; payload: any }) => call.path === "/providers/commit")!;
    // a、b 都没改：必须与服务端下发的原始 JSON 完全一致，服务端才不会误判当前供应商变了。
    assert.deepEqual(sent.payload.relayProfiles.find((profile: any) => profile.id === "a"), raw.settings.relayProfiles[0]);
    assert.deepEqual(sent.payload.relayProfiles.find((profile: any) => profile.id === "b"), raw.settings.relayProfiles[1]);
  });

  it("新入口登记进所有写死内置 id 的地方", () => {
    assert.match(renderer, /codexPlusRailProviderId = "codex-plus-rail-provider"/);
    assert.match(renderer, /\{ id: codexPlusRailProviderId, label: "供应商", iconMarkup: icons\.provider, withStatus: false, onActivate: openCodexPlusProviderPage \}/);
    assert.match(renderer, /\[codexPlusRailProviderId, "provider"\],/);
    assert.match(renderer, /if \(tab === codexPlusProviderTab\) return "provider";/);
    assert.match(renderer, /\[codexPlusRailNavId, codexPlusRailProviderId, codexPlusRailRemoteId, codexPlusRailWeixinId, codexPlusRailTasksId, codexPlusRailExtensionsId, codexPlusRailSponsorId, codexPlusRailPluginMarketId\]\.forEach/);
    // 两处图标栏点击监听都要排除新入口，否则点它会把刚打开的页面关掉。
    assert.equal(renderer.split("#${codexPlusRailNavId}, #${codexPlusRailProviderId}, #${codexPlusRailExtensionsId}, #${codexPlusRailSponsorId}").length - 1, 2);
    assert.match(renderer, /\[codexPlusRailNavId, codexPlusRailProviderId, codexPlusRailRemoteId, codexPlusRailWeixinId, codexPlusRailTasksId, codexPlusRailExtensionsId, codexPlusRailSponsorId\]\.forEach\(\(id\) => \{/);
    assert.match(renderer, /#\$\{codexPlusRailProviderId\} > button/);
    assert.match(pageSource, /data-codex-plus-active-tab="\$\{codexPlusProviderTab\}"/);
    assert.match(pageSource, /window\.__codexPlusExtensionPageCleanup = \(\) => \{/);
  });
});

function poolEntry(id: string, extra: Record<string, unknown> = {}) {
  return {
    id, name: id.toUpperCase(), baseUrl: `https://${id}.example/v1`, protocol: "chatCompletions", official: false,
    models: ["m-1", "m-2"], keyCount: 1, enabled: true, source: "custom", keys: [{ id: "default", masked: "sk-…1111" }],
    price: { currency: "USD", input: 1.5, cachedInput: 0.5, output: 6 }, proxy: "", ...extra,
  };
}

function gatewayResult(pool = [poolEntry("gw-a"), poolEntry("acct", { account: "copilot", plan: "Copilot Pro" })], codexEnabled = true) {
  return {
    status: "ok", message: "", port: 57321, helperRunning: true, relayProfilesEnabled: true, pool, groups: [], agents: [],
    health: { keys: [], providers: [] }, codexProviders: [], claudeProviders: [], grokProfiles: [],
    gateway: { providers: [], groups: [], bindings: { codex: { enabled: codexEnabled, model: "gw-a/m-1", extraModels: [], claudeRoles: { opus: "", sonnet: "", haiku: "", subagent: "" }, unknownModel: "", poolExclude: [] } } },
  };
}

describe("供应商页的网关页签（44 分片）", () => {
  it("账号 / 原生登录 / 拓展条目只读，普通 API Key 供应商可编辑", () => {
    const page = loadPage({ postJson: () => listResult() });
    assert.equal(page.gwEditable(poolEntry("x")), true);
    assert.equal(page.gwEditable(poolEntry("x", { account: "copilot" })), false);
    assert.equal(page.gwEditable(poolEntry("x", { account: "chatgpt" })), false);
    assert.equal(page.gwEditable(poolEntry("x", { official: true })), false);
    assert.equal(page.gwEditable(poolEntry("x", { source: "native:external" })), false);
    assert.equal(page.gwEditable(poolEntry("x", { source: "extension:demo" })), false);
  });

  it("编辑保存原样带回价格与代理，Key 留空表示不改", async () => {
    const page = loadPage({ postJson: (path, payload) => (payload?.command === "update_gateway_provider" ? gatewayResult() : listResult()) });
    page.gwApply(gatewayResult());
    page.gwSelect("gw-a");
    page.gwInput({ target: { dataset: { codexGatewayField: "name" }, value: "改名" } });
    await page.gwSave();
    const sent = page.calls.find((call: { path: string; payload: any }) => call.payload?.command === "update_gateway_provider")!;
    assert.equal(sent.path, "/providers/gateway");
    assert.equal(sent.payload.args.id, "gw-a");
    assert.equal(sent.payload.args.input.name, "改名");
    assert.deepEqual(sent.payload.args.input.price, { currency: "USD", input: 1.5, cachedInput: 0.5, output: 6 });
    assert.deepEqual(sent.payload.args.input.keys, []);
    assert.equal(page.gw.dirty, false);
    assert.match(page.gw.status, /下一条请求/);
  });

  it("直连与网关两个页签的详情都用原生设置页的卡片，删除收在底部「危险操作」", () => {
    const page = loadPage({ postJson: () => listResult() });
    page.applyList(listResult("r1", "a"));
    page.state.selectedId = "b";
    page.applyList(listResult("r1", "a"));
    const direct = page.detail();
    assert.doesNotMatch(direct, /codex-plus-provider-section"|codex-plus-provider-field/, "不再用旧的平铺小节");
    assert.match(direct, /codex-plus-provider-card/);
    const directHeader = direct.slice(0, direct.indexOf("</div></div>"));
    assert.doesNotMatch(directHeader, /data-codex-provider-action="delete"/, "删除不放在标题栏");
    assert.match(direct, /危险操作[\s\S]*data-codex-provider-action="delete"/);
    page.gwApply(gatewayResult());
    for (const id of ["gw-a", "acct"]) {
      page.gwSelect(id);
      const html = page.gwDetail();
      assert.doesNotMatch(html, /codex-plus-provider-section"|codex-plus-provider-field/, `${id} 不再用旧的平铺小节`);
      assert.match(html, /codex-plus-provider-card/);
      assert.match(html, /危险操作[\s\S]*data-codex-gateway-action="delete"/);
      assert.match(html, /data-codex-gateway-test-model="true"/, "「测试」读的下拉还在");
    }
  });

  it("新建必须填 Key，提交 create_gateway_provider{source:custom}", async () => {
    const page = loadPage({ postJson: (path, payload) => (payload?.command === "create_gateway_provider" ? gatewayResult([poolEntry("gw-a"), poolEntry("new-one")]) : listResult()) });
    page.gwApply(gatewayResult());
    page.gwStartNew();
    for (const [field, value] of [["name", "新的"], ["baseUrl", "https://new.example/v1"], ["model", "m-x"]]) {
      page.gwInput({ target: { dataset: { codexGatewayField: field }, value } });
    }
    await page.gwSave();
    assert.match(page.gw.status, /API Key/, "没填 Key 不能新建");
    page.gwInput({ target: { dataset: { codexGatewayField: "keys" }, value: "sk-new-key-0001" } });
    await page.gwSave();
    const sent = page.calls.find((call: { path: string; payload: any }) => call.payload?.command === "create_gateway_provider")!;
    assert.equal(sent.payload.args.source, "custom");
    assert.deepEqual(sent.payload.args.input.keys, ["sk-new-key-0001"]);
    assert.equal(page.gw.selectedId, "new-one");
  });

  it("接入网关沿用上次的接入设置并只作用在 Codex 上", async () => {
    const page = loadPage({ postJson: (path, payload) => (payload?.command ? gatewayResult() : listResult()) });
    page.gwApply(gatewayResult(undefined, false));
    await page.gwConnect();
    const sent = page.calls.find((call: { path: string; payload: any }) => call.payload?.command === "save_gateway_binding")!;
    assert.equal(sent.payload.args.agent, "codex");
    assert.equal(sent.payload.args.reconnect, true);
    assert.equal(sent.payload.args.binding.enabled, true);
    assert.equal(sent.payload.args.binding.model, "gw-a/m-1");
  });

  it("断开先取预览，确认时带上预览 token；预览列不出逐项改动时走完整切换", async () => {
    for (const [preview, command] of [
      [{ agent: "codex", previewToken: "tok-1", files: [{ path: "config.toml", kind: "config", diff: "-a\n+b" }], warnings: [], canDisconnect: true }, "disable_gateway_binding"],
      [{ agent: "codex", previewToken: "tok-2", files: [], warnings: [], canDisconnect: false, fullSwitch: { relayId: "a", name: "A" } }, "disable_gateway_binding_full_switch"],
    ] as const) {
      const page = loadPage({ postJson: (path, payload) => (payload?.command === "preview_gateway_disconnect" ? { status: "ok", preview } : payload?.command ? gatewayResult() : listResult()) });
      page.gwApply(gatewayResult());
      await page.gwOpenDisconnect();
      assert.equal(page.gw.disconnect.preview.previewToken, preview.previewToken);
      await page.gwApplyDisconnect();
      const sent = page.calls.find((call: { path: string; payload: any }) => call.payload?.command === command)!;
      assert.ok(sent, command);
      assert.deepEqual(sent.payload.args, { agent: "codex", previewToken: preview.previewToken });
      assert.equal(page.gw.disconnect, null);
    }
  });

  it("网关模式下在直连页签点「使用」：先断开并还原，成功后再切换", async () => {
    let gatewayMode = true;
    const page = loadPage({
      postJson: (path, payload) => {
        if (payload?.command === "preview_gateway_disconnect") return { status: "ok", preview: { agent: "codex", previewToken: "tok", files: [], warnings: [], canDisconnect: true } };
        if (payload?.command === "disable_gateway_binding") { gatewayMode = false; return gatewayResult(undefined, false); }
        if (payload?.command) return gatewayResult();
        if (path === "/providers/commit") return { ...listResult("r2", "b"), switched: true, gatewayMode };
        return { ...listResult(), gatewayMode };
      },
    });
    page.applyList({ ...listResult(), gatewayMode: true });
    await page.use("b");
    assert.equal(page.calls.some((call: { path: string }) => call.path === "/providers/commit"), false, "还没确认断开时不能直接切换");
    await page.gwApplyDisconnect();
    const commit = page.calls.find((call: { path: string; payload: any }) => call.path === "/providers/commit")!;
    assert.equal(commit.payload.activate, "b");
  });

  it("网关路由不开放给拓展、Codex 页面用 /providers/gateway", () => {
    assert.match(gatewaySource, /postJson\("\/providers\/gateway", \{ command, args \}\)/);
    assert.match(renderer, /"\/providers\/gateway" =>|providers\/gateway/);
  });
});
