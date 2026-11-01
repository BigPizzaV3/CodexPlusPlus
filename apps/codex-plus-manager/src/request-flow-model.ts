/**
 * 实时请求流的纯数据逻辑：托盘「流量」页签和总览拓扑图共用。
 * 数据来自 `load_request_flow`（请求日志末尾 + 进行中的请求）；按当前进行中的请求分组，
 * 沿 Agent → Codex++ → 供应商的完整路线流动，响应反向，结束即停止。
 * 页面只负责画和动画。
 */
import { maskAccountName } from "./gateway-model.ts";

/** `load_request_flow` 返回的一条请求（请求日志条目里用得到的字段）。 */
export type FlowEntry = {
  id: string;
  agent?: string | null;
  startedAtMs: number;
  timestampMs: number;
  durationMs: number;
  model: string;
  upstreamModel?: string;
  relayId?: string;
  relayName?: string;
  statusCode?: number;
  success?: boolean;
  state?: string | null;
  phase?: string | null;
  attempts?: number;
  firstTokenMs?: number | null;
  estimatedOutputTokens?: number | null;
  usage?: { inputTokens: number; outputTokens: number } | null;
  error?: string | null;
};

export type FlowStatus = "running" | "ok" | "error";

export function flowStatus(entry: FlowEntry): FlowStatus {
  if (entry.state === "running") return "running";
  return entry.success !== false && (entry.statusCode ?? 200) < 400 ? "ok" : "error";
}

/** 发起请求的 Agent：没写的是 Codex 直连代理的记录。 */
export function flowAgent(entry: FlowEntry): string {
  return entry.agent?.trim() || "codex";
}

/** 拓扑图连线的键，和 HubTopology 里的 `${from}>${to}` 一致。 */
export function agentLinkKey(agent: string): string {
  return `agent:${agent}>hub`;
}

export function providerLinkKey(relayId: string): string {
  return `hub>provider:${relayId}`;
}

export type FlowHop = { key: string; reverse: boolean };

/** 光点依次走过的连线；回程倒着走。 */
export function pulseHops(pulse: { agent: string; relayId: string; kind: "out" | "back" }): FlowHop[] {
  const out: FlowHop[] = [{ key: agentLinkKey(pulse.agent), reverse: false }];
  if (pulse.relayId) out.push({ key: providerLinkKey(pulse.relayId), reverse: false });
  const back = [...out].reverse().map((hop) => ({ ...hop, reverse: true }));
  if (pulse.kind === "out") return out;
  return back;
}

/** 有请求正在进行的连线：画成流动的虚线。 */
export function busyLinks(entries: FlowEntry[]): Set<string> {
  const keys = new Set<string>();
  for (const entry of entries) {
    if (flowStatus(entry) !== "running") continue;
    keys.add(agentLinkKey(flowAgent(entry)));
    if (entry.relayId) keys.add(providerLinkKey(entry.relayId));
  }
  return keys;
}

export type LiveFlowRoute = { key: string; agent: string; relayId: string; reverse: boolean; count: number };

/** 按路线和方向分组，发送和响应可以同时经过同一根连线。完成请求不留动画。 */
export function liveFlowRoutes(entries: FlowEntry[]): LiveFlowRoute[] {
  const routes = new Map<string, LiveFlowRoute>();
  for (const entry of entries) {
    if (flowStatus(entry) !== "running") continue;
    const agent = flowAgent(entry);
    const relayId = entry.relayId ?? "";
    const reverse = returningLinks([entry]).size > 0;
    const key = JSON.stringify([agent, relayId, reverse]);
    const route = routes.get(key) ?? { key, agent, relayId, reverse, count: 0 };
    route.count += 1;
    routes.set(key, route);
  }
  return [...routes.values()];
}

/** 在整条路线的累计长度上采样，不在每个节点重置速度或循环。 */
export function flowRoutePosition(lengths: number[], distance: number): { segment: number; progress: number } | null {
  if (distance < 0) return null;
  let start = 0;
  for (let segment = 0; segment < lengths.length; segment += 1) {
    const length = lengths[segment];
    if (length > 0 && distance <= start + length) return { segment, progress: (distance - start) / length };
    start += length;
  }
  return null;
}

/** 已收到上游首字后，响应沿连线反向流回 Agent。 */
export function returningLinks(entries: FlowEntry[]): Set<string> {
  const keys = new Set<string>();
  for (const entry of entries) {
    if (flowStatus(entry) !== "running") continue;
    if (entry.phase === "retrying" || entry.phase === "waiting" || entry.phase === "requesting") continue;
    if (entry.phase !== "generating" && entry.firstTokenMs == null && (entry.estimatedOutputTokens ?? 0) <= 0) continue;
    keys.add(agentLinkKey(flowAgent(entry)));
    if (entry.relayId) keys.add(providerLinkKey(entry.relayId));
  }
  return keys;
}

export type FlowColumnNode = { id: string; name: string; live: boolean; count: number };

/** 托盘小拓扑图两侧的节点：最近请求里出现过的 Agent 和供应商，最近用过的在上。 */
export function flowColumns(entries: FlowEntry[], maxPerSide = 3): { agents: FlowColumnNode[]; providers: FlowColumnNode[] } {
  const agents = new Map<string, FlowColumnNode>();
  const providers = new Map<string, FlowColumnNode>();
  const ordered = [...entries].sort((left, right) => right.startedAtMs - left.startedAtMs);
  for (const entry of ordered) {
    const live = flowStatus(entry) === "running";
    const agent = flowAgent(entry);
    const a = agents.get(agent) ?? { id: agent, name: agent, live: false, count: 0 };
    a.live = a.live || live;
    a.count += 1;
    agents.set(agent, a);
    const relayId = entry.relayId ?? "";
    if (!relayId) continue;
    const p = providers.get(relayId) ?? { id: relayId, name: maskAccountName(entry.relayName || relayId), live: false, count: 0 };
    p.live = p.live || live;
    p.count += 1;
    providers.set(relayId, p);
  }
  return { agents: [...agents.values()].slice(0, maxPerSide), providers: [...providers.values()].slice(0, maxPerSide) };
}

/** 耗时：进行中的按现在算（界面每轮刷新时走秒），结束的用记录的时长。 */
export function flowElapsed(entry: FlowEntry, now: number): number {
  if (flowStatus(entry) === "running") return Math.max(0, now - (entry.startedAtMs || now));
  return Math.max(0, entry.durationMs);
}

/** 悬停提示里的开始时间：时:分:秒。 */
export function clockTime(ms: number): string {
  const date = new Date(ms);
  return [date.getHours(), date.getMinutes(), date.getSeconds()].map((part) => String(part).padStart(2, "0")).join(":");
}
