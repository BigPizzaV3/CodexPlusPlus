import assert from "node:assert/strict";
import test from "node:test";
import { updatableSkills } from "./skill-updates.ts";

test("batch updates only installed repository skills with safe comparison results", () => {
  const base = { id: "ready", installed: true, bundled: false, repoKey: "acme/kit@main", updateAvailable: true };
  const candidates = [
    base,
    { ...base, id: "uninstalled", installed: false },
    { ...base, id: "bundled", bundled: true },
    { ...base, id: "plugin", plugin: true },
    { ...base, id: "local", repoKey: "" },
    { ...base, id: "imported", repoKey: "cc-switch" },
    { ...base, id: "current", updateAvailable: false },
    { ...base, id: "modified", localDifference: true },
    { ...base, id: "unknown", comparisonError: "Cannot read local files" },
    { ...base, id: "disabled", enabled: false },
  ];
  assert.deepEqual(updatableSkills(candidates).map((skill) => skill.id), ["ready", "disabled"]);
  assert.deepEqual(updatableSkills([]), []);
  assert.equal(candidates.length, 10);
});
