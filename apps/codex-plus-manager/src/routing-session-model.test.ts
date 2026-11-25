import assert from "node:assert/strict";
import test from "node:test";
import type { FlowEntry } from "./request-flow-model.ts";
import { contextSeries, groupRoutingRequests, isContextCompaction, requestContext, routingSessionKey, selectRoutingRequest } from "./routing-session-model.ts";

const request = (id: string, time: number, extra: Partial<FlowEntry> = {}): FlowEntry => ({
  id, startedAtMs: time, timestampMs: time + 1, model: "m", durationMs: 1, success: true, ...extra,
});
const route: NonNullable<FlowEntry["route"]> = { requestedModel: "m", model: "m", result: "answered", contextWindow: 5_000,
  elapsedMs: 1, finishedAtMs: 1, steps: [], planned: [] };

test("会话按 Agent 和明确会话 ID 分组，未关联请求不合并", () => {
  const entries = [
    request("old", 1, { agent: "codex", threadId: "A", threadTitle: "旧记录仍可提供标题" }),
    request("new", 6, { agent: "codex", threadId: "A" }),
    request("other-agent", 5, { agent: "claude", threadId: "A" }),
    request("missing-a", 2),
    request("missing-b", 3, { threadId: " " }),
  ];
  const grouped = groupRoutingRequests(entries);
  assert.deepEqual(grouped.map(group => group.requests.map(entry => entry.id)), [["new", "old"], ["other-agent"], ["missing-b"], ["missing-a"]]);
  assert.equal(grouped[0].title, "旧记录仍可提供标题");
});

test("上限和缓存字段缺失不补零，明确回传的零仍有效", () => {
  const usage = { inputTokens: 1_000, outputTokens: 10, cachedTokens: 0 };
  assert.deepEqual(requestContext(request("unknown", 1, { usage })), { input: 1000, window: null, cached: null, ratio: null });
  assert.deepEqual(requestContext(request("zero", 1, { route, usage: { ...usage, cachedTokensKnown: true } })), {
    input: 1000, window: 5000, cached: 0, ratio: .2,
  });
  assert.equal(requestContext(request("bad", 1, { route, usage: { ...usage, cachedTokens: 1001 } })).cached, null);
});

test("上下文序列按开始时间，失败、压缩、进行中不是已测占用点", () => {
  const extra = { route, usage: { inputTokens: 1000, outputTokens: 0 } };
  const compact = request("compact", 2, { ...extra, path: "/v1/responses/compact?test=1" });
  assert.ok(isContextCompaction(compact));
  const series = contextSeries([request("last", 4, { ...extra, state: "running" }), compact,
    request("first", 1, extra), request("failed", 3, { ...extra, success: false })]);
  assert.deepEqual(series.map(({ entry, ratio }) => [entry.id, ratio]), [
    ["first", .2], ["compact", null], ["failed", null], ["last", null],
  ]);
});

test("锁定历史请求不受新会话刷新影响，逐条模式不选被过滤的请求", () => {
  const old = request("old", 1, { threadId: "A" });
  const newer = request("newer", 2, { threadId: "A" });
  const latest = request("latest", 3, { threadId: "B" });
  const groups = groupRoutingRequests([old, newer, latest]).map(group => ({ group, shows: group.requests.slice(0, 1) }));
  const pinned = { request: old.id, session: routingSessionKey(old) };
  assert.equal(selectRoutingRequest(groups, latest, pinned, true).entry, old);
  assert.equal(selectRoutingRequest(groups, latest, pinned, true).pinned, true);
  assert.equal(selectRoutingRequest(groups, latest, pinned, false).entry, latest);
  assert.equal(selectRoutingRequest(groups, latest, undefined, true).pinned, false);
});
