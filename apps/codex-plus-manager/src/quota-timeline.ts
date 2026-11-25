import { quotaWindowFresh, type ProviderQuota, type QuotaObservation, type QuotaWindow } from "./gateway-model.ts";

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

const HOUR = 3_600_000;
const windowHours = (minutes: number | null) => minutes == null ? 30 * 24 : minutes / 60;

/** Percentage-point distance from spending uniformly in this known-length window. */
export function quotaPace(quota: ProviderQuota | null | undefined, minutes: number | null, now: number, maxAgeSeconds = 600): number | undefined {
  const window = quota?.windows.find(item => item.windowMinutes === minutes);
  if (!quota || !window || minutes == null || minutes <= 0 || window.resetsAtMs == null
      || !quotaWindowFresh(quota, window, now, maxAgeSeconds)) return undefined;
  const length = minutes * 60_000;
  const observed = window.observedAtMs ?? quota.updatedAtMs;
  const left = window.resetsAtMs - observed;
  if (left <= 0 || left > length) return undefined;
  return window.usedPercent - (1 - left / length) * 100;
}

export function quotaTrend(history: QuotaObservation[], minutes: number | null, now: number, quota?: ProviderQuota | null, maxAgeSeconds = 600) {
  const hours = windowHours(minutes);
  const horizon = Math.min(35 * 24, Math.max(24, hours * 2)) * HOUR;
  const gapLimit = Math.min(18, Math.max(.75, hours / 12)) * HOUR;
  const ordered = history.filter(point => point.windowMinutes === minutes
    && point.observedAtMs >= now - horizon && point.observedAtMs <= now
    && point.usedPercent >= 0 && point.usedPercent <= 100).sort((a, b) => a.observedAtMs - b.observedAtMs);
  const segments: QuotaObservation[][] = [];
  for (const point of ordered) {
    const segment = segments.at(-1);
    const previous = segment?.at(-1);
    // 拟合比绘图严格，不跨回充或过大的采样间隔。
    if (!segment || !previous || quotaCycleBreak(previous, point) || point.usedPercent < previous.usedPercent
        || point.observedAtMs - previous.observedAtMs > gapLimit) segments.push([point]);
    else segment.push(point);
  }
  const last = ordered.at(-1);
  const recent = segments.at(-1) ?? [];
  const current = quota?.windows.find(item => item.windowMinutes === minutes);
  const reset = last?.resetsAtMs;
  const noForecast = (reason: "insufficient" | "stale" | "flat" | "exhausted") => ({ segments, last, forecast: undefined, reason });
  if (!last || reset == null || !quota || !current || current.resetsAtMs !== reset) return noForecast("insufficient");
  if (!quotaWindowFresh(quota, current, now, maxAgeSeconds)
      || (now - last.observedAtMs) > Math.max(maxAgeSeconds * 1000, gapLimit / 2)
      || Math.abs(current.usedPercent - last.usedPercent) > .1) return noForecast("stale");
  if (last.usedPercent >= 100) return noForecast("exhausted");
  const cohort = recent.filter(point => last.observedAtMs - point.observedAtMs <= Math.min(120, hours / 2) * HOUR);
  const first = cohort[0];
  const span = first ? last.observedAtMs - first.observedAtMs : 0;
  if (cohort.length < 3 || span < Math.min(4, Math.max(.25, hours / 15)) * HOUR) return noForecast("insufficient");
  const delta = last.usedPercent - first.usedPercent;
  if (delta < 1) return noForecast("flat");
  // 用近期全部读数拟合速率，预测线仍从最后一次实测起步。
  const n = cohort.length;
  const meanX = cohort.reduce((sum, point) => sum + (point.observedAtMs - first.observedAtMs) / HOUR, 0) / n;
  const meanY = cohort.reduce((sum, point) => sum + point.usedPercent, 0) / n;
  let covariance = 0;
  let variance = 0;
  for (const point of cohort) {
    const x = (point.observedAtMs - first.observedAtMs) / HOUR - meanX;
    covariance += x * (point.usedPercent - meanY);
    variance += x * x;
  }
  const perHour = covariance / variance;
  if (!(perHour > 0) || !Number.isFinite(perHour)) return noForecast("flat");
  const exhaustedAt = last.observedAtMs + (100 - last.usedPercent) / perHour * HOUR;
  const endAt = Math.min(reset, exhaustedAt, last.observedAtMs + span * 4);
  if (endAt <= now) return noForecast("stale");
  const endLeft = Math.max(0, 100 - last.usedPercent - (endAt - last.observedAtMs) / HOUR * perHour);
  return { segments, last, reason: undefined, forecast: { endAt, endLeft, perHour, endsEmpty: exhaustedAt <= endAt, reachesReset: endAt === reset } };
}

