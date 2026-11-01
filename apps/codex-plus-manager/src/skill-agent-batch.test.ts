import assert from "node:assert/strict";
import test from "node:test";
import { mergeSkillAgentResult, skillBatchHasPending, skillBatchState, skillBatchTargets, SkillBatchRegistry } from "./skill-agent-batch.ts";

test("pending uninstall reserves its ID across observers until released", () => {
  const registry = new SkillBatchRegistry();
  assert.equal(registry.reserveUninstall("alpha"), true);
  assert.equal(registry.reserveUninstall("alpha"), false);
  assert.equal(registry.enqueueUpdate("alpha", "update_skill", async () => {}), false);
  assert.equal(registry.getUpdates()[0].command, "uninstall_skill");
  registry.releaseUninstall("alpha");
  assert.equal(registry.getUpdates().length, 0);
  assert.equal(registry.reserveUninstall("alpha"), true);
  registry.releaseUninstall("alpha");
});

test("one queue wave aggregates bounded failure details and exact totals", async () => {
  const registry = new SkillBatchRegistry();
  const reports: Array<{ completed: number; failed: number; details: string[] }> = [];
  for (const id of ["one", "two", "three", "four", "five"]) {
    registry.enqueueUpdate(id, id === "five" ? "install_skill" : "update_skill", async () => {
      registry.recordUpdateOutcome(id, { success: id === "five", message: "x".repeat(500) }, (result) => reports.push(result));
    });
  }
  await registry.run(async () => {});
  assert.equal(reports.length, 1);
  assert.equal(reports[0].completed, 1);
  assert.equal(reports[0].failed, 4);
  assert.equal(reports[0].details.length, 3);
  assert.ok(reports[0].details.every((detail) => detail.length <= 240));
  assert.match(reports[0].details[0], /^one：/);
  assert.equal(registry.getFailures()?.count, 4);
  registry.enqueueUpdate("six", "update_skill", async () => registry.recordUpdateOutcome("six", { success: true, message: "ok" }, () => {}));
  await registry.run(async () => {});
  assert.equal(registry.getFailures()?.count, 4);
  registry.clearFailures();
  assert.equal(registry.getFailures(), null);
  registry.enqueueUpdate("seven", "update_skill", async () => registry.recordUpdateOutcome("seven", { success: false, message: "later" }, () => {}));
  await registry.run(async () => {});
  assert.deepEqual(registry.getFailures(), { count: 1, details: [{ id: "seven", message: "later" }] });
  registry.enqueueUpdate("seven", "update_skill", async () => registry.recordUpdateOutcome("seven", { success: false, message: "new" }, () => {}));
  await registry.run(async () => {});
  assert.deepEqual(registry.getFailures(), { count: 2, details: [{ id: "seven", message: "new" }] });
});

test("updates queue with agent work, reject duplicates and continue after failure", async () => {
  const registry = new SkillBatchRegistry();
  const calls: string[] = [];
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => { release = resolve; });
  const agent = registry.run(async () => { calls.push("agent"); await blocked; });
  assert.equal(registry.enqueueUpdate("one", "update_skill", async () => { calls.push("one"); throw new Error("failed"); }), true);
  assert.equal(registry.enqueueUpdate("one", "install_skill", async () => {}), false);
  assert.equal(registry.enqueueUpdate("two", "install_skill", async () => { calls.push("two"); }), true);
  await Promise.resolve();
  assert.deepEqual(calls, ["agent"]);
  assert.deepEqual(registry.getUpdates().map((item) => item.state), ["queued", "queued"]);
  release();
  await agent;
  await registry.run(async () => {});
  assert.deepEqual(calls, ["agent", "one", "two"]);
  assert.equal(registry.getUpdates().length, 0);
});

