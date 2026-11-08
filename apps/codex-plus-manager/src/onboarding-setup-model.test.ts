import assert from "node:assert/strict";
import test from "node:test";
import { blankBinding, type GatewayPayload } from "./gateway-model.ts";
import { hasCodexEnhancements, probeMatches, serializeSetupState, setupGatewayState, shouldShowSetup } from "./onboarding-setup-model.ts";

test("仅启用拓展不能绕过增强总开关，独立额度解锁可单独使用", () => {
  const config = {
    settings: { enhancementsEnabled: false, codexAppStandaloneQuotaUnlock: false },
    user_scripts: { enabled: true, skip_next_start: false, scripts: [{ enabled: true, status: "not_loaded", error: "" }] },
  };
  assert.equal(hasCodexEnhancements(null), false);
  assert.equal(hasCodexEnhancements(config), false);
  config.user_scripts.enabled = false;
  assert.equal(hasCodexEnhancements(config), false);
  config.settings.codexAppStandaloneQuotaUnlock = true;
  assert.equal(hasCodexEnhancements(config), true);
  config.settings.codexAppStandaloneQuotaUnlock = false;
  config.settings.enhancementsEnabled = true;
  assert.equal(hasCodexEnhancements(config), true);
});

function fixture(): GatewayPayload {
  return {
    gateway: {
      version: 1,
      bindings: {
        codex: { ...blankBinding(), enabled: true, model: "shared", token: "do-not-include-binding-token" },
        claude: { ...blankBinding(), enabled: true, model: "backup" },
      },
      keyPolicy: { mode: "sticky" },
      restoreOnExit: false,
    },
    port: 57321,
    helperRunning: true,
    pool: [
      { id: "first", name: "First", baseUrl: "https://first.test/v1", protocol: "responses", official: false, models: ["shared", "image-model"], imageModels: ["image-model"], keyCount: 0, account: "subscription" },
      { id: "second", name: "Second", baseUrl: "https://second.test/v1", protocol: "chatCompletions", official: false, models: ["shared", "backup"], keyCount: 1, keys: [{ id: "key-1", masked: "sk-...abcd" }] },
    ],
    groups: [],
    agents: [
      { agent: "codex", name: "Codex", enabled: true, baseUrl: "http://127.0.0.1:57321", defaultModel: "shared", models: ["shared"] },
      { agent: "claude", name: "Claude Code", enabled: true, baseUrl: "http://127.0.0.1:57321", defaultModel: "backup", models: ["backup"] },
    ],
    health: { keys: [], providers: [] },
    codexProviders: [],
    claudeProviders: [],
    grokProfiles: [],
  };
}

test("入门清单只自动面向新用户，完成与跳过都独立记忆，旧导览用户不强制重看", () => {
  assert.equal(shouldShowSetup(null, null), true);
  assert.equal(shouldShowSetup("broken", null), true);
  assert.equal(shouldShowSetup("null", null), true);
  assert.equal(shouldShowSetup('{"version":1,"state":"inProgress"}', null), true);
  for (const state of ["completed", "dismissed"] as const) {
    const stored = serializeSetupState(state);
    assert.deepEqual(JSON.parse(stored), { version: 1, state });
    assert.equal(shouldShowSetup(stored, null), false);
  }
  assert.equal(shouldShowSetup(null, "1"), false);
  assert.equal(shouldShowSetup(null, "0"), false);
  assert.equal(shouldShowSetup("broken", "garbage"), false);
});

test("状态区分未读到配置、Agent 不存在、未接入、漂移、网关停止和缺少模型", () => {
  assert.equal(setupGatewayState("all", null, "claude").issue, "loading");
  assert.equal(setupGatewayState("all", fixture(), "absent").issue, "agent_missing");
  const data = fixture();
  data.agents[0].enabled = false;
  assert.equal(setupGatewayState("codex", data, "codex").issue, "not_connected");
  data.agents[0].enabled = true;
  data.agents[0].drift = { endpoint: "https://other.test" };
  assert.equal(setupGatewayState("codex", data, "codex").issue, "drift");
  data.agents[0].drift = null;
  data.helperRunning = false;
  assert.equal(setupGatewayState("codex", data, "codex").issue, "helper_stopped");
  data.helperRunning = true;
  data.agents[0].defaultModel = "";
  assert.equal(setupGatewayState("codex", data, "codex").issue, "no_model");
  data.agents[0].defaultModel = "missing";
  assert.equal(setupGatewayState("codex", data, "codex").issue, "unroutable");
});

