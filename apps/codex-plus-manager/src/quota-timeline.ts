import { quotaWindowFresh, type ProviderQuota, type QuotaWindow } from "./gateway-model.ts";

export type QuotaSegment = { start: number; end: number; resetAt: number; projected: boolean };

/** 只画官方读数对应的窗口及其后续推算，不把推算伪装成历史记录。 */
export function quotaSegments(window: QuotaWindow, start: number, end: number): QuotaSegment[] {
  const duration = (window.windowMinutes ?? 0) * 60_000;
  const reset = window.resetsAtMs;
  if (!duration || !Number.isFinite(duration) || duration < 0 || reset == null || !Number.isFinite(reset) || end <= start) return [];
  const result: QuotaSegment[] = [];
  const first = Math.max(0, Math.floor((start - reset) / duration) + 1);
  for (let index = first; index < first + 100; index++) {
    const left = reset - duration + index * duration;
    const right = reset + index * duration;
    if (left >= end) break;
    if (right > start) result.push({ start: Math.max(start, left), end: Math.min(end, right), resetAt: right, projected: index > 0 });
  }
  return result;
}

export function quotaPercentLeft(quota: ProviderQuota, window: QuotaWindow, now: number, maxAgeSeconds = 600): number | null {
  return quotaWindowFresh(quota, window, now, maxAgeSeconds) ? Math.round(100 - window.usedPercent) : null;
}

/** 用本地日历对齐日期，避免夏令时切换后午夜错位。 */
export function quotaRange(now: number, mode: "week" | "fiveHour", offset: number) {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  if (mode === "week") start.setDate(start.getDate() - start.getDay() + offset * 14);
  else start.setDate(start.getDate() + offset);
  const ticks = Array.from({ length: mode === "week" ? 15 : 9 }, (_, index) => {
    const date = new Date(start);
    if (mode === "week") date.setDate(date.getDate() + index);
    else date.setHours(index * 3);
    return date.getTime();
  });
  return { start: ticks[0], end: ticks[ticks.length - 1], ticks };
}