test("batch state and targets only include installed mutable Skills for the requested Agent", () => {
  const items = [
    { id: "a", installed: true, bundled: false, enabled: true, agents: ["codex", "claude"] },
    { id: "b", installed: true, bundled: false, enabled: false, agents: ["grok"] },
    { id: "builtin", installed: true, bundled: true, enabled: true, agents: ["codex"] },
    { id: "remote", installed: false, bundled: false, enabled: false, agents: [] },
  ];
  assert.deepEqual(skillBatchState(items, "codex"), { total: 2, checked: false, mixed: true });
  assert.deepEqual(skillBatchTargets(items, "codex", true).map((item) => item.id), ["b"]);
  assert.deepEqual(skillBatchTargets(items, "claude", false).map((item) => item.id), ["a"]);
  assert.deepEqual(skillBatchTargets(items, "grok", true).map((item) => item.id), ["a"]);
  assert.deepEqual(skillBatchState([items[2]], "codex"), { total: 0, checked: false, mixed: false });
  assert.equal(skillBatchState([{ installed: true, bundled: false, enabled: true }], "codex").checked, true);
  assert.equal(skillBatchState([{ installed: true, bundled: false, enabled: true }], "claude").checked, false);
});

test("pending operations remain visible and only lock matching Skills and Agents", () => {
  const registry = new SkillBatchRegistry();
  const first = { groupKey: "one", agent: "codex", enabled: true, skillIds: ["a"], settledSkillIds: [] as string[], owner: Symbol("first") };
  const second = { groupKey: "one", agent: "claude", enabled: true, skillIds: ["a"], settledSkillIds: [] as string[], owner: Symbol("second") };
  const settled: symbol[] = [];
  const unsubscribe = registry.subscribeSettled((owner) => settled.push(owner));
  registry.add(first);
  registry.add(second);
  assert.equal(skillBatchHasPending(registry.getSnapshot(), [{ id: "a" }], "codex"), true);
  assert.equal(skillBatchHasPending(registry.getSnapshot(), [{ id: "b" }], "codex"), false);
  const done = { ...first, settledSkillIds: ["a"] };
  registry.replace(first, done);
  registry.remove(done);
  assert.equal(skillBatchHasPending(registry.getSnapshot(), [{ id: "a" }], "codex"), false);
  assert.equal(skillBatchHasPending(registry.getSnapshot(), [{ id: "a" }], "claude"), true);
  assert.deepEqual(settled, [first.owner]);
  unsubscribe();
});

test("batch responses do not replace other Agents or unrelated Skills", () => {
  const current = [
    { id: "a", installed: true, bundled: false, enabled: false, agents: ["claude"], name: "current" },
    { id: "b", installed: true, bundled: false, enabled: true, agents: ["codex"], name: "other" },
    { id: "a", installed: false, bundled: false, enabled: false, agents: [], name: "remote" },
  ];
  const old = [{ ...current[0], enabled: true, agents: ["codex"], name: "old" }, { ...current[1], enabled: false, agents: [] }];
  const merged = mergeSkillAgentResult(current, old, "a", "codex");
  assert.deepEqual(merged[0], { ...current[0], enabled: true, agents: ["claude", "codex"] });
  assert.equal(merged[1], current[1]);
  assert.equal(merged[2], current[2]);
  assert.equal(mergeSkillAgentResult(current, [], "a", "codex"), current);
});

test("mutation queue serializes concurrent requests and continues after rejection", async () => {
  const registry = new SkillBatchRegistry();
  const events: string[] = [];
  let release!: () => void;
  const first = registry.run(async () => { events.push("first start"); await new Promise<void>((resolve) => { release = resolve; }); events.push("first end"); throw new Error("failure"); });
  const second = registry.run(async () => { events.push("second"); return 2; });
  await Promise.resolve();
  assert.deepEqual(events, ["first start"]);
  release();
  await assert.rejects(first, /failure/);
  assert.equal(await second, 2);
  assert.deepEqual(events, ["first start", "first end", "second"]);
});
