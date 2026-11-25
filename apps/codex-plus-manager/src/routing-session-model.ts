import { flowAgent, flowStatus, type FlowEntry } from "./request-flow-model.ts";

export function routingSessionKey(entry: FlowEntry) {
  const thread = entry.threadId?.trim();
  return JSON.stringify(thread ? [flowAgent(entry), thread] : [entry.id]);
}

export function requestTime(entry: FlowEntry) {
  return entry.startedAtMs || entry.timestampMs;
}

export function groupRoutingRequests(entries: FlowEntry[]) {
  const groups = new Map<string, { key: string; thread?: string; title?: string; requests: FlowEntry[] }>();
  for (const entry of [...entries].sort((a, b) => requestTime(b) - requestTime(a))) {
    const key = routingSessionKey(entry);
    let group = groups.get(key);
    if (!group) {
      group = { key, thread: entry.threadId?.trim() || undefined, requests: [] };
      groups.set(key, group);
    }
    group.title ??= entry.threadTitle?.trim() || undefined;
    group.requests.push(entry);
  }
  return [...groups.values()];
}

export function isContextCompaction(entry: FlowEntry) {
  return /\/responses\/compact\/?(?:\?|$)/.test(entry.path ?? "");
}

export function routingPathTargets(entry: FlowEntry) {
  const used = entry.route?.steps.filter(step => step.outcome === "answered").at(-1);
  const isUsed = (target: { relayId: string; model: string }) =>
    used?.relayId === target.relayId && used.model === target.model;
  const plan = entry.route?.planned ?? [];
  const targets = plan.length ? [...plan] : [];
  if (used && !targets.some(isUsed)) {
    const { relayId, relayName, model } = used;
    targets.push({ relayId, relayName, model });
  } else if (!targets.length && entry.relayId) {
    targets.push({ relayId: entry.relayId, relayName: entry.relayName ?? "", model: entry.upstreamModel || entry.model });
  }
  const compact = targets.filter((target, index) => index < 3 || isUsed(target));
  return { targets, compact, used };
}

export function requestContext(entry: FlowEntry) {
  const valid = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value) && value >= 0;
  const input = valid(entry.usage?.inputTokens) ? entry.usage.inputTokens : null;
  const reportedWindow = entry.route?.contextWindow;
  const window = valid(reportedWindow) && reportedWindow > 0 ? reportedWindow : null;
  const cache = entry.usage?.cachedTokens;
  const cached = (entry.usage?.cachedTokensKnown ?? (valid(cache) && cache > 0))
    && input !== null && valid(cache) && cache <= input ? cache : null;
  return { input, window, cached, ratio: input !== null && window !== null ? input / window : null };
}

export function contextSeries(requests: FlowEntry[]) {
  return [...requests].sort((a, b) => requestTime(a) - requestTime(b)).map(entry => {
    const context = requestContext(entry);
    const printable = flowStatus(entry) === "ok" && !isContextCompaction(entry);
    return { entry, ratio: printable ? context.ratio : null };
  });
}

export function selectRoutingRequest<G extends { key: string; requests: FlowEntry[] }>(
  matches: { group: G; shows: FlowEntry[] }[],
  newest: FlowEntry | undefined,
  pinned: { request: string; session: string } | undefined,
  sessionView: boolean,
) {
  const active = matches.find(({ group, shows }) => group.key === pinned?.session
    && (sessionView ? group.requests : shows).some(entry => entry.id === pinned.request))
    ?? matches.find(({ group }) => newest && group.key === routingSessionKey(newest));
  const selected = active && (sessionView ? active.group.requests : active.shows).find(entry => entry.id === pinned?.request);
  return { active, entry: selected ?? active?.shows[0], pinned: Boolean(selected) };
}