/** Keep a current reading visible even before a second persisted sample arrives. */
export function quotaCycle(history: QuotaObservation[], window: QuotaWindow, updatedAtMs: number, now: number) {
  const latest = { ...window, observedAtMs: window.observedAtMs ?? updatedAtMs };
  const end = window.resetsAtMs ?? now;
  const cycles = quotaRecordedCycles([...history, latest].filter(point =>
    point.windowMinutes === window.windowMinutes && point.observedAtMs <= now));
  const current = cycles.at(-1)?.points.at(-1)?.resetsAtMs === window.resetsAtMs ? cycles.at(-1) : undefined;
  const points = current?.points ?? [];
  const start = current?.fullAt ?? points[0]?.observedAtMs ?? end - 60_000;
  return { points, current, cycles, start: Math.min(start, end - 60_000), end };
}

function quotaCycleBreak(a: QuotaObservation, b: QuotaObservation): boolean {
  const refill = a.usedPercent - b.usedPercent > 5;
  if (a.resetsAtMs == null || b.resetsAtMs == null) return a.resetsAtMs !== b.resetsAtMs || refill;
  if (b.observedAtMs >= a.resetsAtMs) return true;
  const drift = b.resetsAtMs - a.resetsAtMs;
  const slack = 5 * 60_000;
  if (Math.abs(drift) <= slack) return refill;
  // 滚动窗口的重置时间随采样时间向前移动，不是每次采样都回满。
  return drift < 0 || Math.abs(drift - (b.observedAtMs - a.observedAtMs)) > slack;
}

export type QuotaRecordedCycle = { points: QuotaObservation[]; fullAt?: number };

export function quotaCalendarRange(now: number, minutes: number | null, page: number) {
  const left = new Date(now);
  left.setHours(0, 0, 0, 0);
  const mode = minutes === null || minutes > 14 * 1440 ? "month" : minutes <= 1440 ? "day" : "fortnight";
  if (mode === "month") {
    left.setDate(1);
    left.setMonth(left.getMonth() + page);
  } else {
    left.setDate(left.getDate() + (mode === "day" ? page : 14 * page - (left.getDay() + 6) % 7));
  }
  const right = new Date(left);
  if (mode === "month") right.setMonth(right.getMonth() + 1);
  else right.setDate(right.getDate() + (mode === "day" ? 1 : 14));
  const start = left.getTime();
  const end = right.getTime();
  const ticks: number[] = [];
  for (const date = new Date(left); date < right;) {
    ticks.push(date.getTime());
    if (mode === "day") date.setTime(date.getTime() + 3 * HOUR);
    else date.setDate(date.getDate() + (mode === "month" ? 3 : 1));
  }
  return { mode, start, end, ticks };
}

export function quotaCalendarSlots(cycle: ReturnType<typeof quotaCycle>, minutes: number | null, now: number, range: { start: number; end: number }) {
  type Slot = { start?: number; resetAt: number; sample?: QuotaObservation; kind: "current" | "history" | "projected" };
  const slots: Slot[] = [];
  for (const [index, group] of cycle.cycles.entries()) {
    const sample = group.points.at(-1);
    if (sample?.resetsAtMs == null) continue;
    const kind = group === cycle.current ? (sample.resetsAtMs > now ? "current" : "history") : "history";
    // 提前重置时，旧周期按下一周期重新观测到的边界收尾。
    const next = cycle.cycles[index + 1];
    const resetAt = next ? Math.min(sample.resetsAtMs, next.fullAt ?? next.points[0].observedAtMs) : sample.resetsAtMs;
    slots.push({ start: group.fullAt, resetAt, sample, kind });
  }
  const current = slots.at(-1);
  const length = (minutes ?? 0) * 60_000;
  if (current?.kind === "current" && current.start !== undefined && length > 0) {
    if (length >= (range.end - range.start) / 96) {
      const first = Math.max(current.resetAt, current.resetAt + Math.floor((range.start - current.resetAt) / length) * length);
      for (let start = first; start < range.end; start += length) {
        slots.push({ start, resetAt: start + length, kind: "projected" });
      }
    } else {
      slots.push({ start: current.resetAt, resetAt: current.resetAt + length, kind: "projected" });
    }
  }
  return slots.filter(slot => slot.start === undefined
    ? slot.resetAt >= range.start && slot.resetAt < range.end
    : slot.start < range.end && slot.resetAt > range.start);
}

