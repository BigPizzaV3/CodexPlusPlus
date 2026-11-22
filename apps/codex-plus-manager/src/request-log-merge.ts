import { keepIfSame } from "./use-polling.ts";

/** 增量读请求日志的游标（后端 `RequestLogCursor`）：文件里最后读到的那一行的起始字节和请求 id。 */
export type RequestLogCursor = { offset: number; id: string };

type MergeableEntry = { id: string; startedAtMs: number; timestampMs?: number; state?: string | null };

function requestStartedAt(entry: MergeableEntry): number {
  return entry.startedAtMs > 0 ? entry.startedAtMs : entry.timestampMs ?? 0;
}

/** 整页重读回来的列表：内容没变的记录沿用旧对象（行组件靠引用判断要不要重渲染），整体没变时原样返回旧数组。 */
export function reuseRequestLogEntries<T extends MergeableEntry>(previous: T[], next: T[]): T[] {
  const previousById = new Map(previous.map((entry) => [entry.id, entry]));
  const entries = next.map((entry) => {
    const old = previousById.get(entry.id);
    return old ? keepIfSame(old, entry) : entry;
  });
  return entries.length === previous.length && entries.every((entry, index) => entry === previous[index]) ? previous : entries;
}

/**
 * 把增量读到的请求并进已有列表（请求日志页自动刷新用）：
 * - 增量结果带着现在全部进行中的请求，旧列表里的进行中记录以它为准（已结束的会作为新完成的记录回来）；
 * - 同 id 的新记录替换旧的，内容没变的沿用旧对象；
 * - 按开始时间倒序（和后端一致），丢掉时间段以外的，最多留 `limit` 条，截掉了就 `truncated`。
 * 什么都没变时 `entries` 就是传入的 `previous`。
 */
export function mergeRequestLogDelta<T extends MergeableEntry>(
  previous: T[],
  delta: T[],
  limit: number,
  sinceMs: number | null | undefined,
): { entries: T[]; truncated: boolean } {
  const previousById = new Map(previous.map((entry) => [entry.id, entry]));
  const incoming = new Map<string, T>();
  for (const entry of delta) {
    const old = previousById.get(entry.id);
    incoming.set(entry.id, old ? keepIfSame(old, entry) : entry);
  }
  const merged = [...incoming.values()];
  for (const entry of previous) {
    if (incoming.has(entry.id) || entry.state === "running") continue;
    merged.push(entry);
  }
  const inRange = sinceMs == null ? merged : merged.filter((entry) => requestStartedAt(entry) >= sinceMs);
  inRange.sort((a, b) => b.startedAtMs - a.startedAtMs);
  const truncated = inRange.length > limit;
  const entries = truncated ? inRange.slice(0, limit) : inRange;
  const unchanged = entries.length === previous.length && entries.every((entry, index) => entry === previous[index]);
  return { entries: unchanged ? previous : entries, truncated };
}
