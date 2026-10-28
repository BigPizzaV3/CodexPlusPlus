/** Claude Code / Grok 概览仪表板的数据整理：页面只负责画。 */
import { maskAccountName, type UsageCounts, type UsageSummary } from "./gateway-model.ts";

export type DashboardRequest = {
  id: string;
  model: string;
  provider: string;
  durationMs: number;
  success: boolean;
  running: boolean;
  timestampMs: number;
};

type RequestLogLike = {
  id: string;
  agent?: string | null;
  timestampMs: number;
  durationMs: number;
  success: boolean;
  model: string;
  upstreamModel: string;
  relayName: string;
  state?: string | null;
};

/** 请求日志里这个 Agent 最近的几条，新的在前。 */
export function recentAgentRequests(entries: RequestLogLike[], agent: string, limit = 6): DashboardRequest[] {
  return entries
    // Codex 直连代理的记录没有 agent 字段，算作 Codex。
    .filter((entry) => (entry.agent || "codex") === agent)
    .sort((left, right) => right.timestampMs - left.timestampMs)
    .slice(0, limit)
    .map((entry) => ({
      id: entry.id,
      model: entry.model || entry.upstreamModel,
      provider: maskAccountName(entry.relayName),
      durationMs: entry.durationMs,
      success: entry.success,
      running: entry.state === "running",
      timestampMs: entry.timestampMs,
    }));
}

export type DashboardStats = { calls: number; successRate: number | null; tokens: number };

/** 今日用量里这个 Agent 的那一组；没有就是 0。 */
export function agentUsageStats(usage: UsageSummary | null, agent: string): DashboardStats | null {
  if (!usage) return null;
  const group: UsageCounts | undefined = usage.byAgent.find((item) => item.key === agent);
  if (!group) return { calls: 0, successRate: null, tokens: 0 };
  return {
    calls: group.calls,
    successRate: group.calls ? (group.calls - group.errors) / group.calls : null,
    tokens: group.inputTokens + group.outputTokens,
  };
}

export type AccessState = "gateway" | "drift" | "direct" | "missing";

/** 顶部状态：经网关 / 配置被别的工具改掉 / 用自己的设置 / 没装。 */
export function agentAccessState(view: { enabled: boolean; version?: string | null; drift?: unknown } | undefined): AccessState {
  if (!view) return "missing";
  if (view.enabled && view.drift) return "drift";
  if (view.enabled) return "gateway";
  return "direct";
}
