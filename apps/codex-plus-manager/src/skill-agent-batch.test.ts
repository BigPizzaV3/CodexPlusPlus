import assert from "node:assert/strict";
import test from "node:test";
import { mergeSkillAgentResult, skillBatchHasPending, skillBatchState, skillBatchTargets, SkillBatchRegistry } from "./skill-agent-batch.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const deferred = () => {
  let release!: () => void;
  const promise = new Promise<void>((resolve) => { release = resolve; });
  return { promise, release };
};
async function waitForUpdates(registry: SkillBatchRegistry, count = 0) {
  for (let attempt = 0; registry.getUpdates().length !== count && attempt < 50; attempt++) await tick();
  assert.equal(registry.getUpdates().length, count);
}

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
  await waitForUpdates(registry);
  assert.equal(reports.length, 1);
  assert.equal(reports[0].completed, 1);
  assert.equal(reports[0].failed, 4);
  assert.equal(reports[0].details.length, 3);
  assert.ok(reports[0].details.every((detail) => detail.length <= 240));
  assert.match(reports[0].details[0], /^one：/);
  assert.equal(registry.getFailures()?.count, 4);
  registry.enqueueUpdate("six", "update_skill", async () => registry.recordUpdateOutcome("six", { success: true, message: "ok" }, () => {}));
  await waitForUpdates(registry);
  assert.equal(registry.getFailures()?.count, 4);
  registry.clearFailures();
  assert.equal(registry.getFailures(), null);
  registry.enqueueUpdate("seven", "update_skill", async () => registry.recordUpdateOutcome("seven", { success: false, message: "later" }, () => {}));
  await waitForUpdates(registry);
  assert.deepEqual(registry.getFailures(), { count: 1, details: [{ repoKey: "", id: "seven", message: "later" }] });
  registry.enqueueUpdate("seven", "update_skill", async () => registry.recordUpdateOutcome("seven", { success: false, message: "new" }, () => {}));
  await waitForUpdates(registry);
  assert.deepEqual(registry.getFailures(), { count: 2, details: [{ repoKey: "", id: "seven", message: "new" }] });
});

test("failures distinguish complete repository and Skill IDs and replace only the matching retry", () => {
  const registry = new SkillBatchRegistry();
  const record = (repo: string, id: string, message: string) => registry.recordUpdateOutcome(id, { success: false, message }, () => {}, repo);
  record("host:a", "b", "first");
  record("host", "a:b", "second");
  record("other", "b", "third");
  record("host:a", "b", "latest");
  assert.deepEqual(registry.getFailures(), { count: 4, details: [
    { repoKey: "host:a", id: "b", message: "latest" },
    { repoKey: "host", id: "a:b", message: "second" },
    { repoKey: "other", id: "b", message: "third" },
  ] });
});

test("a slow update does not block another update or agent work", async () => {
  const registry = new SkillBatchRegistry();
  const calls: string[] = [];
  const blocked = deferred();
  assert.equal(registry.enqueueUpdate("slow", "update_skill", async () => { calls.push("slow"); await blocked.promise; }, "a/repo@main"), true);
  assert.equal(registry.enqueueUpdate("slow", "install_skill", async () => {}, "other/repo@main"), false);
  assert.equal(registry.reserveUninstall("slow"), false);
  assert.equal(registry.enqueueUpdate("same", "update_skill", async () => { calls.push("same"); }, "a/repo@main"), true);
  assert.equal(registry.enqueueUpdate("other", "install_skill", async () => { calls.push("other"); }, "b/repo@main"), true);
  const agent = registry.run(async () => { calls.push("agent"); });
  try {
    await tick();
    assert.deepEqual([...calls], ["slow", "same", "other", "agent"]);
    await waitForUpdates(registry, 1);
    assert.deepEqual(registry.getUpdates().map((item) => [item.id, item.state]), [["slow", "running"]]);
  } finally {
    blocked.release();
    await agent;
    await waitForUpdates(registry);
  }
  assert.equal(registry.reserveUninstall("slow"), true);
  registry.releaseUninstall("slow");
});

