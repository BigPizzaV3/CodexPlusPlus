import assert from "node:assert/strict";
import test from "node:test";
import { requestRouteTiming, requestTimingStages } from "./request-route-timing.ts";

test("splits only observed request-clock timings", () => {
  assert.deepEqual(requestRouteTiming({ durationMs: 1100, firstTokenMs: 1000, stream: true }),
    { total: 1100, first: 1000, afterFirst: 100 });
  assert.deepEqual(requestRouteTiming({ durationMs: 1100, firstTokenMs: 1200, stream: true }),
    { total: 1100, first: null, afterFirst: null });
  assert.deepEqual(requestRouteTiming({ durationMs: 1100, firstTokenMs: 1000, stream: false }),
    { total: 1100, first: null, afterFirst: null });
  assert.deepEqual(requestRouteTiming({ durationMs: 0, firstTokenMs: 1000, stream: true, state: "running" }),
    { total: null, first: 1000, afterFirst: null });
});

test("joins request and route clocks only when the stages fit observed boundaries", () => {
  const entry = { startedAtMs: 1000, durationMs: 1180, firstTokenMs: 1072, stream: true,
    route: { finishedAtMs: 1535, elapsedMs: 530, steps: [{ outcome: "answered", elapsedMs: 524 }] } };
  assert.deepEqual(requestTimingStages(entry), [
    { key: "prepare", ms: 5 }, { key: "route", ms: 524 }, { key: "body", ms: 543 }, { key: "transfer", ms: 108 },
  ]);
  assert.deepEqual(requestTimingStages({ ...entry, firstTokenMs: 500 }), [
    { key: "first", ms: 500 }, { key: "transfer", ms: 680 },
  ]);
  assert.deepEqual(requestTimingStages({ ...entry, durationMs: 0, state: "running" }), [{ key: "first", ms: 1072 }]);
});
