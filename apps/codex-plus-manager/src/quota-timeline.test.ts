import assert from "node:assert/strict";
import test from "node:test";
import { quotaCalendarRange, quotaCalendarSlots, quotaCursorReading, quotaCycle, quotaPace, quotaPercentLeft, quotaRange, quotaSegments, quotaTimeScale, quotaTrend } from "./quota-timeline.ts";

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

test("仅用同一个窗口新鲜且发生变化的读数推算耗尽", () => {
  const now = 180_000_000;
  const minute = 60_000;
  const point = (ago: number, usedPercent: number, shifts = 0) => ({
    windowMinutes: 300, usedPercent, observedAtMs: now - ago * minute, resetsAtMs: now + (120 + shifts) * minute,
  });
  const current = point(0, 80);
  const quota = { updatedAtMs: now, windows: [current] };
  const forecast = quotaTrend([point(30, 50), point(15, 65), current], 300, now, quota).forecast;
  assert.equal(forecast?.perHour, 60);
  assert.equal(forecast?.endAt, now + 20 * minute);
  assert.ok(forecast?.endsEmpty);
  assert.equal(quotaTrend([point(30, 79.5), point(15, 79.9), current], 300, now, quota).reason, "flat");
  assert.equal(quotaTrend([point(30, 99), point(15, 75), current], 300, now, quota).segments.length, 2);
  assert.equal(quotaTrend([point(30, 50, -30), point(15, 65), current], 300, now, quota).reason, "insufficient");
  assert.equal(quotaTrend([point(30, 50), point(15, 65), current], 300, now + 11 * minute, quota).reason, "stale");
  assert.equal(quotaTrend([point(120, 50), point(15, 65), current], 300, now, quota).reason, "insufficient");
});

test("匀速基线比较在可信周期内的最新观测，而非把它当预测", () => {
  const now = 120_000_000;
  const half = 150 * 60_000;
  const window = { windowMinutes: 300, usedPercent: 65, resetsAtMs: now + half, observedAtMs: now };
  const quota = { updatedAtMs: now, windows: [window] };
  assert.equal(quotaPace(quota, 300, now), 15);
  assert.equal(quotaPace(quota, 300, now + 601_000), undefined);
  assert.equal(quotaPace({ ...quota, windows: [{ ...window, resetsAtMs: now + 3 * half }] }, 300, now), undefined);
  assert.equal(quotaPace(quota, null, now), undefined);
});

test("走势以完整重置窗为横轴，保留单点并跨采样间隔连接同窗读数", () => {
  const hour = 3_600_000;
  const now = 1000 * hour;
  const reset = now + 2 * hour;
  const window = { windowMinutes: 300, usedPercent: 22, resetsAtMs: reset };
  const point = { ...window, usedPercent: 14, observedAtMs: now - 2 * hour };
  const cycle = quotaCycle([point, { ...point, resetsAtMs: reset - 5 * hour }], window, now, now);
  assert.equal(cycle.start, reset - 5 * hour);
  assert.equal(cycle.end, reset);
  assert.deepEqual(cycle.points.map(sample => sample.usedPercent), [14, 22]);
  assert.equal(cycle.current?.fullAt, cycle.start);
  assert.equal(quotaCycle([], window, now, now).points.length, 1);
  const refill = quotaCycle([{ ...point, usedPercent: 44 }], window, now, now);
  assert.equal(refill.cycles.length, 2);
  assert.equal(refill.current?.fullAt, undefined, "a reported refill alone does not prove 100% at a known time");
});

test("近期趋势拟合使用中间读数，有限外推不被当成重置时读数", () => {
  const hour = 3_600_000;
  const now = 1000 * hour;
  const reset = now + 4 * hour;
  const point = (ago: number, usedPercent: number) => ({
    windowMinutes: 300, usedPercent, observedAtMs: now - ago * hour, resetsAtMs: reset,
  });
  const current = point(0, 24);
  const quota = { updatedAtMs: now, windows: [current] };
  const forecast = quotaTrend([point(.5, 20), point(.4, 20), point(.25, 20), current], 300, now, quota).forecast!;
  assert.notEqual(forecast.perHour, 8, "not the difference between the endpoints");
  assert.equal(forecast.endAt, now + 2 * hour);
  assert.equal(forecast.reachesReset, false);
  assert.ok(Math.abs(forecast.endLeft - (76 - 2 * forecast.perHour)) < 1e-9);
});

test("任意时长的配额窗每次确实重置后重新起笔，满额起点不计入观测", () => {
  const hour = 3_600_000;
  const point = (at: number, reset: number, usedPercent: number) =>
    ({ observedAtMs: at * hour, resetsAtMs: reset * hour, usedPercent, windowMinutes: 180 });
  const samples = [point(11, 13, 20), point(12, 13, 80), point(14, 16, 10), point(15, 16, 20)];
  const cycle = quotaCycle(samples, samples[3], 15 * hour, 15 * hour);
  assert.deepEqual(cycle.cycles.map(item => item.points.map(sample => sample.usedPercent)), [[20, 80], [10, 20]]);
  assert.deepEqual(cycle.cycles.map(item => item.fullAt), [10 * hour, 13 * hour]);
  assert.equal(cycle.points.length, 2);
  assert.equal(cycle.end, 16 * hour);
});

