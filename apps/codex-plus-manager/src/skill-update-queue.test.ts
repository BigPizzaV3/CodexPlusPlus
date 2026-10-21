import assert from "node:assert/strict";
import test from "node:test";
import { createSkillUpdateQueue } from "./skill-update-queue.ts";

const candidate = (id: string, repoKey = "acme/one@main") => ({ id, repoKey, name: id });
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
test("repositories can join a running queue, with observable progress and no concurrent writes or duplicates", async () => {
  const pending: { id: string; resolve: (result: { status: string; message: string }) => void }[] = [];
  const queue = createSkillUpdateQueue((skill) => new Promise<{ status: string; message: string }>((resolve) => pending.push({ id: skill.id, resolve })));
  let notifications = 0;
  const unsubscribe = queue.subscribe(() => notifications++);
  queue.enqueue("one", [candidate("alpha"), candidate("beta")]);
  queue.enqueue("two", [candidate("gamma", "acme/two@main")]);
  queue.enqueue("duplicate", [candidate("alpha"), candidate("beta")]);
  assert.deepEqual(queue.getSnapshot().tasks.map((task) => task.state), ["running", "queued", "queued"]);
  assert.equal(pending.length, 1);
  pending[0].resolve({ status: "ok", message: "alpha done" });
  await tick();
  assert.deepEqual(queue.getSnapshot().tasks.map((task) => task.state), ["succeeded", "running", "queued"]);
  assert.equal(pending[1].id, "beta");
  pending[1].resolve({ status: "ok", message: "beta done" });
  await tick();
  assert.equal(pending[2].id, "gamma");
  pending[2].resolve({ status: "ok", message: "gamma done" });
  await tick();
  assert.equal(queue.getSnapshot().active, false);
  assert(queue.getSnapshot().tasks.every((task) => task.state === "succeeded"));
  assert.equal(queue.getSnapshot().result?.message, "gamma done");
  assert(notifications >= 7);
  unsubscribe();
});

test("returned errors and rejected commands remain visible; subsequent tasks and retries succeed", async () => {
  let retry = false;
  const queue = createSkillUpdateQueue(async (skill) => {
    if (!retry && skill.id === "rejected") throw new Error("connection lost");
    return { status: !retry && skill.id === "failed" ? "failed" : "ok", message: skill.id };
  });
  queue.enqueue("one", [candidate("failed"), candidate("rejected"), candidate("last")]);
  await tick();
  assert.deepEqual(queue.getSnapshot().tasks.map((task) => task.state), ["failed", "failed", "succeeded"]);
  assert.equal(queue.getSnapshot().tasks[0].error, "failed");
  assert.match(queue.getSnapshot().tasks[1].error, /connection lost/);
  retry = true;
  queue.enqueue("one", [candidate("failed"), candidate("rejected")]);
  await tick();
  assert.equal(queue.getSnapshot().tasks.length, 3);
  assert(queue.getSnapshot().tasks.every((task) => task.state === "succeeded"));
});

test("queue survives listener removal and distinguishes the same skill id in different repositories", async () => {
  const calls: string[] = [];
  const queue = createSkillUpdateQueue(async (skill) => {
    calls.push(skill.repoKey);
    return { status: "ok", message: "" };
  });
  const off = queue.subscribe(() => {});
  queue.enqueue("one", [candidate("same"), candidate("same", "acme/two@main")]);
  off();
  await tick();
  assert.deepEqual(calls, ["acme/one@main", "acme/two@main"]);
  assert.equal(queue.getSnapshot().active, false);
  assert.equal(queue.getSnapshot().tasks.length, 2);
});