test("a slow agent operation does not block downloads", async () => {
  const registry = new SkillBatchRegistry();
  const calls: string[] = [];
  const blocked = deferred();
  const agent = registry.run(async () => { calls.push("agent"); await blocked.promise; });
  registry.enqueueUpdate("download", "update_skill", async () => { calls.push("download"); }, "a/repo@main");
  try {
    await tick();
    assert.deepEqual([...calls], ["agent", "download"]);
    await waitForUpdates(registry);
  } finally {
    blocked.release();
    await agent;
    await waitForUpdates(registry);
  }
});

test("updates respect total and per-repository limits across branches and subdirectories", async () => {
  const registry = new SkillBatchRegistry();
  const started: string[] = [];
  const gates = new Map<string, ReturnType<typeof deferred>>();
  let active = 0;
  let peak = 0;
  const activeByRepo = new Map<string, number>();
  const peakByRepo = new Map<string, number>();
  const enqueue = (id: string, repoKey: string, repo: string) => {
    const gate = deferred();
    gates.set(id, gate);
    assert.equal(registry.enqueueUpdate(id, "update_skill", async () => {
      started.push(id);
      active++;
      peak = Math.max(peak, active);
      const count = (activeByRepo.get(repo) ?? 0) + 1;
      activeByRepo.set(repo, count);
      peakByRepo.set(repo, Math.max(peakByRepo.get(repo) ?? 0, count));
      await gate.promise;
      active--;
      activeByRepo.set(repo, activeByRepo.get(repo)! - 1);
    }, repoKey), true);
  };
  enqueue("a1", "ACME/Kit@main:one", "a");
  enqueue("a2", "acme/kit@dev:two", "a");
  enqueue("a3", "acme/kit@main:three", "a");
  enqueue("a4", "acme/kit@main", "a");
  enqueue("b1", "other/repo@main", "b");
  enqueue("b2", "other/repo@main", "b");
  enqueue("b3", "other/repo@main", "b");
  enqueue("c1", "third/repo@main", "c");
  try {
    await tick();
    assert.deepEqual([...started], ["a1", "a2", "b1", "b2"]);
    assert.equal(registry.getUpdates().find((item) => item.id === "a3")?.state, "queued");
    assert.equal(registry.getUpdates().find((item) => item.id === "c1")?.state, "queued");
    assert.equal(registry.enqueueUpdate("a3", "install_skill", async () => {}, "third/repo@main"), false);
    assert.equal(registry.reserveUninstall("a3"), false);
    gates.get("b1")!.release();
    await tick();
    assert.deepEqual([...started], ["a1", "a2", "b1", "b2", "b3"]);
    gates.get("a1")!.release();
    await tick();
    assert.equal(started.at(-1), "a3");
    gates.get("b2")!.release();
    await tick();
    assert.equal(started.at(-1), "c1");
  } finally {
    for (const gate of gates.values()) gate.release();
    await waitForUpdates(registry);
  }
  assert.equal(peak, 4);
  assert.deepEqual([...peakByRepo], [["a", 2], ["b", 2], ["c", 1]]);
});

test("empty repository keys share a limit without blocking known repositories", async () => {
  const registry = new SkillBatchRegistry();
  const gate = deferred();
  const started: string[] = [];
  for (const [id, repo] of [["unknown1", ""], ["unknown2", "  "], ["unknown3", ""], ["known", "a/repo@main"]]) {
    registry.enqueueUpdate(id, "update_skill", async () => { started.push(id); await gate.promise; }, repo);
  }
  try {
    await tick();
    assert.deepEqual([...started], ["unknown1", "unknown2", "known"]);
    assert.equal(registry.getUpdates().find((item) => item.id === "unknown3")?.state, "queued");
  } finally {
    gate.release();
    await waitForUpdates(registry);
  }
  assert.deepEqual(started, ["unknown1", "unknown2", "known", "unknown3"]);
});

