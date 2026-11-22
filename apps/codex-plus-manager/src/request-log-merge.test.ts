import assert from "node:assert/strict";
import test from "node:test";
import { mergeRequestLogDelta, reuseRequestLogEntries } from "./request-log-merge.ts";

type Entry = { id: string; startedAtMs: number; timestampMs?: number; state?: string | null; note?: string };

const entry = (id: string, startedAtMs: number, extra: Partial<Entry> = {}): Entry => ({ id, startedAtMs, state: "success", ...extra });

test("没有新记录、进行中的也没变：原样返回旧数组，不触发重渲染", () => {
  const running = entry("r", 300, { state: "running", note: "phase-1" });
  const previous = [running, entry("b", 200), entry("a", 100)];
  const result = mergeRequestLogDelta(previous, [{ ...running }], 300, null);
  assert.equal(result.entries, previous);
  assert.equal(result.truncated, false);
});

test("新完成的记录替换同 id 的进行中记录，已消失的进行中记录被去掉，未变的沿用旧对象", () => {
  const a = entry("a", 100);
  const previous = [entry("gone", 400, { state: "running" }), entry("r", 300, { state: "running" }), a];
  const finished = entry("r", 300, { state: "success" });
  const fresh = entry("c", 500, { state: "running" });
  const result = mergeRequestLogDelta(previous, [fresh, finished], 300, null);
  assert.deepEqual(result.entries.map((item) => item.id), ["c", "r", "a"]);
  assert.equal(result.entries[1].state, "success");
  assert.equal(result.entries[2], a, "没变的记录保持同一个对象");
});

test("开始得早、结束得晚的请求按开始时间插到中间；超过上限截掉并标记", () => {
  const previous = [entry("c", 300), entry("b", 200), entry("a", 100)];
  const result = mergeRequestLogDelta(previous, [entry("late", 250), entry("d", 400)], 4, null);
  assert.deepEqual(result.entries.map((item) => item.id), ["d", "c", "late", "b"]);
  assert.equal(result.truncated, true);
});

test("滚动时间段：落到时间段以外的旧记录被去掉", () => {
  const previous = [entry("b", 200), entry("a", 100)];
  const result = mergeRequestLogDelta(previous, [entry("c", 300)], 300, 150);
  assert.deepEqual(result.entries.map((item) => item.id), ["c", "b"]);
});

test("整页重读：内容没变时沿用旧数组和旧对象", () => {
  const previous = [entry("b", 200), entry("a", 100)];
  assert.equal(reuseRequestLogEntries(previous, previous.map((item) => ({ ...item }))), previous);
  const next = reuseRequestLogEntries(previous, [entry("c", 300), { ...previous[0] }, { ...previous[1] }]);
  assert.notEqual(next, previous);
  assert.equal(next[1], previous[0]);
  assert.equal(next[2], previous[1]);
});
