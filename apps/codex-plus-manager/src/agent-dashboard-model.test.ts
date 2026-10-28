import assert from "node:assert/strict";
import test from "node:test";
import { agentAccessState, agentUsageStats, recentAgentRequests } from "./agent-dashboard-model.ts";
import type { UsageSummary } from "./gateway-model.ts";

const log = (id: string, agent: string | null, timestampMs: number, success = true) => ({
  id, agent, timestampMs, durationMs: 1800, success, model: "claude-sonnet", upstreamModel: "deepseek-v4-pro", relayName: "DeepSeek", state: null,
});

test("最近请求只取这个 Agent 的，新的在前，最多 limit 条", () => {
  const entries = [log("a", "claude", 1), log("b", "grok", 5), log("c", "claude", 3), log("d", null, 9), log("e", "claude", 2)];
  assert.deepEqual(recentAgentRequests(entries, "claude", 2).map((item) => item.id), ["c", "e"]);
  assert.equal(recentAgentRequests(entries, "claude")[0].provider, "DeepSeek");
});

test("今日用量：没有这个 Agent 的数据时是 0，成功率按失败数算", () => {
  const counts = { calls: 20, errors: 1, inputTokens: 1000, outputTokens: 500, cachedTokens: 0, reasoningTokens: 0 };
  const usage = { totals: counts, days: [], byAgent: [{ ...counts, key: "claude", label: "Claude Code" }], byProvider: [], byModel: [], costs: [] } as UsageSummary;
  assert.deepEqual(agentUsageStats(usage, "claude"), { calls: 20, successRate: 0.95, tokens: 1500 });
  assert.deepEqual(agentUsageStats(usage, "grok"), { calls: 0, successRate: null, tokens: 0 });
  assert.equal(agentUsageStats(null, "claude"), null);
});

test("顶部状态：经网关、被别的工具改掉、用自己的设置、未安装", () => {
  assert.equal(agentAccessState({ enabled: true }), "gateway");
  assert.equal(agentAccessState({ enabled: true, drift: { endpoint: "x" } }), "drift");
  assert.equal(agentAccessState({ enabled: false }), "direct");
  assert.equal(agentAccessState(undefined), "missing");
});

test("Codex 的最近请求包含没有 agent 字段的直连记录", () => {
  const entries = [log("a", null, 1), log("b", "codex", 2), log("c", "claude", 3)];
  assert.deepEqual(recentAgentRequests(entries, "codex").map((item) => item.id), ["b", "a"]);
});
