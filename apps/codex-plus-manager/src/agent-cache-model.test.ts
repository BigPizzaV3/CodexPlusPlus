import assert from "node:assert/strict";
import { test } from "node:test";
import { cacheApps, cacheCleanupSummary, cacheSelection, type CacheGroup } from "./agent-cache-model.ts";

const group = (id: string, patch: Partial<CacheGroup> = {}): CacheGroup => ({
  id, app: "Codex", kind: "cache", path: `/test/${id}`, totalBytes: 100,
  eligibleBytes: 60, eligibleFiles: 2, skippedFiles: 1, cleanable: true, ...patch,
});

test("recommended excludes logs while all includes only eligible cleanable groups", () => {
  const groups = [group("cache"), group("logs", { kind: "logs" }),
    group("update", { kind: "update" }), group("temporary", { kind: "temporary", cleanable: false }),
    group("recent", { eligibleFiles: 0, eligibleBytes: 0 })];
  assert.deepEqual(cacheSelection(groups, "recommended"), ["cache", "update"]);
  assert.deepEqual(cacheSelection(groups, "all"), ["cache", "logs", "update"]);
  assert.deepEqual(cacheSelection([], "all"), []);
});

test("app cards aggregate total and cleanable sizes separately, including statistics-only files", () => {
  const groups = [group("cache"), group("temporary", { totalBytes: 200, eligibleBytes: 0, eligibleFiles: 0, cleanable: false }),
    group("claude", { app: "Claude", totalBytes: 300, eligibleBytes: 120 })];
  const apps = cacheApps(groups);
  assert.deepEqual(apps.map((app) => app.name), ["Codex", "Claude", "Codex++"]);
  assert.equal(apps[0].totalBytes, 300);
  assert.equal(apps[0].eligibleBytes, 60);
  assert.equal(apps[0].groups.length, 2);
  assert.equal(apps[1].totalBytes, 300);
  assert.equal(apps[2].totalBytes, 0);
  assert.deepEqual(apps[2].groups, []);
});

test("cleanup summary stays compact with many directories and excludes ineligible files", () => {
  const groups = Array.from({ length: 100 }, (_, index) => group(`directory-${index}`, {
    app: index % 2 ? "Claude" : "Codex",
  }));
  groups.push(group("plus", { app: "Codex++" }),
    group("statistics", { app: "Other", cleanable: false }),
    group("recent", { app: "Other", eligibleFiles: 0, eligibleBytes: 0 }));
  assert.deepEqual(cacheCleanupSummary(groups), {
    apps: ["Codex", "Claude", "Codex++"], files: 202, bytes: 6060,
  });
  assert.deepEqual(cacheCleanupSummary([]), { apps: [], files: 0, bytes: 0 });
});