test("只管理 Codex 固定检查 Codex，多 Agent 模式检查用户选择，不偷换目标", () => {
  const data = fixture();
  assert.equal(setupGatewayState("codex", data, "claude").agent?.agent, "codex");
  assert.equal(setupGatewayState("all", data, "claude").agent?.agent, "claude");
  assert.deepEqual(setupGatewayState("all", data, "claude").candidates.map((item) => item.model), ["backup"]);
});

test("无 Key 订阅也能配置，禁用和排除的供应商不能成为测试候选，生图模型不算", () => {
  const data = fixture();
  data.gateway.bindings.codex!.poolExclude = ["second"];
  const ready = setupGatewayState("codex", data, "codex");
  assert.equal(ready.providersConfigured, true);
  assert.equal(ready.bindingReady, true);
  assert.deepEqual(ready.candidates.map((item) => [item.providerId, item.model]), [["first", "shared"]]);
  data.pool[0].enabled = false;
  assert.equal(setupGatewayState("codex", data, "codex").issue, "no_providers");
  data.pool[0].enabled = true;
  data.pool[0].models = ["image-model"];
  data.agents[0].defaultModel = "image-model";
  assert.equal(setupGatewayState("codex", data, "codex").issue, "no_providers");
});

test("具体供应商支持自定义模型；带斜杠的已声明完整模型名不会被误拆", () => {
  const data = fixture();
  data.agents[0].defaultModel = "first/custom-new";
  assert.deepEqual(setupGatewayState("codex", data, "codex").candidates.map((item) => [item.providerId, item.model]), [["first", "custom-new"]]);
  data.pool[0].models = [];
  data.gateway.bindings.codex!.poolExclude = ["second"];
  assert.equal(setupGatewayState("codex", data, "codex").bindingReady, true, "显式自定义模型不要求先出现在供应商列表里");
  data.gateway.bindings.codex!.poolExclude = [];
  data.pool[1].models.push("first/custom-new");
  assert.deepEqual(setupGatewayState("codex", data, "codex").candidates.map((item) => [item.providerId, item.model]), [["second", "first/custom-new"]]);
});

test("模型组展开具体候选并去重，失效成员不使整组假就绪，环和过深嵌套可退出", () => {
  const data = fixture();
  data.groups = [
    { id: "daily", name: "Daily", mode: "inOrder", members: [{ model: "group/nested", weight: 1 }, { model: "first/shared", weight: 1 }] },
    { id: "nested", name: "Nested", mode: "rotate", members: [{ model: "shared", weight: 1 }, { model: "group/daily", weight: 1 }] },
  ];
  data.agents[0].defaultModel = "group/daily";
  const state = setupGatewayState("codex", data, "codex");
  assert.equal(state.bindingReady, true);
  assert.equal(state.multipleTargets, true);
  assert.deepEqual(state.candidates.map((item) => item.providerId), ["first", "second"]);
  data.groups[0].members = [{ model: "group/nested", weight: 1 }];
  data.groups[1].members = [{ model: "group/daily", weight: 1 }];
  assert.equal(setupGatewayState("codex", data, "codex").issue, "unroutable");
  data.groups[1].members = [{ model: "first/missing-in-list", weight: 1 }];
  data.gateway.bindings.codex!.poolExclude = ["first"];
  assert.equal(setupGatewayState("codex", data, "codex").issue, "unroutable");
  data.gateway.bindings.codex!.poolExclude = [];
  data.groups = Array.from({ length: 5 }, (_, index) => ({
    id: `depth-${index}`, name: `Depth ${index}`, mode: "inOrder" as const,
    members: [{ model: index === 4 ? "shared" : `group/depth-${index + 1}`, weight: 1 }],
  }));
  data.agents[0].defaultModel = "group/depth-0";
  assert.equal(setupGatewayState("codex", data, "codex").issue, "unroutable");
  data.agents[0].defaultModel = "group/depth-1";
  assert.equal(setupGatewayState("codex", data, "codex").bindingReady, true);
});

test("入门检查跳过停用的嵌套组，切换状态会使先前验证失效", () => {
  const data = fixture();
  data.groups = [
    { id: "outer", name: "Outer", mode: "inOrder", members: [{ model: "GROUP/INNER", weight: 1 }, { model: "second/backup", weight: 1 }] },
    { id: "inner", name: "Inner", mode: "inOrder", members: [{ model: "first/shared", weight: 1 }] },
  ];
  data.agents[0].defaultModel = "GROUP/OUTER";
  const before = setupGatewayState("codex", data, "codex");
  assert.deepEqual(before.candidates.map((item) => item.providerId), ["first", "second"]);
  const probe = { fingerprint: before.fingerprint, candidateId: before.candidates[0].id, ok: true };
  data.groups[1].disabled = true;
  const disabled = setupGatewayState("codex", data, "codex");
  assert.deepEqual(disabled.candidates.map((item) => item.providerId), ["second"]);
  assert.equal(probeMatches(disabled, probe), false);
  data.groups[1].disabled = false;
  assert.deepEqual(setupGatewayState("codex", data, "codex").candidates.map((item) => item.providerId), ["first", "second"]);
});