test("a rejected update releases its slot and reports the complete concurrent wave", async () => {
  const registry = new SkillBatchRegistry();
  const blocked = deferred();
  const calls: string[] = [];
  const reports: Array<{ completed: number; failed: number; details: string[] }> = [];
  const report = (summary: { completed: number; failed: number; details: string[] }) => reports.push(summary);
  registry.enqueueUpdate("slow", "update_skill", async () => {
    calls.push("slow");
    await blocked.promise;
    registry.recordUpdateOutcome("slow", { success: true, message: "ok" }, report);
  }, "a/repo@main");
  registry.enqueueUpdate("failed", "update_skill", async () => {
    calls.push("failed");
    registry.recordUpdateOutcome("failed", { success: false, message: "timeout" }, report);
    throw new Error("timeout");
  }, "a/repo@main");
  registry.enqueueUpdate("next", "update_skill", async () => {
    calls.push("next");
    registry.recordUpdateOutcome("next", { success: true, message: "ok" }, report);
  }, "a/repo@main");
  registry.enqueueUpdate("other", "install_skill", async () => {
    calls.push("other");
    registry.recordUpdateOutcome("other", { success: true, message: "ok" }, report);
  }, "b/repo@main");
  try {
    await tick();
    assert.deepEqual([...calls], ["slow", "failed", "other", "next"]);
    await waitForUpdates(registry, 1);
    assert.deepEqual(registry.getFailures(), { count: 1, details: [{ repoKey: "", id: "failed", message: "timeout" }] });
    assert.deepEqual(reports, []);
  } finally {
    blocked.release();
    await waitForUpdates(registry);
  }
  assert.deepEqual(reports, [{ completed: 3, failed: 1, details: ["failed：timeout"] }]);
});

test("a synchronous failure releases its slot and does not stop queued updates", async () => {
  const registry = new SkillBatchRegistry();
  const gate = deferred();
  const calls: string[] = [];
  registry.enqueueUpdate("blocked", "update_skill", async () => { await gate.promise; }, "a/repo@main");
  registry.enqueueUpdate("failed", "update_skill", () => { calls.push("failed"); throw new Error("failed"); }, "a/repo@main");
  registry.enqueueUpdate("next", "update_skill", async () => { calls.push("next"); }, "a/repo@main");
  try {
    await tick();
    assert.deepEqual([...calls], ["failed", "next"]);
  } finally {
    gate.release();
    await waitForUpdates(registry);
  }
});

test("an unrelated uninstall reservation does not occupy a download slot or delay its summary", async () => {
  const registry = new SkillBatchRegistry();
  const reports: Array<{ completed: number; failed: number; details: string[] }> = [];
  assert.equal(registry.reserveUninstall("removing"), true);
  assert.equal(registry.enqueueUpdate("removing", "update_skill", async () => {}, "a/repo@main"), false);
  assert.equal(registry.enqueueUpdate("download", "update_skill", async () => {
    registry.recordUpdateOutcome("download", { success: true, message: "ok" }, (summary) => reports.push(summary));
  }, "a/repo@main"), true);
  await waitForUpdates(registry, 1);
  assert.deepEqual(registry.getUpdates().map(({ id, command }) => [id, command]), [["removing", "uninstall_skill"]]);
  assert.deepEqual(reports, [{ completed: 1, failed: 0, details: [] }]);
  registry.releaseUninstall("removing");
  assert.equal(registry.getUpdates().length, 0);
});

test("updates continue after the last observer unsubscribes", async () => {
  const registry = new SkillBatchRegistry();
  const gate = deferred();
  const calls: string[] = [];
  const unsubscribe = registry.subscribeUpdates(() => calls.push("observed"));
  registry.enqueueUpdate("a", "update_skill", async () => { await gate.promise; calls.push("completed"); }, "a/repo@main");
  unsubscribe();
  gate.release();
  await waitForUpdates(registry);
  assert.equal(calls.at(-1), "completed");
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
