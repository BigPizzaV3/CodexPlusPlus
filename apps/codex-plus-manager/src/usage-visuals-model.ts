import type { UsageRouteDay } from "./gateway-model.ts";

export const USAGE_DIMENSIONS = ["agent", "providerId", "model"] as const;
export type UsageDimension = (typeof USAGE_DIMENSIONS)[number];
export type UsageVisualFilters = Partial<Record<UsageDimension, string>>;
export type UsageVisualMetric = "tokens" | "calls";

const MAX_DAYS = 53 * 7;
const MAX_NAMED_NODES = 6;
const DAY_MS = 86_400_000;

type ObservedUsage = { calls: number; tokens: number; meteredCalls: number };
export type UsageCalendarDay = { date: string; usage?: ObservedUsage };

const dateAsUtc = (date: string) => new Date(`${date}T00:00:00Z`);
export const weekDay = (date: string) => (dateAsUtc(date).getUTCDay() + 6) % 7;

export function visualDays(days: readonly string[]): string[] {
  if (!days.length) return [];
  const end = dateAsUtc(days[days.length - 1]).getTime();
  const start = Math.max(dateAsUtc(days[0]).getTime(), end - (MAX_DAYS - 1) * DAY_MS);
  if (!Number.isFinite(start)) return [];
  return Array.from({ length: 1 + Math.round((end - start) / DAY_MS) }, (_, index) =>
    new Date(start + index * DAY_MS).toISOString().slice(0, 10));
}

export function matchUsageFilters(row: UsageRouteDay, filters: UsageVisualFilters): boolean {
  return USAGE_DIMENSIONS.every(dimension => filters[dimension] === undefined || filters[dimension] === row[dimension]);
}

export function usageCalendar(days: readonly string[], rows: readonly UsageRouteDay[], filters: UsageVisualFilters): UsageCalendarDay[] {
  const observed = new Map<string, ObservedUsage>();
  for (const row of rows) {
    let usage = observed.get(row.date);
    if (!usage) {
      usage = { calls: 0, tokens: 0, meteredCalls: 0 };
      observed.set(row.date, usage);
    }
    if (!matchUsageFilters(row, filters)) continue;
    usage.calls += row.calls;
    usage.tokens += row.inputTokens + row.outputTokens;
    usage.meteredCalls += row.tokenMeteredCalls ?? 0;
  }
  return days.map(date => ({ date, usage: observed.get(date) }));
}

export function calendarValue(usage: ObservedUsage | undefined, metric: UsageVisualMetric): number | undefined {
  if (!usage) return undefined;
  if (metric === "calls") return usage.calls;
  return usage.tokens > 0 || usage.meteredCalls >= usage.calls ? usage.tokens : undefined;
}

export function heatLevel(value: number | undefined, peak: number): number {
  if (value === undefined) return -1;
  if (value === 0) return 0;
  return Math.min(4, Math.max(1, Math.ceil(value / Math.max(1, peak) * 4)));
}

export type UsageNode = {
  id: string;
  /** Undefined only for the Other bucket; the empty string remains a filterable unknown. */
  key?: string;
  providerId?: string;
  value: number;
  folded: number;
  y: number;
  height: number;
};

type UsageLink = {
  id: string;
  stage: number;
  source: UsageNode;
  target: UsageNode;
  sourceTop: number;
  targetTop: number;
  height: number;
  value: number;
};

export const FLOW_HEIGHT = 248;

export function usageFlow(rows: readonly UsageRouteDay[], metric: UsageVisualMetric) {
  const weight = (row: UsageRouteDay) => metric === "calls" ? row.calls : row.inputTokens + row.outputTokens;
  const weighted = rows.filter(row => weight(row) > 0);
  const identity = (row: UsageRouteDay, stage: number) => stage === 2 && row.model
    ? JSON.stringify([row.providerId, row.model])
    : row[USAGE_DIMENSIONS[stage]];
  const columns = USAGE_DIMENSIONS.map((dimension, stage) => {
    const groups = new Map<string, { key: string; providerId?: string; value: number }>();
    for (const row of weighted) {
      const id = identity(row, stage);
      const group = groups.get(id);
      if (group) group.value += weight(row);
      else groups.set(id, { key: row[dimension], providerId: stage === 2 && row.model ? row.providerId : undefined, value: weight(row) });
    }
    const named = [...groups].filter(([id]) => id !== "").sort(([a, left], [b, right]) => right.value - left.value || a.localeCompare(b));
    const kept = named.slice(0, MAX_NAMED_NODES);
    if (groups.has("")) kept.push(["", groups.get("")!]);
    const nodes: UsageNode[] = kept.map(([id, group]) => ({
      id: `${stage}:${JSON.stringify(id)}`, ...group, folded: 0, y: 0, height: 0,
    }));
    const byKey = new Map(kept.map(([id], index) => [id, nodes[index]]));
    const rest = named.slice(MAX_NAMED_NODES);
    if (rest.length) {
      const other: UsageNode = {
        id: `${stage}:other`, value: 0, folded: rest.length, y: 0, height: 0,
      };
      for (const [id, group] of rest) {
        other.value += group.value;
        byKey.set(id, other);
      }
      nodes.push(other);
    }
    return { byKey, nodes };
  });
  const total = weighted.reduce((sum, row) => sum + weight(row), 0);
  const gap = 25;
  const plot = FLOW_HEIGHT - (Math.max(1, ...columns.map(column => column.nodes.length)) - 1) * gap;
  const scale = total === 0 ? 0 : plot / total;
  for (const column of columns) {
    let y = (FLOW_HEIGHT - plot - Math.max(0, column.nodes.length - 1) * gap) / 2;
    for (const node of column.nodes) {
      node.y = y;
      node.height = node.value * scale;
      y += node.height + gap;
    }
  }

  // Count each observed joint route once in each adjacent stage. Folding nodes
  // changes the label, never the link total; independent margins cannot do this.
  const links: UsageLink[] = [];
  for (const stage of [0, 1]) {
    const pairs = new Map<string, { source: UsageNode; target: UsageNode; value: number }>();
    for (const row of weighted) {
      const source = columns[stage].byKey.get(identity(row, stage))!;
      const target = columns[stage + 1].byKey.get(identity(row, stage + 1))!;
      const key = `${source.id}>${target.id}`;
      const pair = pairs.get(key);
      if (pair) pair.value += weight(row);
      else pairs.set(key, { source, target, value: weight(row) });
    }
    const sourceOffsets = new Map<string, number>();
    const targetOffsets = new Map<string, number>();
    for (const pair of [...pairs.values()].sort((a, b) => a.source.y - b.source.y || a.target.y - b.target.y)) {
      const sourceTop = sourceOffsets.get(pair.source.id) ?? pair.source.y;
      const targetTop = targetOffsets.get(pair.target.id) ?? pair.target.y;
      const height = pair.value * scale;
      sourceOffsets.set(pair.source.id, sourceTop + height);
      targetOffsets.set(pair.target.id, targetTop + height);
      links.push({ ...pair, id: `${pair.source.id}>${pair.target.id}`, stage, sourceTop, targetTop, height });
    }
  }
  return { columns: columns.map(column => column.nodes), links, total };
}
