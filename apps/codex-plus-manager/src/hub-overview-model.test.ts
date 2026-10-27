import assert from "node:assert/strict";
import test from "node:test";
import type { GatewayPayload } from "./gateway-model.ts";
import { hubAgents, hubFeed, hubLinks, hubProviders, hubTotals, successRate } from "./hub-overview-model.ts";

function payload(): GatewayPayload {
  const binding = (model: string) => ({
    enabled: true,
    model,
    extraModels: [],
    claudeRoles: { opus: "", sonnet: "", haiku: "", subagent: "" },
    unknownModel: "mapToDefault" as const,
    claudeEnableEffort: false,
    token: "t",
  });
  return {
    gateway: { version: 1, bindings: { claude: binding("deepseek-v4-pro") }, keyPolicy: { mode: "sticky" }, restoreOnExit: true },
    port: 63130,
    helperRunning: true,
    pool: [
      { id: "ds", name: "DeepSeek", baseUrl: "https://api.deepseek.com", protocol: "chatCompletions", official: true, models: ["deepseek-v4-pro"], keyCount: 2 },
      { id: "kimi", name: "Kimi", baseUrl: "https://api.moonshot.cn/v1", protocol: "chatCompletions", official: true, models: ["kimi-k2"], keyCount: 1 },
      { id: "off", name: "Off", baseUrl: "https://example.com", protocol: "responses", official: false, models: ["x"], keyCount: 1, enabled: false },
    ],
    groups: [],
    agents: [
      { agent: "claude", name: "Claude Code", enabled: true, baseUrl: "", defaultModel: "deepseek-v4-pro", models: [] },
      { agent: "grok", name: "Grok", enabled: false, baseUrl: "", defaultModel: "", models: [] },
    ],
    health: {
      keys: [{ relayId: "kimi", keyId: "k1", state: "cooling" }],
      providers: [{ relayId: "kimi", breakerOpen: true, consecutiveErrors: 3 }],
    },
    activity: {
      agents: [
        {
          agent: "claude",
          requests: 12,
          rerouted: 2,
          errors: 1,
          inFlight: [],
          last: {
            requestedModel: "claude-sonnet",
            model: "deepseek-v4-pro",
            planned: [{ relayId: "kimi", relayName: "Kimi", model: "kimi-k2" }],
            steps: [
              { relayId: "kimi", relayName: "Kimi", keyId: "k1", model: "kimi-k2", outcome: "failed", elapsedMs: 100 },
              { relayId: "ds", relayName: "DeepSeek", keyId: "k2", model: "deepseek-v4-pro", outcome: "answered", elapsedMs: 900 },
            ],
            result: "answered",
            finishedAtMs: 2000,
            elapsedMs: 1000,
          },
        },
      ],
    },
    codexProviders: [],
    claudeProviders: [],
    grokProfiles: [],
  } as GatewayPayload;
}

test("Codex 走直连排在第一位，其余 Agent 按网关绑定显示", () => {
  const agents = hubAgents(payload(), { running: true, providerName: "DeepSeek 官方" });
  assert.deepEqual(agents.map((agent) => agent.id), ["codex", "claude", "grok"]);
  assert.equal(agents[0].direct, true);
  assert.equal(agents[0].detail, "DeepSeek 官方");
  assert.equal(agents[1].enabled, true);
  assert.equal(agents[1].requests, 12);
  assert.equal(agents[1].live, true);
  assert.equal(agents[2].enabled, false);
  assert.equal(agents[2].live, false);
});

test("Codex 接入网关后只画网关节点；在网关列表里但没打开时仍画成直连，不重复", () => {
  const base = payload();
  const codexRow = { agent: "codex", name: "Codex", enabled: true } as (typeof base.agents)[number];
  const connected = { ...base, agents: [codexRow, ...base.agents] } as GatewayPayload;
  const viaGateway = hubAgents(connected, { running: true, providerName: "DeepSeek 官方" });
  assert.deepEqual(viaGateway.map((agent) => agent.id), ["codex", "claude", "grok"]);
  assert.equal(viaGateway[0].direct, false);
  assert.equal(viaGateway[0].enabled, true);

  const off = { ...base, agents: [{ ...codexRow, enabled: false }, ...base.agents] } as GatewayPayload;
  const direct = hubAgents(off, { running: true, providerName: "DeepSeek 官方" });
  assert.deepEqual(direct.map((agent) => agent.id), ["codex", "claude", "grok"]);
  assert.equal(direct[0].direct, true);
  assert.equal(direct[0].detail, "DeepSeek 官方");
});

test("供应商池只列打开的供应商，带上今日调用量", () => {
  const providers = hubProviders(payload(), new Map([["ds", 40]]));
  assert.deepEqual(providers.map((provider) => provider.id), ["ds", "kimi"]);
  assert.equal(providers[0].calls, 40);
  assert.equal(providers[1].calls, 0);
  assert.equal(providers[1].status.state, "cooling");
  assert.equal(hubProviders(payload(), null)[0].calls, null);
});

test("连线：直连、未接入、熔断各有自己的样式", () => {
  const data = payload();
  const links = hubLinks(hubAgents(data, { running: false, providerName: null }), hubProviders(data, new Map([["ds", 40], ["kimi", 10]])));
  const tone = (from: string, to: string) => links.find((link) => link.from === from && link.to === to)?.tone;
  assert.equal(tone("agent:codex", "hub"), "direct");
  assert.equal(tone("agent:claude", "hub"), "live");
  assert.equal(tone("agent:grok", "hub"), "off");
  assert.equal(tone("hub", "provider:kimi"), "warn");
  assert.equal(links.find((link) => link.to === "provider:ds")?.weight, 1);
});

test("路由动态：换过上游的成功请求标成「换上游」，显示最终回答的供应商", () => {
  const [item] = hubFeed(payload());
  assert.equal(item.result, "rerouted");
  assert.equal(item.provider, "DeepSeek");
  assert.equal(item.model, "deepseek-v4-pro");
  assert.equal(item.requested, "claude-sonnet");
});

test("hub 统计与成功率", () => {
  assert.deepEqual(hubTotals(payload()), { requests: 12, rerouted: 2, coolingKeys: 1, openBreakers: 1 });
  assert.deepEqual(hubTotals(null), { requests: 0, rerouted: 0, coolingKeys: 0, openBreakers: 0 });
  assert.equal(successRate({ calls: 10, errors: 1, inputTokens: 0, outputTokens: 0, cachedTokens: 0, reasoningTokens: 0 }), 0.9);
  assert.equal(successRate({ calls: 0, errors: 0, inputTokens: 0, outputTokens: 0, cachedTokens: 0, reasoningTokens: 0 }), null);
});
