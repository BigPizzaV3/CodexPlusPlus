import assert from "node:assert/strict";
import test from "node:test";
import {
  busyLinks,
  returningLinks,
  liveFlowRoutes,
  flowRoutePosition,
  clockTime,
  flowColumns,
  flowElapsed,
  flowStatus,
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

test("真实生成阶段持续回流，等待重试和已完成请求不回流", () => {
  assert.equal(returningLinks([entry("a", { state: "running", phase: "waiting" })]).size, 0);
  assert.deepEqual([...returningLinks([entry("a", { state: "running", phase: "generating" })])], ["agent:claude>hub", "hub>provider:ds"]);
  assert.equal(returningLinks([entry("a", { state: "running", firstTokenMs: 0 })]).size, 2);
  assert.equal(returningLinks([entry("a", { state: "running", phase: "retrying", firstTokenMs: 100 })]).size, 0);
  assert.equal(returningLinks([entry("a", { phase: "generating", firstTokenMs: 100 })]).size, 0);
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

test("首次打开即恢复活动路线，历史成功失败不产生延迟光点", () => {
  const live = entry("live", { state: "running" });
  assert.equal(liveFlowRoutes([live]).length, 1);
  assert.equal(liveFlowRoutes([entry("done"), entry("error", { success: false })]).length, 0);
  assert.equal(liveFlowRoutes([]).length, 0);
});

test("同一 Agent 的并发发送和响应保持独立方向，同方向去重", () => {
  const routes = liveFlowRoutes([
    entry("a", { state: "running", phase: "waiting" }),
    entry("b", { state: "running", phase: "generating" }),
    entry("c", { state: "running", phase: "generating" }),
  ]);
  assert.deepEqual(routes.map(r => [r.reverse, r.count]), [[false, 1], [true, 2]]);
});

test("路线补全和切换不残留旧供应商", () => {
  assert.equal(liveFlowRoutes([entry("a", { state: "running", relayId: "" })])[0].relayId, "");
  assert.equal(liveFlowRoutes([entry("a", { state: "running", relayId: "new" })])[0].relayId, "new");
});

test("累计路程经过连线、hub 间隙和下一段，尾迹不从终点绕回", () => {
  assert.deepEqual(flowRoutePosition([30, 30, 60], 15), { segment: 0, progress: 0.5 });
  assert.deepEqual(flowRoutePosition([30, 30, 60], 45), { segment: 1, progress: 0.5 });
  assert.deepEqual(flowRoutePosition([30, 30, 60], 90), { segment: 2, progress: 0.5 });
  assert.equal(flowRoutePosition([30, 30, 60], -1), null);
  assert.equal(flowRoutePosition([30, 30, 60], 121), null);
});
