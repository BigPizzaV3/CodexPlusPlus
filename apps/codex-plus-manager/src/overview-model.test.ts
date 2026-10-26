import assert from "node:assert/strict";
import { test } from "node:test";
import { DAY_MS, summarizeRecentRequests, type RequestLogLite } from "./overview-model.ts";

const now = 10 * DAY_MS;
const entry = (minutesAgo: number, patch: Partial<RequestLogLite> = {}): RequestLogLite => ({
  timestampMs: now - minutesAgo * 60_000, durationMs: 1000, success: true, model: "gpt-5", ...patch,
});

test("only requests inside the window are counted", () => {
  const summary = summarizeRecentRequests([
    entry(5),
    entry(60, { success: false, durationMs: 3000 }),
    entry(25 * 60), // 超出 24 小时
    entry(-5),      // 时间在未来（时钟漂移），不计
  ], now);
  assert.equal(summary.total, 2);
  assert.equal(summary.failed, 1);
  assert.equal(summary.successRate, 0.5);
  assert.equal(summary.avgDurationMs, 2000);
});

test("empty window has no rate or duration", () => {
  assert.deepEqual(summarizeRecentRequests([entry(30 * 60)], now), {
    total: 0, failed: 0, successRate: null, avgDurationMs: null, topModel: null,
  });
});

test("top model prefers the requested name, falls back to upstream, breaks ties by recency", () => {
  const summary = summarizeRecentRequests([
    entry(50, { model: "a" }),
    entry(40, { model: "b" }),
    entry(30, { model: "", upstreamModel: "a" }),
    entry(10, { model: "b" }),
  ], now);
  assert.equal(summary.topModel, "b");
  assert.equal(summarizeRecentRequests([entry(1, { model: " ", upstreamModel: "" })], now).topModel, null);
});
