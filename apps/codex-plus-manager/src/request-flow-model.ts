/**
 * 实时请求流的纯数据逻辑：托盘「流量」页签和总览拓扑图共用。
 * 数据来自 `load_request_flow`（请求日志末尾 + 进行中的请求）；每轮和上一轮对比，
 * 新出现的请求发一颗「去程」光点（Agent → Codex++ → 供应商），请求结束发一颗「回程」光点。
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

/**
 * 一颗光点：out 是去程，back 是回程，round 是一轮之间就开始又结束的请求（先去后回）。
 * `seq` 单调递增，画动画的一方据此判断哪些已经放过。
 */
export type FlowPulse = { seq: number; agent: string; relayId: string; kind: "out" | "back" | "round"; tone: FlowStatus };

export type FlowSeen = Map<string, FlowStatus>;

/** 一轮最多放这么多颗；面板隐藏后重新出现时积压的请求不全部回放。 */
export const MAX_PULSES_PER_TICK = 6;

/**
 * 对比上一轮看到的状态，得出这一轮要放的光点。第一轮（`seen` 为 null）只记状态，不回放历史。
 * 光点按请求开始时间从早到晚排，超出上限时留最近的。
 */
export function diffFlow(seen: FlowSeen | null, entries: FlowEntry[], nextSeq: number): { seen: FlowSeen; pulses: FlowPulse[] } {
  const next: FlowSeen = new Map();
  const changes: Array<{ entry: FlowEntry; kind: FlowPulse["kind"]; tone: FlowStatus }> = [];
  for (const entry of entries) {
    const status = flowStatus(entry);
    next.set(entry.id, status);
    if (!seen) continue;
    const before = seen.get(entry.id);
    if (before === undefined) changes.push({ entry, kind: status === "running" ? "out" : "round", tone: status });
    else if (before === "running" && status !== "running") changes.push({ entry, kind: "back", tone: status });
  }
  changes.sort((left, right) => left.entry.startedAtMs - right.entry.startedAtMs);
  const pulses = changes.slice(-MAX_PULSES_PER_TICK).map((change, index) => ({
    seq: nextSeq + index,
    agent: flowAgent(change.entry),
    relayId: change.entry.relayId ?? "",
    kind: change.kind,
    tone: change.tone,
  }));
  return { seen: next, pulses };
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
export function pulseHops(pulse: Pick<FlowPulse, "agent" | "relayId" | "kind">): FlowHop[] {
  const out: FlowHop[] = [{ key: agentLinkKey(pulse.agent), reverse: false }];
  if (pulse.relayId) out.push({ key: providerLinkKey(pulse.relayId), reverse: false });
  const back = [...out].reverse().map((hop) => ({ ...hop, reverse: true }));
  if (pulse.kind === "out") return out;
  if (pulse.kind === "back") return back;
  return [...out, ...back];
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
