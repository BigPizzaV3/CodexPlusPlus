// 概览页「最近请求」的统计：只看时间窗内的记录，空窗口返回 total = 0，由界面显示空状态。

export type RequestLogLite = {
  timestampMs: number;
  durationMs: number;
  success: boolean;
  model: string;
  upstreamModel?: string;
};

export type RecentRequestSummary = {
  total: number;
  failed: number;
  /** 0–1；没有请求时为 null。 */
  successRate: number | null;
  /** 毫秒；没有请求时为 null。 */
  avgDurationMs: number | null;
  /** 次数最多的模型（优先用 Codex 请求的模型名）；并列取最近出现的那个。 */
  topModel: string | null;
};

export const DAY_MS = 24 * 60 * 60 * 1000;

export function summarizeRecentRequests(
  entries: RequestLogLite[],
  nowMs: number,
  windowMs: number = DAY_MS,
): RecentRequestSummary {
  const recent = entries.filter((entry) => entry.timestampMs <= nowMs && nowMs - entry.timestampMs <= windowMs);
  if (!recent.length) {
    return { total: 0, failed: 0, successRate: null, avgDurationMs: null, topModel: null };
  }
  const succeeded = recent.filter((entry) => entry.success).length;
  const duration = recent.reduce((sum, entry) => sum + Math.max(0, entry.durationMs), 0);
  const counts = new Map<string, { count: number; latest: number }>();
  for (const entry of recent) {
    const model = (entry.model || entry.upstreamModel || "").trim();
    if (!model) continue;
    const current = counts.get(model) ?? { count: 0, latest: 0 };
    counts.set(model, { count: current.count + 1, latest: Math.max(current.latest, entry.timestampMs) });
  }
  let topModel: string | null = null;
  let best = { count: 0, latest: -1 };
  for (const [model, stat] of counts) {
    if (stat.count > best.count || (stat.count === best.count && stat.latest > best.latest)) {
      topModel = model;
      best = stat;
    }
  }
  return {
    total: recent.length,
    failed: recent.length - succeeded,
    successRate: succeeded / recent.length,
    avgDurationMs: Math.round(duration / recent.length),
    topModel,
  };
}
