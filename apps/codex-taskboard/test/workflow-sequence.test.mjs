import assert from "node:assert/strict";
import test from "node:test";
import {
  insertWorkflowStep,
  layoutWorkflowSteps,
  orderedWorkflowStepIds,
  reorderWorkflowStep,
  workflowSequenceEdges,
} from "../shared/workflow-sequence.mjs";

const nodes = [
  { id: "review", position: { x: 650, y: 220 } },
  { id: "plan-child", parentId: "plan", position: { x: 10, y: 86 } },
  { id: "trigger", position: { x: 20, y: 220 } },
  { id: "update", position: { x: 950, y: 220 } },
  { id: "plan", position: { x: 330, y: 153 } },
];

const edges = [
  { id: "trigger-plan", source: "trigger", target: "plan" },
  { id: "plan-review", source: "plan", target: "review" },
  { id: "review-update", source: "review", target: "update" },
];

test("workflow roots are converted from a free canvas graph into one execution sequence", () => {
  assert.deepEqual(
    orderedWorkflowStepIds(nodes, edges),
    ["trigger", "plan", "review", "update"],
  );
});

test("disconnected workflow roots are appended deterministically without including nested plan items", () => {
  const withDisconnected = [
    ...nodes,
    { id: "later", position: { x: 200, y: 800 } },
    { id: "earlier", position: { x: 100, y: 600 } },
  ];
  assert.deepEqual(
    orderedWorkflowStepIds(withDisconnected, edges),
    ["trigger", "plan", "review", "update", "earlier", "later"],
  );
});

test("steps insert and reorder within the sequence while the trigger stays pinned first", () => {
  const initial = ["trigger", "plan", "review"];
  assert.deepEqual(
    insertWorkflowStep(initial, "skill", "plan"),
    ["trigger", "plan", "skill", "review"],
  );
  assert.deepEqual(
    insertWorkflowStep([], "trigger", null),
    ["trigger"],
  );
  assert.deepEqual(
    reorderWorkflowStep(initial, "review", 1, "trigger"),
    ["trigger", "review", "plan"],
  );
  assert.deepEqual(
    reorderWorkflowStep(initial, "trigger", 2, "trigger"),
    initial,
  );
});

test("the execution sequence owns its connectors and vertical layout", () => {
  assert.deepEqual(workflowSequenceEdges(["trigger", "plan", "review"]), [
    { id: "sequence-trigger-plan", source: "trigger", target: "plan" },
    { id: "sequence-plan-review", source: "plan", target: "review" },
  ]);

  const laidOut = layoutWorkflowSteps(
    nodes,
    ["trigger", "plan", "review", "update"],
    { trigger: 78, plan: 210, review: 78, update: 78 },
    { top: 48, gap: 58 },
  );
  assert.deepEqual(
    laidOut.filter((node) => !node.parentId).map((node) => [node.id, node.position]),
    [
      ["trigger", { x: 0, y: 48 }],
      ["plan", { x: 0, y: 184 }],
      ["review", { x: 0, y: 452 }],
      ["update", { x: 0, y: 588 }],
    ],
  );
  assert.deepEqual(
    laidOut.find((node) => node.id === "plan-child")?.position,
    { x: 10, y: 86 },
  );
});
