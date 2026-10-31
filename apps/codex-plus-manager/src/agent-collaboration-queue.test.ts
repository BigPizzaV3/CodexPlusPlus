import assert from "node:assert/strict";
import test from "node:test";
import { agentQueueState, DEFAULT_MAX_QUEUED_PER_AGENT, normalizeConcurrency } from "./agent-collaboration-queue.ts";

const task = (agentId: string, status: string) => ({ agentId, status });

test("同一 Agent 繁忙时可以继续提交，只统计该 Agent 未结束的任务", () => {
  const state = agentQueueState([
    task("deepseek", "running"),
    task("deepseek", "queued"),
    task("deepseek", "completed"),
    task("claude", "running"),
  ], "deepseek", 8);
  assert.deepEqual(state, { active: 2, running: 1, queued: 1, limit: 8, concurrency: 1, willQueue: true, full: false });
  assert.deepEqual(agentQueueState([], "codex", 8), { active: 0, running: 0, queued: 0, limit: 8, concurrency: 1, willQueue: false, full: false });
});

test("排队数达到上限时表单应禁止提交，取消或结束的任务不占位", () => {
  const tasks = [task("codex", "running"), ...Array.from({ length: 8 }, () => task("codex", "queued"))];
  assert.equal(agentQueueState(tasks, "codex", 8).full, true);
  tasks[1] = task("codex", "cancelled");
  assert.equal(agentQueueState(tasks, "codex", 8).full, false);
});

test("旧版快照没有上限字段时使用默认值", () => {
  assert.equal(agentQueueState([], "codex").limit, DEFAULT_MAX_QUEUED_PER_AGENT);
  assert.equal(agentQueueState([], "codex", 0).limit, DEFAULT_MAX_QUEUED_PER_AGENT);
  assert.equal(agentQueueState([], "codex").concurrency, 1);
});

test("并发上限内新任务直接启动，达到上限或前面有排队任务时才排队（#2800）", () => {
  const running = [task("codex", "running")];
  assert.equal(agentQueueState(running, "codex", 8, 2).willQueue, false);
  assert.equal(agentQueueState(running, "codex", 8, 1).willQueue, true);
  const full = [task("codex", "running"), task("codex", "running")];
  assert.equal(agentQueueState(full, "codex", 8, 2).willQueue, true);
  assert.equal(agentQueueState([task("codex", "queued")], "codex", 8, 2).willQueue, true);
});

test("并发上限缺省或无效时按 1，超过范围时钳到上限", () => {
  assert.equal(normalizeConcurrency(undefined), 1);
  assert.equal(normalizeConcurrency(0), 1);
  assert.equal(normalizeConcurrency(2.5), 1);
  assert.equal(normalizeConcurrency(3), 3);
  assert.equal(normalizeConcurrency(9), 4);
  assert.equal(normalizeConcurrency(3, 2), 2);
});
