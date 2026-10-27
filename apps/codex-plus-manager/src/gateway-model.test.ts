import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import {
  agentModels,
  bindingFromDraft,
  blankBinding,
  connectedAgents,
  draftFromBinding,
  formatRemaining,
  activeSponsorIds,
  gatewayPresets,
  groupModelId,
  memberLabel,
  memberStatus,
  modelDisplay,
  modelRoutable,
  parseContextWindow,
  pickerSections,
  presetAdded,
  presetHost,
  providerDraftFromEntry,
  providerDraftFromPreset,
  providerInput,
  routeMembers,
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
  assert.deepEqual(modelDisplay(payload(), ""), { title: "deepseek-v4-pro", sub: "DeepSeek", group: false });
  assert.deepEqual(modelDisplay(payload(), "glm-5.3"), { title: "glm-5.3", sub: "GLM", group: false });
  assert.deepEqual(modelDisplay(payload(), "deepseek/deepseek-v4-flash"), { title: "deepseek-v4-flash", sub: "DeepSeek", group: false });
  assert.deepEqual(modelDisplay(payload(), "group/daily"), { title: "Daily", sub: "group/daily", group: true });
  assert.equal(memberLabel(payload(), "deepseek/deepseek-v4-flash"), "DeepSeek · deepseek-v4-flash");
  assert.equal(memberLabel(payload(), "group/daily"), "Daily");
  assert.equal(groupModelId("daily"), "group/daily");
  assert.equal(slugify(" 系统 默认 "), "系统-默认");
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

test("成员状态：正在回答 > 熔断 / Key 全部冷却 > 最近回答了谁 > Key 数", () => {
  const data = withActivity();
  assert.deepEqual(memberStatus(data, "glm"), { state: "answered", text: "回答了 {0}", args: ["Claude Code"] });
  assert.deepEqual(memberStatus(data, "deepseek"), { state: "idle", text: "{0} 个 Key", args: [2] });
  data.health.keys = [{ relayId: "deepseek", keyId: "key-1", state: "cooling", remainingMs: 35_000, lastFailure: "rate_limited" }];
  assert.deepEqual(memberStatus(data, "deepseek"), { state: "idle", text: "{0} 个 Key · {1} 个冷却中", args: [2, 1] });
  data.health.keys.push({ relayId: "deepseek", keyId: "key-2", state: "cooling", remainingMs: 20_000, lastFailure: "rate_limited" });
  assert.deepEqual(memberStatus(data, "deepseek"), { state: "cooling", text: "被限流 · {0} 后恢复", args: ["20s"] });
  data.health.providers = [{ relayId: "glm", breakerOpen: true, remainingMs: 60_000, consecutiveErrors: 3 }];
  assert.equal(memberStatus(data, "glm").state, "cooling");
  data.activity!.agents[0].inFlight = ["glm"];
  assert.deepEqual(memberStatus(data, "glm"), { state: "busy", text: "正在回答…", args: [] });
  assert.deepEqual(memberStatus(payload(), "bare"), { state: "idle", text: "一个 Key", args: [] });
});

test("路由页：所有接入的 Agent 共用一张卡片；成员是池里打开的供应商", () => {
  const data = withActivity();
  assert.deepEqual(connectedAgents(data), ["claude"]);
  const records = data.activity!.agents.map((agent) => agent.last!);
  assert.deepEqual(routeMembers(data, records), [
    { relayId: "deepseek", name: "DeepSeek", model: "deepseek-v4-flash" },
    { relayId: "glm", name: "GLM", model: "glm-5.3" },
    { relayId: "bare", name: "Bare", model: "" },
  ]);
});

test("上一个请求的路由说明：先列计划，再逐个尝试，最后谁回答", () => {
  const data = withActivity();
  const lines = routeNarrative("claude", data.activity!.agents[0].last!, data);
  assert.deepEqual(lines.map((line) => line.tone), ["muted", "step", "step", "ok"]);
  assert.deepEqual(lines[0].args, ["Claude Code", "claude-sonnet-5", "Daily", "DeepSeek/deepseek-v4-flash, GLM/glm-5.3"]);
  assert.deepEqual(lines[2].args, ["DeepSeek", "key-1", "被限流", " · HTTP 429"]);
  assert.deepEqual(lines[3].args, ["GLM", "glm-5.3", "1.2 s"]);
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
    proxy: "",
  });
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
    "delete_gateway_provider",
    "set_gateway_provider_enabled",
    "reorder_gateway_providers",
    "import_gateway_provider",
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
