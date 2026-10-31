import assert from "node:assert/strict";
import test from "node:test";
import { mergeSkillAgentResult, skillBatchHasPending, skillBatchState, skillBatchTargets } from "./skill-agent-batch.ts";

test("batch checkboxes and targets are independent for each agent", () => {
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
});

test("legacy entries without agents use the Codex enabled field only", () => {
  const items = [{ installed: true, bundled: false, enabled: true }];
  assert.equal(skillBatchState(items, "codex").checked, true);
  assert.equal(skillBatchState(items, "claude").checked, false);
});

test("pending work locks only its target skills for the same agent", () => {
  const operations = [{ groupKey: "repo:one", agent: "codex", enabled: true, skillIds: ["a", "b"] }];
  assert.equal(skillBatchHasPending(operations, [{ id: "a" }], "codex"), true);
  assert.equal(skillBatchHasPending(operations, [{ id: "other" }], "codex"), false);
  assert.equal(skillBatchHasPending(operations, [{ id: "a" }], "claude"), false);
  assert.equal(skillBatchHasPending(operations, [{ id: "other" }, { id: "b" }], "codex"), true);
  assert.equal(skillBatchHasPending([], [{ id: "a" }], "codex"), false);
});

test("a stale batch response changes only the target agent on the target installed skill", () => {
  const current = [
    { id: "a", installed: true, bundled: false, enabled: false, agents: ["claude"], name: "current" },
    { id: "b", installed: true, bundled: false, enabled: true, agents: ["codex"], name: "other" },
    { id: "a", installed: false, bundled: false, enabled: false, agents: [], name: "remote" },
  ];
  const stale = [
    { ...current[0], enabled: true, agents: ["codex"], name: "old" },
    { ...current[1], enabled: false, agents: [], name: "old other" },
  ];
  const merged = mergeSkillAgentResult(current, stale, "a", "codex");
  assert.deepEqual(merged[0], { ...current[0], enabled: true, agents: ["claude", "codex"] });
  assert.equal(merged[1], current[1]);
  assert.equal(merged[2], current[2]);
  assert.equal(mergeSkillAgentResult(current, [], "a", "codex"), current);
});

test("failed or legacy agent results do not alter unrelated agent flags", () => {
  const current = [{ id: "a", installed: true, bundled: false, enabled: true, agents: ["codex", "claude"] }];
  const failed = [{ ...current[0], agents: ["codex"], enabled: false }];
  assert.deepEqual(mergeSkillAgentResult(current, failed, "a", "claude")[0],
    { ...current[0], agents: ["codex"] });
  const legacy = [{ id: "a", installed: true, bundled: false, enabled: false }];
  assert.deepEqual(mergeSkillAgentResult(current, legacy, "a", "codex")[0],
    { ...current[0], enabled: false, agents: ["claude"] });
});