test("滚动窗口和小幅重置时间漂移不断线，也不伪造起点", () => {
  const minute = 60_000;
  const fixed = { observedAtMs: 510 * minute, resetsAtMs: 610 * minute, usedPercent: 28, windowMinutes: 180 };
  const prior = { ...fixed, observedAtMs: 480 * minute, usedPercent: 30 };
  const rolling = { ...prior, resetsAtMs: fixed.resetsAtMs - 30 * minute };
  const rounding = { ...prior, resetsAtMs: fixed.resetsAtMs + 2 * minute };
  for (const previous of [rolling, rounding]) {
    const cycle = quotaCycle([previous], fixed, fixed.observedAtMs, fixed.observedAtMs);
    assert.equal(cycle.cycles.length, 1);
    assert.equal(cycle.points.length, 2);
    assert.equal(cycle.current?.fullAt, undefined);
  }
  const monthly = { ...fixed, windowMinutes: null };
  assert.equal(quotaCycle([], monthly, fixed.observedAtMs, fixed.observedAtMs).current?.fullAt, undefined);
});

test("短窗口局部放大，段间边界及悬停时间可往返映射", () => {
  const scale = quotaTimeScale(0, 168, { start: 55, end: 60 });
  assert.equal(scale.expanded, true);
  assert.deepEqual(scale.breakpoints, [55, 60]);
  assert.ok(Math.abs(scale.position(60) - scale.position(55) - .36) < 1e-9);
  for (const at of [0, 20, 55, 57, 60, 100, 168]) assert.ok(Math.abs(scale.timeAt(scale.position(at)) - at) < 1e-9);
  for (const focus of [{ start: 10, end: 65 }, { start: -1, end: 2 }, undefined]) {
    const linear = quotaTimeScale(0, 100, focus);
    assert.equal(linear.expanded, false);
    assert.equal(linear.timeAt(.52), 52);
  }
  for (const focus of [{ start: 0, end: 5 }, { start: 95, end: 100 }]) {
    const edge = quotaTimeScale(0, 100, focus);
    assert.equal(edge.timeAt(0), 0);
    assert.equal(edge.timeAt(1), 100);
    assert.ok(Math.abs(edge.timeAt(edge.position(97)) - 97) < 1e-9);
  }
});

test("悬停只显示周期内实测、已知起点或预测，不拿别的周期补缺口", () => {
  const point = (observedAtMs: number, usedPercent: number) => ({ observedAtMs, usedPercent, windowMinutes: 300, resetsAtMs: null });
  const cycles = [{ fullAt: 10, points: [point(13, 9), point(15, 20)] }, { points: [point(30, 20), point(38, 50)] }];
  const forecast = { endAt: 46, endLeft: 10 };
  assert.deepEqual(quotaCursorReading(cycles, 11, forecast), { kind: "start", at: 10, left: 100 });
  assert.deepEqual(quotaCursorReading(cycles, 14, forecast), { kind: "sample", at: 13, left: 91 });
  assert.equal(quotaCursorReading(cycles, 20, forecast), undefined);
  assert.deepEqual(quotaCursorReading(cycles, 42, forecast), { kind: "forecast", at: 42, left: 30 });
  assert.equal(quotaCursorReading(cycles, 47, forecast), undefined);
});

test("配额日历按实际窗口选择日、两周或月刻度，翻页衔接", () => {
  const now = new Date(2026, 10, 24, 13).getTime();
  assert.equal(quotaCalendarRange(now, 180, 0).mode, "day");
  assert.equal(quotaCalendarRange(now, 2880, 0).mode, "fortnight");
  assert.equal(quotaCalendarRange(now, null, 0).mode, "month");
  for (const minutes of [180, 2880, null]) {
    const today = quotaCalendarRange(now, minutes, 0);
    assert.ok(today.start <= now && now < today.end);
    assert.equal(today.ticks[0], today.start);
    assert.equal(quotaCalendarRange(now, minutes, -1).end, today.start);
    assert.equal(quotaCalendarRange(now, minutes, 1).start, today.end);
  }
});

test("配额日历只用观测确认历史，区分当前和推算；时长未知时只标重置", () => {
  const hour = 3_600_000;
  const earlier = { windowMinutes: 180, usedPercent: 29, observedAtMs: 12 * hour, resetsAtMs: 13 * hour };
  const current = { ...earlier, usedPercent: 51, observedAtMs: 15 * hour, resetsAtMs: 16 * hour };
  const now = current.observedAtMs;
  const cycle = quotaCycle([earlier], current, now, now);
  const slots = quotaCalendarSlots(cycle, 180, now, { start: 0, end: 20 * hour });
  assert.deepEqual(slots.map(slot => [slot.kind, slot.start! / hour, slot.resetAt / hour]), [
    ["history", 10, 13], ["current", 13, 16], ["projected", 16, 19], ["projected", 19, 22],
  ]);
  assert.deepEqual(quotaCalendarSlots(cycle, 180, now, { start: 0, end: 10 * hour }), []);
  assert.deepEqual(quotaCalendarSlots(cycle, 180, 17 * hour, { start: 16 * hour, end: 20 * hour }), []);
  const monthly = { ...current, windowMinutes: null };
  const markers = quotaCalendarSlots(quotaCycle([], monthly, now, now), null, now, { start: 0, end: 32 * hour });
  assert.equal(markers.length, 1);
  assert.equal(markers[0].kind, "current");
  assert.equal(markers[0].start, undefined);
  assert.equal(markers[0].resetAt, 16 * hour);
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
