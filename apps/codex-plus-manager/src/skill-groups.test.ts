import assert from "node:assert/strict";
import test from "node:test";
import { moveSkillGroup, orderedSkillGroups, readSkillGroups } from "./skill-groups.ts";

test("loads existing groups without losing assignments and rejects malformed storage", () => {
  assert.deepEqual(readSkillGroups('{"names":["Work","Work"],"assignments":{"one":"Work","two":4}}'),
    { names: ["Work"], assignments: { one: "Work" }, order: [] });
  for (const value of ["null", "{", "[]"]) assert.deepEqual(readSkillGroups(value), { names: [], assignments: {}, order: [] });
});

test("retains empty custom groups, adds newly discovered sources, and removes obsolete names", () => {
  assert.deepEqual(orderedSkillGroups(["Local", "Empty", "New"], ["Missing", "Empty", "Local", "Empty"]), ["Empty", "Local", "New"]);
});

test("reorders against full inventory so filtered or other-tab groups are retained", () => {
  const order = ["Empty", "Hidden", "Local", "Repository"];
  assert.deepEqual(moveSkillGroup(order, "Local", "Empty"), ["Local", "Empty", "Hidden", "Repository"]);
  assert.deepEqual(moveSkillGroup(order, "Empty", "Repository"), ["Hidden", "Local", "Repository", "Empty"]);
  assert.deepEqual(moveSkillGroup(order, "Unknown", "Empty"), order);
});
