import assert from "node:assert/strict";
import test from "node:test";
import { mergeSkillAgentResult, skillBatchHasPending, skillBatchState, skillBatchTargets, SkillBatchRegistry } from "./skill-agent-batch.ts";

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