test("手动组和自动组只检查实际选中成员，通配符沿用可用池的匹配规则", () => {
  const data = fixture();
  data.groups = [{ id: "manual", name: "Manual", mode: "manual", pick: "second/backup", members: [{ model: "first/*", weight: 1 }, { model: "second/backup", weight: 1 }] }];
  data.agents[0].defaultModel = "group/manual";
  assert.deepEqual(setupGatewayState("codex", data, "codex").candidates.map((item) => [item.providerId, item.model]), [["second", "backup"]]);
  data.groups[0].pick = "first/*";
  assert.deepEqual(setupGatewayState("codex", data, "codex").candidates.map((item) => [item.providerId, item.model]), [["first", "shared"]]);
  data.agents[0].defaultModel = "shared";
  data.autoGroups = [{ id: "auto:shared", name: "shared", mode: "manual", pick: "second/shared", members: [{ model: "first/shared", weight: 1 }, { model: "second/shared", weight: 1 }], hidden: false, customized: true }];
  assert.deepEqual(setupGatewayState("codex", data, "codex").candidates.map((item) => item.providerId), ["second"]);
  data.gateway.autoModels = false;
  assert.deepEqual(setupGatewayState("codex", data, "codex").candidates.map((item) => item.providerId), ["first", "second"]);
});

test("测试只有明确成功、配置相同且候选仍存在时才能完成验证", () => {
  const data = fixture();
  const state = setupGatewayState("codex", data, "codex");
  const probe = { fingerprint: state.fingerprint, candidateId: state.candidates[0].id, ok: true };
  assert.equal(probeMatches(state, probe), true);
  assert.equal(state.multipleTargets, true, "一次成功并不消除还有其它候选这一事实");
  assert.equal(probeMatches(state, null), false);
  assert.equal(probeMatches(state, { ...probe, ok: false }), false);
  assert.equal(probeMatches(state, { ...probe, candidateId: "unknown" }), false);
  data.agents[0].defaultModel = "backup";
  assert.equal(probeMatches(setupGatewayState("codex", data, "codex"), probe), false);
  data.agents[0].defaultModel = "shared";
  data.helperRunning = false;
  assert.equal(probeMatches(setupGatewayState("codex", data, "codex"), probe), false);
});

test("配置标记忽略统计与价格且不包含明文凭据，绑定、模型、地址、代理、Key 标识或组变化会失效", () => {
  const data = fixture();
  data.pool[0].baseUrl = "https://user:url-password@first.test/v1?token=query-secret";
  data.pool[0].proxy = "http://proxy-user:proxy-password@proxy.test";
  const original = setupGatewayState("codex", data, "codex").fingerprint;
  for (const secret of ["do-not-include-binding-token", "url-password", "query-secret", "proxy-password"]) assert.ok(!original.includes(secret));
  data.health.providers = [{ relayId: "first", breakerOpen: true, consecutiveErrors: 1 }];
  data.speeds = [{ relayId: "first", model: "shared", ms: 900, samples: 4 }];
  data.pool[0].price = { currency: "USD", input: 1, output: 2, cachedInput: 0 };
  data.pool[0].quota = { updatedAtMs: 1, windows: [{ windowMinutes: 300, resetsAtMs: 2, usedPercent: 42 }] };
  assert.equal(setupGatewayState("codex", data, "codex").fingerprint, original);
  const changes: Array<(next: GatewayPayload) => void> = [
    (next) => { next.pool[0].baseUrl += "/new"; },
    (next) => { next.pool[0].protocol = "anthropicMessages"; },
    (next) => { next.pool[0].proxy = "http://changed.test"; },
    (next) => { next.pool[0].models.push("new-model"); },
    (next) => { next.pool[1].keys![0].id = "key-2"; },
    (next) => { next.gateway.bindings.codex!.poolExclude = ["first"]; },
    (next) => { next.gateway.bindings.codex!.model = "backup"; },
    (next) => { next.groups.push({ id: "new", name: "New", mode: "inOrder", members: [{ model: "shared", weight: 1 }] }); },
  ];
  for (const change of changes) {
    const next = structuredClone(data);
    change(next);
    assert.notEqual(setupGatewayState("codex", next, "codex").fingerprint, original);
  }
});
