/** 网关总览（Agent → Codex++ → 供应商池）的纯数据整理，页面只负责画。 */
import {
  maskAccountName,
  memberStatus,
  modelDisplay,
  providerEnabled,
  type GatewayPayload,
  type MemberStatus,
  type RouteRecordView,
  type UsageCounts,
} from "./gateway-model.ts";

/** 拓扑图左列的一个 Agent。Codex 没接入网关时走直连模式，不经网关的供应商池。 */
export type HubAgentNode = {
  id: string;
  name: string;
  /** 接入了网关（直连的 Codex 为 false，用 `direct` 表示）。 */
  enabled: boolean;
  direct: boolean;
  /** 副标题：网关模型，或直连的供应商名。 */
  detail: string;
  requests: number;
  live: boolean;
};

export type HubProviderNode = {
  id: string;
  name: string;
  baseUrl: string;
  keyCount: number;
  status: MemberStatus;
  /** 今日调用次数，读不到用量时为 null。 */
  calls: number | null;
};

export type HubLink = { from: string; to: string; tone: "live" | "idle" | "direct" | "warn" | "off"; weight: number };

export type HubFeedItem = {
  agent: string;
  requested: string;
  provider: string;
  model: string;
  elapsedMs: number;
  result: "answered" | "rerouted" | "failed" | "cooling" | "busy";
  finishedAtMs: number;
};

export type CodexSummary = { running: boolean; providerName: string | null };

export function hubAgents(payload: GatewayPayload | null, codex: CodexSummary): HubAgentNode[] {
  const activity = payload?.activity?.agents ?? [];
  // Codex 只出现一次：接入了网关就按网关节点画，否则排第一个、画成直连。
  const codexViaGateway = (payload?.agents ?? []).some((agent) => agent.agent === "codex" && agent.enabled);
  const nodes: HubAgentNode[] = codexViaGateway
    ? []
    : [{ id: "codex", name: "Codex", enabled: false, direct: true, detail: codex.providerName ?? "", requests: 0, live: false }];
  for (const agent of payload?.agents ?? []) {
    if (agent.agent === "codex" && !codexViaGateway) continue;
    const binding = payload?.gateway.bindings[agent.agent];
    const stats = activity.find((item) => item.agent === agent.agent);
    const model = binding?.enabled ? modelDisplay(payload, binding.model, binding.poolExclude ?? []).title : "";
    nodes.push({
      id: agent.agent,
      name: agent.name,
      enabled: agent.enabled,
      direct: false,
      detail: model,
      requests: stats?.requests ?? 0,
      live: agent.enabled && Boolean(stats && (stats.inFlight.length > 0 || stats.requests > 0)),
    });
  }
  return nodes;
}

export function hubProviders(payload: GatewayPayload | null, callsByProvider: Map<string, number> | null): HubProviderNode[] {
  return (payload?.pool ?? []).filter(providerEnabled).map((entry) => ({
    id: entry.id,
    name: maskAccountName(entry.name),
    baseUrl: entry.baseUrl,
    keyCount: entry.keyCount,
    status: memberStatus(payload, entry.id),
    calls: callsByProvider ? callsByProvider.get(entry.id) ?? 0 : null,
  }));
}

/** 连线：Agent → hub、hub → 供应商。粗细按请求量 / 今日调用量归一。 */
export function hubLinks(agents: HubAgentNode[], providers: HubProviderNode[]): HubLink[] {
  const maxRequests = Math.max(1, ...agents.map((agent) => agent.requests));
  const maxCalls = Math.max(1, ...providers.map((provider) => provider.calls ?? 0));
  const links: HubLink[] = agents.map((agent) => ({
    from: `agent:${agent.id}`,
    to: "hub",
    tone: agent.direct ? "direct" : !agent.enabled ? "off" : agent.live ? "live" : "idle",
    weight: agent.requests / maxRequests,
  }));
  for (const provider of providers) {
    const state = provider.status.state;
    links.push({
      from: "hub",
      to: `provider:${provider.id}`,
      tone: state === "busy" || state === "answered" ? "live" : state === "cooling" ? "warn" : "idle",
      weight: (provider.calls ?? 0) / maxCalls,
    });
  }
  return links;
}

/** 路由动态：每个 Agent 最近一次请求 + 正在进行的，按时间倒序。 */
export function hubFeed(payload: GatewayPayload | null): HubFeedItem[] {
  const items: HubFeedItem[] = [];
  for (const agent of payload?.activity?.agents ?? []) {
    if (agent.last) items.push(feedItem(agent.agent, agent.last));
  }
  return items.sort((left, right) => right.finishedAtMs - left.finishedAtMs);
}

function feedItem(agent: string, record: RouteRecordView): HubFeedItem {
  const answered = [...record.steps].reverse().find((step) => step.outcome === "answered");
  const target = answered ?? record.steps[record.steps.length - 1] ?? record.planned[0];
  const rerouted = record.result === "answered" && record.steps.length > 1;
  return {
    agent,
    requested: record.requestedModel,
    provider: maskAccountName(target?.relayName ?? ""),
    model: target?.model ?? record.model,
    elapsedMs: record.elapsedMs,
    result: rerouted ? "rerouted" : record.result,
    finishedAtMs: record.finishedAtMs,
  };
}

/** hub 卡片上的四个数：请求、换上游、冷却中的 Key、熔断中的供应商。 */
export function hubTotals(payload: GatewayPayload | null) {
  const activity = payload?.activity?.agents ?? [];
  return {
    requests: activity.reduce((sum, item) => sum + item.requests, 0),
    rerouted: activity.reduce((sum, item) => sum + item.rerouted, 0),
    coolingKeys: (payload?.health.keys ?? []).filter((key) => key.state !== "ok").length,
    openBreakers: (payload?.health.providers ?? []).filter((item) => item.breakerOpen).length,
  };
}

export function successRate(counts: UsageCounts): number | null {
  return counts.calls > 0 ? (counts.calls - counts.errors) / counts.calls : null;
}
