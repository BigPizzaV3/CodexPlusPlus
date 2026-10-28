/** 网关界面审查问题的回归测试：账号打码、加载中状态、冷却时间、权重输入、登录取消、过期响应等。 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  agentModels,
  maskAccountText,
  memberLabel,
  memberStatus,
  modelDispatch,
  modelDisplay,
  parseWeight,
  type GatewayPayload,
} from "./gateway-model.ts";
import { hubAgents, hubFeed, hubPoolState } from "./hub-overview-model.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

function payload(): GatewayPayload {
  return {
    gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
    port: 57321,
    helperRunning: true,
    pool: [
      { id: "chatgpt-1", name: "ChatGPT Pro · 2649340134@qq.com", baseUrl: "", protocol: "responses", official: true, models: ["gpt-5.5"], keyCount: 1, enabled: true, source: "login:chatgpt", account: "chatgpt" },
      { id: "copilot-1", name: "Copilot · BigPizzaV3", baseUrl: "", protocol: "chatCompletions", official: true, models: ["claude-sonnet-5", "GPT-5.5"], keyCount: 1, enabled: true, source: "login:copilot", account: "copilot" },
    ],
    groups: [],
    agents: [],
    health: { keys: [], providers: [] },
    codexProviders: [],
    claudeProviders: [],
    grokProfiles: [],
  } as GatewayPayload;
}

test("裸模型名的显示、模型组成员标签里的账号名打码", () => {
  const data = payload();
  // 两家都有的 gpt-5.5：裸名是「多家自动切换」，列出按顺序尝试的几家（都打码）；带前缀的固定一家。
  assert.deepEqual(modelDisplay(data, "gpt-5.5"), { title: "GPT-5.5", sub: "ChatGPT Pro · 26****34@qq.com → Copilot · Bi****V3", group: false, auto: true });
  assert.equal(memberLabel(data, "gpt-5.5"), "GPT-5.5 · ChatGPT Pro · 26****34@qq.com → Copilot · Bi****V3");
  assert.equal(memberLabel(data, "claude-sonnet-5"), "Copilot · Bi****V3 · Claude-Sonnet-5");
  assert.equal(memberLabel(data, "copilot-1/claude-sonnet-5"), "Copilot · Bi****V3 · Claude-Sonnet-5");
  // 调度表里的供应商名也打码；大小写不同的同名模型算一行。
  const rows = modelDispatch(data);
  assert.equal(rows[0].model, "gpt-5.5");
  assert.deepEqual(rows[0].providers.map((provider) => provider.name), ["ChatGPT Pro · 26****34@qq.com", "Copilot · Bi****V3"]);
  assert.deepEqual(agentModels(data), ["gpt-5.5", "claude-sonnet-5"]);
});

test("后端提示文字里的账号打码：「」里的名字和裸邮箱，已打码的不再打一次", () => {
  assert.equal(maskAccountText("已添加「ChatGPT Pro · 2649340134@qq.com」，3 个模型。"), "已添加「ChatGPT Pro · 26****34@qq.com」，3 个模型。");
  assert.equal(maskAccountText("已更新「Copilot · BigPizzaV3」的登录。"), "已更新「Copilot · Bi****V3」的登录。");
  assert.equal(maskAccountText("登录失败：me@example.com 没有订阅"), "登录失败：m***@example.com 没有订阅");
  assert.equal(maskAccountText("已添加「DeepSeek」。"), "已添加「DeepSeek」。");
});

test("界面上显示供应商名的地方都打码（含悬停提示、无障碍名）", () => {
  assert.match(read("gateway-ui.tsx"), /title=\{maskAccountName\(name\)\}/);
  assert.doesNotMatch(read("gateway-ui.tsx"), /title=\{name\}/);
  assert.match(read("ProviderExtrasDialog.tsx"), /tf\("\{0\} 的价格", \[maskAccountName\(entry\.name\)\]\)/);
  assert.match(read("ProviderFormDialog.tsx"), /tf\("编辑 \{0\}", \[maskAccountName\(form\.entry\.name\)\]\)/);
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /tf\("\{0\} 参与调度", \[maskAccountName\(entry\.name\)\]\)/);
  assert.match(providers, /<strong>\{maskAccountName\(item\.name\)\}<\/strong>/);
  assert.doesNotMatch(providers, /<strong>\{item\.name\}<\/strong>/);
  const usage = read("GatewayUsage.tsx");
  assert.match(usage, /<strong>\{maskAccountName\(balance\.provider\)\}<\/strong>/);
  assert.doesNotMatch(usage, /title=\{group\.label\}/);
  // 提示（toast）里的后端文案统一过 maskAccountText。
  assert.match(read("gateway-ui.tsx"), /notify\(title, maskAccountText\(result\.message\), "ok"\)/);
});

test("冷却 / 熔断没有剩余时间时不显示空的「 ·  后恢复」；剩余时间取有值的最小值", () => {
  const data = payload();
  data.health.keys = [{ relayId: "copilot-1", keyId: "k1", state: "cooling", remainingMs: null, lastFailure: "rate_limited" }];
  assert.deepEqual(memberStatus(data, "copilot-1"), { state: "cooling", text: "被限流", args: [] });
  data.health.keys = [
    { relayId: "copilot-1", keyId: "k1", state: "cooling", remainingMs: 0, lastFailure: "quota_exhausted" },
  ];
  assert.deepEqual(memberStatus(data, "copilot-1"), { state: "cooling", text: "额度耗尽", args: [] });
  data.pool[1].keyCount = 2;
  data.health.keys = [
    { relayId: "copilot-1", keyId: "k1", state: "cooling", remainingMs: null, lastFailure: "rate_limited" },
    { relayId: "copilot-1", keyId: "k2", state: "cooling", remainingMs: 30_000, lastFailure: "rate_limited" },
  ];
  assert.deepEqual(memberStatus(data, "copilot-1"), { state: "cooling", text: "被限流 · {0} 后恢复", args: ["30s"] });
  data.health.providers = [{ relayId: "chatgpt-1", breakerOpen: true, remainingMs: null, consecutiveErrors: 3 }];
  assert.deepEqual(memberStatus(data, "chatgpt-1"), { state: "cooling", text: "连续出错", args: [] });
});

test("总览：读取中、空池、全部关闭分开显示；最近请求没记请求模型时退回实际模型；直连供应商名打码", () => {
  assert.equal(hubPoolState(null), "loading");
  const data = payload();
  assert.equal(hubPoolState(data), "ready");
  data.pool.forEach((entry) => (entry.enabled = false));
  assert.equal(hubPoolState(data), "allOff");
  data.pool = [];
  assert.equal(hubPoolState(data), "empty");

  data.activity = {
    agents: [{
      agent: "claude", requests: 1, rerouted: 0, errors: 0, inFlight: [],
      last: { requestedModel: "", model: "gpt-5.5", planned: [], steps: [], result: "answered", finishedAtMs: 1, elapsedMs: 1 },
    }],
  };
  assert.equal(hubFeed(data)[0].requested, "gpt-5.5");
  assert.equal(hubAgents(null, { running: true, providerName: "ChatGPT · me@example.com" })[0].detail, "ChatGPT · m***@example.com");

  const hub = read("HubOverview.tsx");
  assert.match(hub, /poolState === "loading" \? <p className="hub-empty">\{t\("读取中…"\)\}<\/p>/);
  assert.match(hub, /poolState === "allOff"/);
  assert.match(hub, /: t\("正在读取网关状态…"\)/);
  const matrix = read("GatewayModelMatrix.tsx");
  assert.match(matrix, /\{!payload \? <div className="gw-empty">\{t\("正在读取…"\)\}<\/div> : null\}/);
  assert.match(matrix, /payload\.pool\.length \? t\("供应商都已关闭"\)/);
  assert.match(read("GatewayPage.tsx"), /\{!payload \? <div className="gw-card gw-empty">\{t\("正在读取…"\)\}<\/div> : null\}/);
});

test("路由说明的失败原因按标记翻译，不再按「是不是汉字开头」猜", () => {
  const hub = read("HubOverview.tsx");
  assert.doesNotMatch(hub, /一-龥/);
  assert.match(hub, /typeof arg === "object" \? t\(arg\.zh\) : arg/);
  assert.match(read("GatewayModelMatrix.tsx"), /\.join\(t\("、"\)\)/);
  assert.match(read("GatewayProviders.tsx"), /\$\{key\.keyId\}\$\{t\("："\)\}/);
});

test("权重输入：删空、输入到一半不改值，失焦后规范成 ≥ 1 的整数", () => {
  assert.equal(parseWeight(""), null);
  assert.equal(parseWeight("  "), null);
  assert.equal(parseWeight("abc"), null);
  assert.equal(parseWeight("15"), 15);
  assert.equal(parseWeight("0"), 1);
  assert.equal(parseWeight("2.7"), 2);
  const page = read("GatewayPage.tsx");
  assert.match(page, /value=\{weightText\[index\] \?\? String\(member\.weight\)\}/);
  assert.match(page, /onBlur=\{\(\) => \{\s*setWeightText/);
});

test("「从上游获取」的勾选列表里按回车不提交表单", () => {
  const form = read("ProviderFormDialog.tsx");
  const picker = form.slice(form.indexOf("function ModelPicker("));
  assert.match(picker, /if \(event\.key === "Enter"\) event\.preventDefault\(\);/);
});

test("用量页快速切换范围：只认最后一次读取的结果", () => {
  const usage = read("GatewayUsage.tsx");
  assert.match(usage, /const id = \+\+latest\.current;/);
  assert.match(usage, /if \(id !== latest\.current\) return;/);
});

test("网关页改了数据立刻推给侧栏和总览", () => {
  const page = read("GatewayPage.tsx");
  assert.match(page, /publishGatewayPayload\(result\);/);
  assert.match(read("HubOverview.tsx"), /export \{ useGatewayPayload \};/);
  assert.match(read("gateway-store.ts"), /export function publishGatewayPayload/);
});

test("轮询时不重复算：调度表 useMemo、筛选条 useMemo、组编辑器的空排除列表是常量", () => {
  const page = read("GatewayPage.tsx");
  assert.match(read("GatewayModelMatrix.tsx"), /const rows = useMemo\(\(\) => modelDispatch\(payload\), \[payload\]\);/);
  assert.match(page, /exclude=\{NO_EXCLUDE\}/);
  assert.doesNotMatch(page, /exclude=\{\[\]\}/);
  assert.match(read("GatewayAgents.tsx"), /const filters = useMemo\(/);
});

test("订阅登录不占全局 busy，可取消：关掉验证码弹窗即取消，后端能中止登录任务", () => {
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /"finish_gateway_copilot_login", \{ code, skipAgents: skip \}, title, false, \{ background: true \}\)/);
  assert.match(providers, /BROWSER_LOGIN_COMMANDS\[kind\], skip \? \{ skipAgents: skip \} : undefined, title, false, \{ background: true \}\)/);
  assert.match(providers, /invoke\("cancel_gateway_login"\)/);
  assert.match(providers, /<DeviceCodeDialog code=\{deviceCode\} notify=\{notify\} onCancel=\{cancel\} \/>/);
  assert.match(providers, /<LoginPendingBar /);
  const ui = read("gateway-ui.tsx");
  assert.match(ui, /if \(!options\.background\) setBusy\(key\);/);
  assert.match(ui, /if \(result\.status === "cancelled"\) return false;/);
  const lib = read("../src-tauri/src/lib.rs");
  assert.match(lib, /gateway_commands::cancel_gateway_login,/);
  const commands = read("../src-tauri/src/gateway_commands.rs");
  assert.match(commands, /pub fn cancel_gateway_login\(\)/);
  for (const name of ["login_gateway_chatgpt", "finish_gateway_copilot_login", "login_gateway_grok", "login_gateway_chatgpt_api"]) {
    const start = commands.indexOf(`pub async fn ${name}(`);
    const body = commands.slice(start, commands.indexOf("\n}\n", start));
    assert.match(body, /cancellable_login\(async move \{/, name);
    assert.match(body, /login_response\(outcome\)/, name);
  }
});

test("「挑选」弹层挂在 body 上，紧凑规则用 .gw-portal 前缀", () => {
  assert.match(read("GatewayPage.css"), /\.gw-portal \.gw-select-item \{/);
  assert.doesNotMatch(read("styles.css"), /\.shell \.gw-select-item/);
});

test("模型选择器顶部的供应商按钮是筛选，不是滚动到那一段", () => {
  const source = read("GatewayAgents.tsx");
  assert.match(source, /onClick=\{\(\) => pickScope\(\{ section: section\.key \}\)\}/);
  assert.match(source, /if \(sectionOnly\) return all\.filter\(\(section\) => section\.key === sectionOnly \|\| section\.key === "custom"\)/);
  assert.doesNotMatch(source, /scrollTop = node\.offsetTop/);
});

test("上下文窗口按模型在供应商池里填，Agent 页只留兜底的默认窗口", () => {
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /"save_gateway_provider_windows", \{ id: entry\.id, windows: modelWindows, fallback \}/);
  const dialog = read("ProviderWindowsDialog.tsx");
  assert.match(dialog, /next\[model\] = windowText\(match\.context\)/, "按公开资料填写用 models.dev 的 limit.context");
  assert.match(read("GatewayAgents.tsx"), /<span>\{t\("默认窗口"\)\}<\/span>/);
});

test("网关设置里选生图模型；供应商池按模型改思考等级，存的是和直连模式同格式的模型资料", () => {
  const page = read("GatewayPage.tsx");
  assert.match(page, /"save_gateway_image_model", \{ model: next \}/);
  assert.match(page, /\{ value: "off", label: t\("关闭"\)/);
  const dialog = read("ProviderReasoningDialog.tsx");
  assert.match(dialog, /setModelReasoningLevelsForSlug\(current, model, levels, defaultLevel\)/);
  assert.match(dialog, /invoke<ReasoningResult>\("gateway_provider_reasoning", \{ id: entry\.id \}\)/);
  assert.match(read("GatewayProviders.tsx"), /"save_gateway_provider_metadata", \{ id: entry\.id, modelMetadata \}/);
});
