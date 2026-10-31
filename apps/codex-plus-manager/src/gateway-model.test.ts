import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import {
  isDirectProxy,
  proxyAddressError,
  agentModels,
  bindingFromDraft,
  blankBinding,
  connectedAgents,
  dispatchStatus,
  draftFromBinding,
  formatRemaining,
  keyLabel,
  activeSponsorIds,
  gatewayPresets,
  groupModelId,
  memberLabel,
  memberStatus,
  modelDispatch,
  modelDisplay,
  prettyModelName,
  modelRoutable,
  parseContextWindow,
  parseKeyLimit,
  formatQueueWait,
  queueWaitMs,
  foreignCompactionDropped,
  FOREIGN_COMPACTION_DROPPED_REASON,
  pickerSections,
  presetAdded,
  presetHost,
  providerDraftFromEntry,
  providerDraftFromPreset,
  providerInput,
  routeNarrative,
  slugify,
  splitModels,
  validateDraft,
  validateProviderDraft,
  type GatewayPayload,
} from "./gateway-model.ts";

const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");

function payload(): GatewayPayload {
  return {
    gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
    port: 57321,
    helperRunning: true,
    pool: [
      { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com/", protocol: "chatCompletions", official: false, models: ["deepseek-v4-pro", "deepseek-v4-flash"], keyCount: 2, enabled: true, source: "" },
      { id: "glm", name: "GLM", baseUrl: "https://glm.test/v1", protocol: "anthropicMessages", official: false, models: ["glm-5.3"], keyCount: 1, enabled: true, source: "" },
      { id: "bare", name: "Bare", baseUrl: "https://bare.test/v1", protocol: "responses", official: false, models: [], keyCount: 1, enabled: true, source: "" },
    ],
    groups: [{ id: "daily", name: "Daily", mode: "inOrder", members: [{ model: "deepseek/deepseek-v4-flash", weight: 1 }, { model: "glm-5.3", weight: 1 }] }],
    agents: [],
    health: { keys: [], providers: [] },
    codexProviders: [],
    claudeProviders: [],
    grokProfiles: [],
  };
}

const initials = (name: string) => name.slice(0, 2).toUpperCase();

test("上下文窗口支持 k / M 写法", () => {
  assert.equal(parseContextWindow(""), null);
  assert.equal(parseContextWindow("200000"), 200000);
  assert.equal(parseContextWindow("200k"), 200000);
  assert.equal(parseContextWindow("1M"), 1000000);
  assert.ok(Number.isNaN(parseContextWindow("abc") as number));
});

test("表单与绑定互转：挑选、token 与恢复现场沿用", () => {
  const previous = { ...blankBinding(), enabled: true, token: "cpx-gw-claude-x", restore: { claudePreviousProviderId: "kimi" } };
  const draft = { ...draftFromBinding(previous), model: " group/daily ", haiku: "deepseek-v4-flash", contextWindow: "1M", poolExclude: ["glm"] };
  const binding = bindingFromDraft(draft, previous);
  assert.equal(binding.model, "group/daily");
  assert.equal(binding.claudeRoles.haiku, "deepseek-v4-flash");
  assert.equal(binding.contextWindow, 1000000);
  assert.deepEqual(binding.poolExclude, ["glm"]);
  assert.equal(binding.token, "cpx-gw-claude-x");
  assert.equal(binding.restore?.claudePreviousProviderId, "kimi");
  assert.equal("target" in binding, false, "绑定不再指向某个供应商：所有 Agent 共用整个池");
  assert.deepEqual(splitModels(" a \n\n b,a"), ["a", "b"]);
});

test("Agent 能用的模型：池里打开的供应商的模型 + 有成员的路由组；挑选排除的不算", () => {
  assert.deepEqual(agentModels(payload()), ["deepseek-v4-pro", "deepseek-v4-flash", "glm-5.3", "group/daily"]);
  assert.deepEqual(agentModels(payload(), ["deepseek"]), ["glm-5.3", "group/daily"]);
  const data = payload();
  data.pool[1].enabled = false;
  data.groups.push({ id: "empty", name: "Empty", mode: "rotate", members: [] });
  assert.deepEqual(agentModels(data), ["deepseek-v4-pro", "deepseek-v4-flash", "group/daily"]);
  assert.equal(modelRoutable(payload(), "deepseek/anything-new"), true, "<供应商 id>/<模型> 总能路由");
  assert.equal(modelRoutable(payload(), "unknown/anything"), false);
  assert.equal(modelRoutable(payload(), "nope"), false);
});

test("校验：池里要有能用的供应商，模型要能路由，窗口格式正确", () => {
  const data = payload();
  const base = draftFromBinding(undefined);
  assert.equal(validateDraft(base, data), null, "不填模型时用池里第一个模型");
  assert.equal(validateDraft({ ...base, model: "glm-5.3" }, data), null);
  assert.equal(validateDraft({ ...base, model: "group/daily" }, data), null);
  assert.equal(validateDraft({ ...base, model: "deepseek/deepseek-v5" }, data), null);
  assert.equal(validateDraft({ ...base, model: "nope" }, data), "供应商池里没有这个模型");
  assert.equal(validateDraft({ ...base, poolExclude: ["deepseek", "glm", "bare"] }, data), "供应商池是空的，先到「供应商」里添加");
  assert.equal(validateDraft({ ...base, contextWindow: "12" }, data), "上下文窗口格式不对，例如 200000、200k 或 1M");
});

test("模型选择器：每个供应商一组，路由组作为 group/ 模型在后；未知输入给出 <供应商>/<模型>", () => {
  const all = pickerSections(payload(), "", initials);
  assert.deepEqual(all.map((section) => section.key), ["provider:deepseek", "provider:glm", "group"]);
  assert.deepEqual(all[0].items.map((item) => item.model), ["deepseek-v4-pro", "deepseek-v4-flash"]);
  assert.deepEqual(all[2].items[0], { model: "group/daily", label: "Daily", sub: "group/daily", avatar: "DA" });
  assert.deepEqual(pickerSections(payload(), "flash", initials)[0].items.map((item) => item.model), ["deepseek-v4-flash"]);
  const custom = pickerSections(payload(), "kimi-k3", initials).at(-1);
  assert.equal(custom?.key, "custom");
  assert.deepEqual(custom?.items.map((item) => item.model), ["deepseek/kimi-k3", "glm/kimi-k3", "bare/kimi-k3"]);
  assert.equal(pickerSections(payload(), "deepseek-v4-pro", initials).some((section) => section.key === "custom"), false);
  // 编辑路由组时不能选它自己；挑选排除的供应商不出现。
  assert.equal(pickerSections(payload(), "", initials, [], "daily").some((section) => section.key === "group"), false);
  assert.deepEqual(pickerSections(payload(), "", initials, ["deepseek"]).map((section) => section.key), ["provider:glm", "group"]);
});

test("模型显示：模型名 + 提供它的供应商；<供应商>/<模型> 拆开；路由组显示组名", () => {
  assert.deepEqual(modelDisplay(payload(), ""), { title: "DeepSeek-V4-Pro", sub: "DeepSeek", group: false });
  assert.deepEqual(modelDisplay(payload(), "glm-5.3"), { title: "GLM-5.3", sub: "GLM", group: false });
  assert.deepEqual(modelDisplay(payload(), "deepseek/deepseek-v4-flash"), { title: "DeepSeek-V4-Flash", sub: "DeepSeek", group: false });
  assert.deepEqual(modelDisplay(payload(), "group/daily"), { title: "Daily", sub: "group/daily", group: true });
  assert.equal(memberLabel(payload(), "deepseek/deepseek-v4-flash"), "DeepSeek · DeepSeek-V4-Flash");
  assert.equal(memberLabel(payload(), "group/daily"), "Daily");
  assert.equal(groupModelId("daily"), "group/daily");
  assert.equal(slugify(" 系统 默认 "), "系统-默认");
});

test("模型名显示成正式写法，只影响显示", () => {
  assert.equal(prettyModelName("gpt-6.1-sol"), "GPT-6.1-Sol");
  assert.equal(prettyModelName("kimi-k2-turbo"), "Kimi-K2-Turbo");
  assert.equal(prettyModelName("qwen3-235b-a22b"), "Qwen3-235B-A22B");
  assert.equal(prettyModelName("o3-mini"), "o3-Mini");
  assert.equal(prettyModelName("anthropic/claude-sonnet-5[1M]"), "anthropic/Claude-Sonnet-5[1M]");
  assert.equal(prettyModelName("MiniMax-M2"), "MiniMax-M2", "本来就带大写的原样保留");
});

function withActivity(): GatewayPayload {
  const data = payload();
  data.gateway.bindings = {
    claude: { ...blankBinding(), enabled: true },
    grok: { ...blankBinding(), enabled: true },
  };
  data.agents = [
    { agent: "claude", name: "Claude Code", enabled: true, baseUrl: "", defaultModel: "", models: [] },
    { agent: "grok", name: "Grok", enabled: false, baseUrl: "", defaultModel: "", models: [] },
  ];
  data.activity = {
    agents: [{
      agent: "claude",
      requests: 3,
      rerouted: 1,
      errors: 0,
      inFlight: [],
      last: {
        requestedModel: "claude-sonnet-5",
        model: "group/daily",
        groupId: "daily",
        planned: [{ relayId: "deepseek", relayName: "DeepSeek", model: "deepseek-v4-flash" }, { relayId: "glm", relayName: "GLM", model: "glm-5.3" }],
        steps: [
          { relayId: "deepseek", relayName: "DeepSeek", keyId: "key-1", model: "deepseek-v4-flash", outcome: "failed", failure: "rate_limited", status: 429, elapsedMs: 100 },
          { relayId: "glm", relayName: "GLM", keyId: "default", model: "glm-5.3", outcome: "answered", status: 200, elapsedMs: 900 },
        ],
        result: "answered",
        finishedAtMs: 1,
        elapsedMs: 1200,
      },
    }],
  };
  return data;
}

test("成员状态：正在回答 > 熔断 / Key 全部冷却 > 10 分钟内哪个 Agent 用过 > Key 数", () => {
  const data = withActivity();
  assert.deepEqual(memberStatus(data, "glm", 60_000), { state: "answered", text: "{0} 刚用过", args: ["Claude Code"] });
  assert.equal(memberStatus(data, "glm", 11 * 60_000).state, "idle", "超过 10 分钟不再显示，回到 Key 数");
  assert.deepEqual(memberStatus(data, "deepseek"), { state: "idle", text: "{0} 个 Key", args: [2] });
  data.health.keys = [{ relayId: "deepseek", keyId: "key-1", state: "cooling", remainingMs: 35_000, lastFailure: "rate_limited" }];
  assert.deepEqual(memberStatus(data, "deepseek"), { state: "idle", text: "{0} 个 Key · {1} 个冷却中", args: [2, 1] });
  data.health.keys.push({ relayId: "deepseek", keyId: "key-2", state: "cooling", remainingMs: 20_000, lastFailure: "rate_limited" });
  assert.deepEqual(memberStatus(data, "deepseek"), { state: "cooling", text: "被限流 · {0} 后恢复", args: ["20s"] });
  data.health.providers = [{ relayId: "glm", breakerOpen: true, remainingMs: 60_000, consecutiveErrors: 3 }];
  assert.equal(memberStatus(data, "glm").state, "cooling");
  data.activity!.agents[0].inFlight = ["glm"];
  assert.deepEqual(memberStatus(data, "glm"), { state: "busy", text: "正在回答…", args: [] });
  assert.deepEqual(memberStatus(payload(), "bare"), { state: "idle", text: "{0} 个 Key", args: [1] });
});

test("「模型」页矩阵：每个模型列出提供它的供应商，按池顺序；有备用的排前面，关掉的不算，标出排除它的 Agent", () => {
  const data = withActivity();
  assert.deepEqual(connectedAgents(data), ["claude"]);
  data.pool[2].models = ["glm-5.3", "GLM-5.3", "bare-1"];
  data.pool.push({ id: "off", name: "Off", baseUrl: "https://off.test/v1", protocol: "responses", official: false, models: ["glm-5.3"], keyCount: 1, enabled: false, source: "" });
  data.gateway.bindings.claude = { ...data.gateway.bindings.claude!, poolExclude: ["bare"] };
  const rows = modelDispatch(data);
  assert.deepEqual(rows.map((row) => row.model), ["glm-5.3", "deepseek-v4-pro", "deepseek-v4-flash", "bare-1"]);
  assert.deepEqual(rows[0].providers.map((provider) => provider.id), ["glm", "bare"], "大小写不同算同一个模型；关掉的供应商不在里面");
  assert.deepEqual(rows[0].providers[1].excludedBy, ["claude"]);
  assert.deepEqual(rows[0].providers[0].excludedBy, []);
});

test("上一个请求的路由说明：先列计划，再逐个尝试，最后谁回答", () => {
  const data = withActivity();
  const lines = routeNarrative("claude", data.activity!.agents[0].last!, data);
  assert.deepEqual(lines.map((line) => line.tone), ["muted", "step", "step", "ok"]);
  assert.deepEqual(lines[0].args, ["Claude Code", "claude-sonnet-5", "Daily", "DeepSeek/deepseek-v4-flash, GLM/glm-5.3"]);
  // 失败原因是界面文案，标成 { zh } 交给界面翻译；供应商名、Key id 原样显示。
  assert.deepEqual(lines[2].args, ["DeepSeek", "key-1", { zh: "被限流" }, " · HTTP 429"]);
  assert.deepEqual(lines[3].args, ["GLM", "glm-5.3", "1.2 s"]);
});

test("调度表的状态按模型：「刚用过」只标在真正被请求的模型上，冷却按整家供应商", () => {
  const data = withActivity();
  data.pool[1].models = ["glm-5.3", "glm-5.3-air"];
  assert.deepEqual(dispatchStatus(data, "glm", "GLM-5.3", 60_000), { state: "answered", text: "{0} 刚用过", args: ["Claude Code"] });
  assert.equal(dispatchStatus(data, "glm", "glm-5.3-air", 60_000), null, "同一家的其它模型不标");
  assert.equal(dispatchStatus(data, "glm", "glm-5.3", 11 * 60_000), null, "超过 10 分钟不标");
  data.health.providers = [{ relayId: "glm", breakerOpen: true, remainingMs: 30_000 } as GatewayPayload["health"]["providers"][number]];
  assert.equal(dispatchStatus(data, "glm", "glm-5.3-air", 60_000)?.state, "cooling");
});

test("冷却剩余时间显示", () => {
  assert.equal(formatRemaining(0), "");
  assert.equal(formatRemaining(1500), "2s");
  assert.equal(formatRemaining(90_000), "2m");
  assert.equal(formatRemaining(3_900_000), "1h5m");
});

test("内置预设：厂商在前、中转其次、本机最后；来源或地址相同的标记为已添加", () => {
  const presets = gatewayPresets();
  const sections = presets.map((preset) => preset.section);
  assert.equal(sections.indexOf("relay") > sections.lastIndexOf("vendor"), true);
  assert.equal(sections.indexOf("local") > sections.lastIndexOf("relay"), true);
  const anthropic = presets.find((preset) => preset.key === "anthropic")!;
  assert.equal(anthropic.protocol, "anthropicMessages");
  const deepseek = presets.find((preset) => preset.key === "deepseek")!;
  assert.equal(deepseek.section, "vendor");
  assert.equal(presetAdded(payload(), deepseek), true, "末尾斜杠和大小写不影响判断");
  assert.equal(presetAdded(payload(), anthropic), false);
  const data = payload();
  data.pool[2].source = "preset:anthropic";
  assert.equal(presetAdded(data, anthropic), true, "按来源判断");
  assert.equal(new Set(presets.map((preset) => preset.key)).size, presets.length, "预设 key 不重复");
  assert.equal(presetHost("https://api.deepseek.com/"), "api.deepseek.com");
  assert.equal(presetHost("http://127.0.0.1:11434/v1"), "localhost:11434");
});

test("供应商表单：多 Key 每行一个；本机服务预填 Key；编辑时 Key 可以留空", () => {
  const ollama = gatewayPresets().find((preset) => preset.key === "ollama")!;
  const draft = providerDraftFromPreset(ollama);
  assert.equal(draft.keys, "ollama");
  assert.equal(validateProviderDraft(draft), "至少填一个模型");
  assert.equal(validateProviderDraft({ ...draft, model: "qwen3" }), null);
  assert.equal(validateProviderDraft({ ...draft, model: "qwen3", baseUrl: "api.x" }), "地址要以 http:// 或 https:// 开头");
  assert.equal(validateProviderDraft({ ...draft, model: "qwen3", keys: " " }), "请填写 API Key");
  assert.equal(validateProviderDraft(providerDraftFromPreset(null)), "请填写名称");
  const editing = providerDraftFromEntry(payload().pool[0]);
  assert.equal(editing.keys, "", "编辑时不回显 Key");
  assert.equal(validateProviderDraft(editing, true), null);
  assert.deepEqual(providerInput({ ...editing, keys: " sk-a \n\nsk-b ", modelList: "a\na, b" }), {
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/",
    protocol: "chatCompletions",
    keys: ["sk-a", "sk-b"],
    model: "deepseek-v4-pro",
    modelList: "a\nb",
    price: null,
    modelPrices: null,
    proxy: "",
    stripImageGenerationTool: false,
  });
  // #228：「移除托管生图工具」从池条目读回、原样提交。
  assert.equal(providerDraftFromEntry({ ...payload().pool[0], stripImageGenerationTool: true }).stripImageGenerationTool, true);
  assert.equal(providerInput({ ...editing, stripImageGenerationTool: true }).stripImageGenerationTool, true);
  assert.equal(validateProviderDraft({ ...editing, proxy: "127.0.0.1:7890" }, true), "代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）");
  assert.equal(validateProviderDraft({ ...editing, proxy: "socks5://127.0.0.1:1080" }, true), null);
  assert.deepEqual(providerInput({ ...editing, priceInput: "2", priceCached: "0.5", priceOutput: "abc" }).price, { currency: "CNY", input: 2, cachedInput: 0.5, output: 0 });
});

test("网关页接入导航，命令已注册", () => {
  assert.match(app, /id: "gateway"/);
  // 网关各页签由侧栏直接选中：路由 → 页签一一对应，页内不再显示标签条。
  assert.match(app, /gatewayTabByRoute\[route\] \? \(\s*<GatewayPage[\s\S]*?tab=\{gatewayTabByRoute\[route\]\}/);
  // 网关的「会话」已并进全局管理的「会话」页（sessions 路由），这里不再有单独的 gatewaySessions。
  for (const [route, tab] of [["gateway", "agents"], ["gatewayProviders", "providers"], ["gatewayRouting", "routing"], ["gatewayUsage", "usage"], ["gatewaySettings", "gateway"]]) {
    assert.match(app, new RegExp(`${route}: "${tab}"`));
  }
  for (const command of [
    "load_gateway",
    "save_gateway_binding",
    "disable_gateway_binding",
    "rotate_gateway_token",
    "save_gateway_options",
    "reset_gateway_key_health",
    "create_gateway_provider",
    "update_gateway_provider",
    "add_gateway_provider_key",
    "replace_gateway_provider_key",
    "delete_gateway_provider_key",
    "delete_gateway_provider",
    "set_gateway_provider_enabled",
    "reorder_gateway_providers",
    "import_gateway_providers",
    "save_gateway_group",
    "delete_gateway_group",
    "reveal_gateway_agent_config",
  ]) {
    assert.match(lib, new RegExp(`gateway_commands::${command},`));
  }
});

test("用量格式：token 缩写、货币、命中率、横轴标签", async () => {
  const { axisLabelIndexes, cacheHitRate, formatMoney, formatTokens, shortDate } = await import("./gateway-model.ts");
  assert.equal(formatTokens(940), "940");
  assert.equal(formatTokens(158_000), "158K");
  assert.equal(formatTokens(2_200_000), "2.2M");
  assert.equal(formatTokens(23_000_000), "23M");
  assert.equal(formatMoney(86.4, "CNY"), "¥86.40");
  assert.equal(formatMoney(23.75, "usd"), "$23.75");
  assert.equal(cacheHitRate({ calls: 1, errors: 0, inputTokens: 100, outputTokens: 0, cachedTokens: 88, reasoningTokens: 0 }), 0.88);
  assert.equal(cacheHitRate({ calls: 0, errors: 0, inputTokens: 0, outputTokens: 0, cachedTokens: 0, reasoningTokens: 0 }), null);
  assert.deepEqual(axisLabelIndexes(30, 7), [0, 5, 10, 15, 19, 24, 29]);
  assert.deepEqual(axisLabelIndexes(3, 7), [0, 1, 2]);
  assert.equal(shortDate("2026-09-01"), "9月1日");
  assert.equal(shortDate("2026-09-01", true), "Sep 1");
});

test("会话：合并排序、按 Agent 与关键词过滤；路径与相对时间", async () => {
  const { filterSessions, relativeTime, shortPath } = await import("./gateway-model.ts");
  const sessions = [
    { agent: "codex" as const, id: "c1", title: "修复登录", cwd: "/Users/me/app", updatedAtMs: 10 },
    { agent: "claude" as const, id: "k1", title: "重构网关", cwd: "/Users/me/gw", updatedAtMs: 30 },
    { agent: "grok" as const, id: "g1", title: "写测试", cwd: "/Users/me/app", updatedAtMs: 20 },
  ];
  assert.deepEqual(filterSessions(sessions, "all", "").map((session) => session.id), ["k1", "g1", "c1"]);
  assert.deepEqual(filterSessions(sessions, "grok", "").map((session) => session.id), ["g1"]);
  assert.deepEqual(filterSessions(sessions, "all", "app").map((session) => session.id), ["g1", "c1"]);
  assert.equal(shortPath("/Users/me/code/app"), "~/code/app");
  assert.equal(shortPath("/Users/me/a/b/c/d/e"), "…/d/e");
  const now = 1_000_000_000;
  assert.deepEqual(relativeTime(now - 5_000, now), { text: "刚刚", args: [] });
  assert.deepEqual(relativeTime(now - 5 * 60_000, now), { text: "{0} 分钟前", args: [5] });
  assert.deepEqual(relativeTime(now - 3 * 3_600_000, now), { text: "{0} 小时前", args: [3] });
  assert.deepEqual(relativeTime(now - 2 * 86_400_000, now), { text: "{0} 天前", args: [2] });
});

test("全应用不用原生下拉框，统一用 AppSelect", () => {
  const dir = new URL("./", import.meta.url);
  for (const file of readdirSync(dir)) {
    if (!file.endsWith(".tsx")) continue;
    const source = readFileSync(new URL(file, dir), "utf8");
    assert.doesNotMatch(source, /<select[\s>]/, file);
  }
  const shared = app.slice(app.indexOf("function SharedServiceSettings("), app.indexOf("function ProxySettingsScreen("));
  assert.match(shared, /<AppSelect/);
});

test("全应用不用系统原生的确认 / 提示 / 输入框，统一用 app-dialog", () => {
  const dir = new URL("./", import.meta.url);
  for (const file of readdirSync(dir)) {
    if (!/\.tsx?$/.test(file) || file.includes(".test.") || file === "app-dialog.tsx") continue;
    const source = readFileSync(new URL(file, dir), "utf8");
    assert.doesNotMatch(source, /window\.(confirm|alert|prompt)\(/, file);
    assert.doesNotMatch(source, /import \{[^}]*\b(confirm|ask|message)\b[^}]*\} from "@tauri-apps\/plugin-dialog"/, file);
  }
  assert.match(app, /<AppDialogHost \/>/);
});

test("供应商的价格与代理在供应商池详情里编辑，添加 / 编辑弹窗只管连接、Key 与模型", () => {
  const form = readFileSync(new URL("./ProviderFormDialog.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(form, /t\("价格"\)|t\("代理"\)|priceInput: event|proxy: event/);
  const providers = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  assert.match(providers, /"save_gateway_provider_extras"/);
  assert.match(providers, /<ProviderExtrasDialog/);
  const extras = readFileSync(new URL("./ProviderExtrasDialog.tsx", import.meta.url), "utf8");
  assert.match(extras, /t\("按公开价格填写"\)/);
  assert.match(extras, /placeholder="http:\/\/127\.0\.0\.1:7890"/);
});

test("赞助商单独一组放在「添加供应商」最前面，不再混在中转里", () => {
  const presets = gatewayPresets(new Date(2026, 9, 26));
  const sponsors = presets.filter((preset) => preset.section === "sponsor").map((preset) => preset.key);
  assert.deepEqual(sponsors, ["jojocode-max", "runapi", "apikeyfun"]);
  // 到期日当天仍显示，第二天起从目录里去掉。
  const keys = (date: Date) => gatewayPresets(date).map((preset) => preset.key);
  assert.ok(keys(new Date(2026, 10, 7)).includes("runapi"));
  assert.ok(!keys(new Date(2026, 10, 8)).includes("runapi"));
  assert.deepEqual(activeSponsorIds(new Date(2026, 10, 23)), ["jojocode-max"]);
  for (const gone of ["pateway", "therouter", "shengsuanyun", "ccsub"]) assert.ok(!keys(new Date(2026, 9, 26)).includes(gone), gone);
  assert.equal(presets[0].section, "sponsor");
  const relays = presets.filter((preset) => preset.section === "relay").map((preset) => preset.key);
  assert.ok(relays.includes("openrouter") && relays.includes("siliconflow") && relays.includes("opencode-go"));
  const catalog = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  assert.ok(catalog.indexOf('key: "sponsor"') < catalog.indexOf('key: "subscription"'));
});

test("起名模型随绑定草稿往返，空值和旧版后端缺字段都当成不指定", () => {
  const legacy = { ...blankBinding(), enabled: true, model: "deepseek-v4-pro" };
  assert.equal(draftFromBinding(legacy).titleModel, "");
  const draft = { ...draftFromBinding(legacy), titleModel: " kimi/kimi-k3 " };
  const binding = bindingFromDraft(draft, legacy);
  assert.equal(binding.titleModel, "kimi/kimi-k3");
  assert.equal(draftFromBinding(binding).titleModel, "kimi/kimi-k3");
  assert.equal(bindingFromDraft({ ...draft, titleModel: "  " }, binding).titleModel, "");
});

test("主动限速：留空不限，只认正整数", () => {
  assert.equal(parseKeyLimit(""), null);
  assert.equal(parseKeyLimit("  "), null);
  assert.equal(parseKeyLimit("20"), 20);
  assert.equal(parseKeyLimit(" 5 "), 5);
  for (const bad of ["0", "-1", "1.5", "abc", "10k"]) assert.ok(Number.isNaN(parseKeyLimit(bad)), bad);
});

test("请求日志认出网关记的限速排队", () => {
  assert.equal(queueWaitMs(undefined), null);
  assert.equal(queueWaitMs(["upstream_stream_failed:server_is_overloaded"]), null);
  assert.equal(queueWaitMs(["upstream_stream_failed:x", "local_rate_limit_wait_ms:3200"]), 3200);
  assert.equal(formatQueueWait(3200), "3.2s");
  assert.equal(formatQueueWait(400), "0.4s");
  assert.equal(formatQueueWait(45_400), "45s");
  assert.match(app, /tf\("限速排队 \{0\}", \[formatQueueWait\(queued\)\]\)/);
});

test("请求日志认出去掉别家签发的压缩上下文，并显示成说明文字", () => {
  assert.equal(FOREIGN_COMPACTION_DROPPED_REASON, "foreign_compaction_dropped");
  assert.equal(foreignCompactionDropped(undefined), false);
  assert.equal(foreignCompactionDropped(["invalid_encrypted_content"]), false);
  assert.equal(foreignCompactionDropped(["invalid_encrypted_content", "foreign_compaction_dropped"]), true);
  assert.match(app, /t\("已去掉别家签发的压缩上下文，较早的对话内容不可用"\)/);
  assert.match(app, /t\("已去掉别家压缩上下文"\)/);
});

test("供应商池详情能编辑每个 Key 的限速", () => {
  const providers = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  assert.match(providers, /"save_gateway_provider_limits", \{ id: entry\.id, rpm, concurrency \}/);
  assert.match(lib, /gateway_commands::save_gateway_provider_limits,/);
});

test("供应商池：编辑在标题栏、各处编辑是图标按钮；已保存的 Key 打码列出，按标识单独替换、删除（#2782）", () => {
  const providers = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  // 详情里不再有「编辑」文字按钮，底部也没有「编辑 >」；标题栏有铅笔按钮，悬停与无障碍名保留文案。
  assert.doesNotMatch(providers, /\{t\("编辑"\)\}/);
  assert.match(providers, /aria-label=\{tf\("编辑 \{0\}", \[maskAccountName\(entry\.name\)\]\)\}\s+className="gw-icon-button gw-pool-edit"/);
  assert.match(providers, /<button aria-label=\{label\} className="gw-icon-button gw-inline-icon"[^>]*title=\{label\}/);
  for (const label of ["编辑模型列表", "编辑价格", "编辑上下文窗口", "编辑思考等级", "编辑代理", "编辑限速"]) {
    assert.match(providers, new RegExp(`<EditIcon [^>]*label=\\{t\\("${label}"\\)\\}`), label);
  }
  assert.match(providers, /<ProviderKeyList busy=\{busy\} entry=\{entry\} onTest=\{\(keyId\) => setTestKey\(keyId\)\} run=\{run\} \/>/);
  const keys = readFileSync(new URL("./ProviderKeyList.tsx", import.meta.url), "utf8");
  assert.match(keys, /"add_gateway_provider_key", \{ id: entry\.id, key \}/);
  assert.match(keys, /"replace_gateway_provider_key", \{ id: entry\.id, keyId: editing\.keyId, key \}/);
  assert.match(keys, /"delete_gateway_provider_key", \{ id: entry\.id, keyId \}/);
  assert.match(keys, /type="password"/, "新填的 Key 不明文显示");
  assert.match(keys, /disabled=\{busy !== null \|\| last\}/, "只剩一个 Key 时不能删");
  // 编辑弹窗里已保存的 Key 只列打码后的样子，不再是「留空表示不改」的空输入框。
  const form = readFileSync(new URL("./ProviderFormDialog.tsx", import.meta.url), "utf8");
  assert.match(form, /savedKeys\.map\(\(key\) => <span className="pf-chip" key=\{key\.id\}><span className="pf-mono">\{key\.masked\}<\/span><\/span>\)/);
  // Key 池健康状态里的 Key 标识换成打码后的 Key；单 Key 时 Key 池记作 primary。
  const saved = [{ id: "key-1", masked: "sk-alp…AAAA" }, { id: "key-2", masked: "sk-bra…BBBB" }];
  assert.equal(keyLabel(saved, "key-2"), "sk-bra…BBBB");
  assert.equal(keyLabel(saved, "primary"), "primary");
  assert.equal(keyLabel([saved[0]], "primary"), "sk-alp…AAAA");
  assert.equal(keyLabel(undefined, "key-9"), "key-9");
});

test("供应商代理：留空和直连都合法，直连不分大小写（#2760）", () => {
  assert.equal(proxyAddressError(""), null);
  assert.equal(proxyAddressError(" direct "), null);
  assert.equal(proxyAddressError("DIRECT"), null);
  assert.equal(proxyAddressError("socks5://127.0.0.1:1080"), null);
  assert.match(proxyAddressError("127.0.0.1:7890") ?? "", /代理地址要写成/);
  assert.equal(isDirectProxy("Direct"), true);
  assert.equal(isDirectProxy(""), false);
  assert.equal(isDirectProxy("http://direct.example:8080"), false);
});