/** 同一分段轴用于绘制和命中测试；区间外仍按各段线性缩放。 */
export function quotaTimeScale(start: number, end: number, focus?: { start: number; end: number }) {
  const duration = Math.max(1, end - start);
  const minimumShare = .36;
  let time = [start, start + duration];
  let positions = [0, 1];
  if (focus && start <= focus.start && focus.end <= end && focus.end > focus.start
      && (focus.end - focus.start) / duration < minimumShare) {
    const before = focus.start - start;
    const after = end - focus.end;
    const outside = 1 - minimumShare;
    const left = before && after ? Math.max(.12, Math.min(outside - .12, outside * before / (before + after)))
      : before ? outside : 0;
    time = [start, focus.start, focus.end, end];
    positions = [0, left, left + minimumShare, 1];
  }
  const map = (value: number, domain: number[], range: number[]) => {
    if (value <= domain[0]) return range[0];
    for (let i = 1; i < domain.length; i++) {
      if (value <= domain[i] && domain[i] > domain[i - 1]) {
        return range[i - 1] + (value - domain[i - 1]) / (domain[i] - domain[i - 1]) * (range[i] - range[i - 1]);
      }
    }
    return range[range.length - 1];
  };
  return {
    expanded: time.length > 2,
    breakpoints: time.slice(1, -1),
    position: (at: number) => map(at, time, positions),
    timeAt: (fraction: number) => map(fraction, positions, time),
  };
}

export function quotaCursorReading(cycles: QuotaRecordedCycle[], at: number, forecast?: { endAt: number; endLeft: number }) {
  for (const { points, fullAt } of cycles) {
    const first = points[0];
    const last = points.at(-1);
    if (!first || !last) continue;
    if (fullAt !== undefined && fullAt <= at && at < first.observedAtMs) {
      return { kind: "start" as const, at: fullAt, left: 100 };
    }
    if (first.observedAtMs <= at && at <= last.observedAtMs) {
      const nearest = points.reduce((a, b) => Math.abs(at - a.observedAtMs) <= Math.abs(at - b.observedAtMs) ? a : b);
      return { kind: "sample" as const, at: nearest.observedAtMs, left: 100 - nearest.usedPercent };
    }
  }
  const latest = cycles.at(-1)?.points.at(-1);
  if (latest && forecast && latest.observedAtMs < at && at <= forecast.endAt) {
    const left = 100 - latest.usedPercent;
    const progress = (at - latest.observedAtMs) / (forecast.endAt - latest.observedAtMs);
    return { kind: "forecast" as const, at, left: left + (forecast.endLeft - left) * progress };
  }
}

/** 只从读数确认存在的周期；满额起点是参考，不写入读数。 */
function quotaRecordedCycles(history: QuotaObservation[]): QuotaRecordedCycle[] {
  const points = [...new Map(history.filter(point =>
    point.observedAtMs > 0
    && point.usedPercent >= 0 && point.usedPercent <= 100
    && (point.resetsAtMs == null || point.observedAtMs <= point.resetsAtMs),
  ).map(point => [point.observedAtMs, point])).values()].sort((a, b) => a.observedAtMs - b.observedAtMs);
  const groups: QuotaObservation[][] = [];
  for (const point of points) {
    const previous = groups.at(-1)?.at(-1);
    if (!previous || quotaCycleBreak(previous, point)) groups.push([point]);
    else groups.at(-1)!.push(point);
  }
  return groups.map((points, index) => {
    const first = points[0];
    const previous = groups[index - 1]?.at(-1);
    const length = (first.windowMinutes ?? 0) * 60_000;
    const start = first.resetsAtMs != null ? first.resetsAtMs - length : NaN;
    const fullAt = length > 0 && start > 0 && start <= first.observedAtMs
      && points.every(point => point.resetsAtMs === first.resetsAtMs)
      && (!previous || (start > previous.observedAtMs && (previous.resetsAtMs == null
        || previous.resetsAtMs > first.observedAtMs || start >= previous.resetsAtMs)))
      ? start : undefined;
    return { points, fullAt };
  });
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
