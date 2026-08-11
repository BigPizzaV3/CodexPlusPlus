import assert from "node:assert/strict";
import test from "node:test";

import {
  normalizeWorkflowConditionBranches,
  workflowConditionEdges,
} from "../shared/workflow-sequence.mjs";
import {
  migrateWorkflowSnapshotV1,
} from "../shared/workflow-control-flow.mjs";

const node = (id, kind, x = 0, y = 0) => ({
  id,
  position: { x, y },
  data: { kind },
});

test("legacy condition edges migrate to two branches followed by one shared continuation", () => {
  const nodes = [
    node("trigger", "issue-trigger"),
    node("condition", "condition", 0, 100),
    node("true-1", "skill", -180, 240),
    node("true-2", "mcp", -180, 380),
    node("false-1", "issue-update", 180, 240),
    node("shared-tail", "codex-review", 0, 560),
  ];
  const edges = workflowConditionEdges(
    ["trigger", "condition"],
    "condition",
    { true: ["true-1", "true-2"], false: ["false-1"] },
  );
  edges.push({
    id: "legacy-shared-tail",
    source: "condition",
    target: "shared-tail",
  });
  const migrated = migrateWorkflowSnapshotV1(nodes, edges);

  assert.deepEqual(migrated.flow.root.items.map((item) => item.nodeId), [
    "trigger",
    "condition",
    "shared-tail",
  ]);
  assert.deepEqual(
    migrated.flow.root.items[1].branches.true.items.map((item) => item.nodeId),
    ["true-1", "true-2"],
  );
  assert.deepEqual(
    migrated.flow.root.items[1].branches.false.items.map((item) => item.nodeId),
    ["false-1"],
  );
});

test("legacy linear steps after a condition migrate to the true path in order", () => {
  const nodes = [
    node("trigger", "issue-trigger"),
    node("condition", "condition", 0, 100),
    node("first", "skill", 0, 240),
    node("second", "issue-update", 0, 380),
  ];
  const graph = normalizeWorkflowConditionBranches(nodes, [
    { id: "a", source: "trigger", target: "condition" },
    { id: "b", source: "condition", target: "first" },
    { id: "c", source: "first", target: "second" },
  ]);

  assert.deepEqual(graph.trunkStepIds, ["trigger", "condition"]);
  assert.deepEqual(graph.branches.true, ["first", "second"]);
  assert.deepEqual(graph.branches.false, []);
  assert.equal(graph.migrated, true);
});

test("empty condition paths remain distinct and adding to one path cannot reorder the other", () => {
  const edges = workflowConditionEdges(
    ["trigger", "condition"],
    "condition",
    { true: ["true-1", "true-2"], false: ["false-1", "false-2"] },
  );
  assert.deepEqual(
    edges
      .filter((edge) => edge.data?.conditionOutcome === "true")
      .map((edge) => [edge.source, edge.target]),
    [["condition", "true-1"], ["true-1", "true-2"]],
  );
  assert.deepEqual(
    edges
      .filter((edge) => edge.data?.conditionOutcome === "false")
      .map((edge) => [edge.source, edge.target]),
    [["condition", "false-1"], ["false-1", "false-2"]],
  );
});

test("condition branch membership and order survive workspace JSON persistence", () => {
  const nodes = [
    node("trigger", "issue-trigger"),
    node("condition", "condition", 0, 100),
    node("accepted", "skill", -180, 240),
    node("rejected", "issue-update", 180, 240),
  ];
  const persisted = JSON.parse(JSON.stringify({
    nodes,
    edges: workflowConditionEdges(
      ["trigger", "condition"],
      "condition",
      { true: ["accepted"], false: ["rejected"] },
    ),
  }));
  const graph = normalizeWorkflowConditionBranches(persisted.nodes, persisted.edges);

  assert.deepEqual(graph.trunkStepIds, ["trigger", "condition"]);
  assert.deepEqual(graph.branches, {
    true: ["accepted"],
    false: ["rejected"],
  });
});
