import assert from "node:assert/strict";
import test from "node:test";
import { quotaPercentLeft, quotaRange, quotaSegments } from "./quota-timeline.ts";

test("只绘制官方窗口和后续推算，不编造过去窗口；边界裁剪", () => {
  const window = { windowMinutes: 300, usedPercent: 25, resetsAtMs: 30_000_000 };
  assert.deepEqual(quotaSegments(window, 0, 12_000_000), []);
  const result = quotaSegments(window, 20_000_000, 60_000_000);
  assert.deepEqual(result, [
    { start: 20_000_000, end: 30_000_000, resetAt: 30_000_000, projected: false },
    { start: 30_000_000, end: 48_000_000, resetAt: 48_000_000, projected: true },
    { start: 48_000_000, end: 60_000_000, resetAt: 66_000_000, projected: true },
  ]);
  assert.equal(quotaSegments(window, 30_000_000, 31_000_000)[0].projected, true);
  assert.deepEqual(quotaSegments({ ...window, windowMinutes: null }, 0, 60_000_000), []);
  assert.deepEqual(quotaSegments({ ...window, resetsAtMs: null }, 0, 60_000_000), []);
});

test("剩余比例不把过期、已重置或非法数据当作恢复额度", () => {
  const window = { windowMinutes: 300, usedPercent: 25, resetsAtMs: 1_000_000 };
  const quota = { updatedAtMs: 10_000, windows: [window] };
  assert.equal(quotaPercentLeft(quota, window, 10_001), 75);
  assert.equal(quotaPercentLeft(quota, window, 700_000), null);
  assert.equal(quotaPercentLeft(quota, window, 1_000_000, 2000), null);
  assert.equal(quotaPercentLeft(quota, { ...window, usedPercent: NaN }, 10_001), null);
  assert.equal(quotaPercentLeft(quota, { ...window, usedPercent: 100 }, 10_001), 0);
});

test("两周和日视图可前后导航并回到今天", () => {
  const now = new Date(2026, 10, 17, 15).getTime();
  const week = quotaRange(now, "week", 0);
  assert.equal(new Date(week.start).getDay(), 0);
  assert.equal(week.ticks.length, 15);
  assert.equal(quotaRange(now, "week", 1).start, week.end);
  const day = quotaRange(now, "fiveHour", 0);
  assert.equal(day.ticks.length, 9);
  assert.equal(new Date(day.start).getHours(), 0);
  assert.equal(quotaRange(now, "fiveHour", -1).end, day.start);
});
