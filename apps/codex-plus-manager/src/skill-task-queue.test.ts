import assert from "node:assert/strict";
import test from "node:test";
import { checkRepositories, SkillTaskQueue } from "./skill-task-queue.ts";

test("updates queue independently, rejects duplicates and continues after errors", async () => {
  const queue = new SkillTaskQueue<string>();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const calls: string[] = [];
  const events: string[] = [];
  const unsubscribe = queue.subscribe(() => events.push(queue.getSnapshot().map((item) => `${item.key}:${item.state}`).join(",")));
  assert.equal(queue.enqueue("a", async () => { calls.push("a"); await gate; return "first"; }), true);
  assert.equal(queue.enqueue("b", async () => { calls.push("b"); throw new Error("failed"); }), true);
  assert.equal(queue.enqueue("c", async () => { calls.push("c"); return "third"; }), true);
  assert.equal(queue.enqueue("a", async () => "duplicate"), false);
  assert.deepEqual(calls, ["a"]);
  assert.equal(queue.getSnapshot().find((item) => item.key === "b")?.state, "queued");
  unsubscribe();
  release();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls, ["a", "b", "c"]);
  assert.deepEqual(queue.getSnapshot().map((item) => item.state), ["done", "failed", "done"]);
  assert.ok(events.some((event) => event.includes("b:queued")));
  assert.equal(queue.enqueue("b", async () => "retry"), true);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(queue.getSnapshot().find((item) => item.key === "b")?.result, "retry");
});

test("a slow repository does not hold back completed repository results", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const completed: string[] = [];
  const checks = checkRepositories(["slow", "fast", "next"], 2, async (item) => {
    if (item === "slow") await gate;
    completed.push(item);
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(completed, ["fast", "next"]);
  release();
  await checks;
  assert.deepEqual(completed, ["fast", "next", "slow"]);
});
