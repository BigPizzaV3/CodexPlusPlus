import assert from "node:assert/strict";
import test from "node:test";
import {
  busyLinks,
  clockTime,
  diffFlow,
  flowColumns,
  flowElapsed,
  flowStatus,
  MAX_PULSES_PER_TICK,
  pulseHops,
  type FlowEntry,
} from "./request-flow-model.ts";

function entry(id: string, fields: Partial<FlowEntry> = {}): FlowEntry {
  return { id, agent: "claude", startedAtMs: 1_000, timestampMs: 2_000, durationMs: 1_000, model: "m", relayId: "ds", relayName: "DeepSeek", success: true, statusCode: 200, ...fields };
}

test("请求状态：进行中 / 成功 / 失败", () => {
  assert.equal(flowStatus(entry("a", { state: "running", success: false })), "running");
  assert.equal(flowStatus(entry("a")), "ok");
  assert.equal(flowStatus(entry("a", { success: false })), "error");
  assert.equal(flowStatus(entry("a", { statusCode: 429 })), "error");
});

test("第一轮只记状态，不回放历史请求", () => {
  const { seen, pulses } = diffFlow(null, [entry("a"), entry("b", { state: "running" })], 0);
  assert.equal(pulses.length, 0);
  assert.equal(seen.get("a"), "ok");
  assert.equal(seen.get("b"), "running");
});

test("新请求发去程，结束发回程，一轮内开始又结束的先去后回", () => {
  const first = diffFlow(null, [entry("old")], 0);
  const second = diffFlow(first.seen, [entry("old"), entry("live", { state: "running", startedAtMs: 3_000 }), entry("quick", { startedAtMs: 2_000, success: false })], 10);
  assert.deepEqual(
    second.pulses.map((pulse) => [pulse.seq, pulse.kind, pulse.tone]),
    [[10, "round", "error"], [11, "out", "running"]],
  );
  const third = diffFlow(second.seen, [entry("old"), entry("live", { startedAtMs: 3_000 }), entry("quick", { success: false })], 12);
  assert.deepEqual(third.pulses.map((pulse) => [pulse.seq, pulse.kind, pulse.tone, pulse.agent, pulse.relayId]), [[12, "back", "ok", "claude", "ds"]]);
  // 状态没变不再放。
  assert.equal(diffFlow(third.seen, [entry("live", { startedAtMs: 3_000 })], 13).pulses.length, 0);
});

test("积压很多时只放最近的几颗", () => {
  const first = diffFlow(null, [], 0);
  const burst = Array.from({ length: 20 }, (_, index) => entry(`r${index}`, { startedAtMs: index }));
  const { pulses } = diffFlow(first.seen, burst, 0);
  assert.equal(pulses.length, MAX_PULSES_PER_TICK);
  assert.deepEqual(pulses.map((pulse) => pulse.seq), [0, 1, 2, 3, 4, 5]);
});

test("光点路线：去程 Agent → hub → 供应商，回程倒着走；没有供应商只走 Agent 一段", () => {
  assert.deepEqual(pulseHops({ agent: "claude", relayId: "ds", kind: "out" }), [
    { key: "agent:claude>hub", reverse: false },
    { key: "hub>provider:ds", reverse: false },
  ]);
  assert.deepEqual(pulseHops({ agent: "claude", relayId: "ds", kind: "back" }), [
    { key: "hub>provider:ds", reverse: true },
    { key: "agent:claude>hub", reverse: true },
  ]);
  assert.equal(pulseHops({ agent: "codex", relayId: "ds", kind: "round" }).length, 4);
  assert.deepEqual(pulseHops({ agent: "codex", relayId: "", kind: "out" }), [{ key: "agent:codex>hub", reverse: false }]);
});

test("进行中的请求所在连线标成流动", () => {
  const keys = busyLinks([entry("a", { state: "running" }), entry("b", { agent: null, relayId: "kimi" })]);
  assert.deepEqual([...keys], ["agent:claude>hub", "hub>provider:ds"]);
});

test("托盘拓扑两侧：按最近使用排序、去重、截断，进行中的标 live", () => {
  const columns = flowColumns([
    entry("a", { startedAtMs: 1, agent: "grok", relayId: "kimi", relayName: "Kimi" }),
    entry("b", { startedAtMs: 3, state: "running" }),
    entry("c", { startedAtMs: 2, agent: null, relayId: "" }),
  ], 2);
  assert.deepEqual(columns.agents.map((node) => [node.id, node.live]), [["claude", true], ["codex", false]]);
  assert.deepEqual(columns.providers.map((node) => [node.id, node.name, node.live]), [["ds", "DeepSeek", true], ["kimi", "Kimi", false]]);
});

test("耗时：进行中的按现在走秒，结束的用记录的时长", () => {
  assert.equal(flowElapsed(entry("a", { state: "running", startedAtMs: 10_000 }), 13_400), 3_400);
  assert.equal(flowElapsed(entry("a", { durationMs: 15_000 }), 99_999), 15_000);
  assert.equal(flowElapsed(entry("a", { state: "running", startedAtMs: 20_000 }), 10_000), 0, "时钟回拨不出负数");
});

test("时间显示成时:分:秒", () => {
  const date = new Date(2026, 10, 1, 0, 30, 5);
  assert.equal(clockTime(date.getTime()), "00:30:05");
});
