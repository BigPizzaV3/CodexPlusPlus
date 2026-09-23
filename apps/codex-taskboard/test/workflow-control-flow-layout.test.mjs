import assert from "node:assert/strict";
import test from "node:test";

const loadControlFlow = () => import("../shared/workflow-control-flow.mjs");

const step = (nodeId) => ({ type: "step", nodeId });
const sequence = (...items) => ({ items });
const condition = (nodeId, trueItems = [], falseItems = []) => ({
  type: "condition",
  nodeId,
  branches: {
    true: sequence(...trueItems),
    false: sequence(...falseItems),
  },
});
const node = (id, kind = "skill", height = 138) => ({
  id,
  data: { kind },
  style: { height },
});
const insertionKey = (insertion) => JSON.stringify(insertion);

function assertOrthogonalMonotone(points) {
  for (let index = 1; index < points.length; index += 1) {
    const previous = points[index - 1];
    const current = points[index];
    assert.ok(previous.x === current.x || previous.y === current.y, "edge segment must be orthogonal");
    assert.ok(current.y >= previous.y, "edge must never turn upward");
  }
}

test("empty and unequal branches merge below the taller side with one add point per insertion", async () => {
  const { deriveWorkflowLayout } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [step("tall-a"), step("tall-b")], []),
      step("shared"),
    ),
  };
  const layout = deriveWorkflowLayout(flow, [
    node("trigger", "issue-trigger"),
    node("outer", "condition"),
    node("tall-a"),
    node("tall-b", "skill", 190),
    node("shared"),
  ]);
  const outer = layout.conditions.outer;
  const shared = layout.positions.shared;

  assert.ok(outer.mergeY > outer.branchBounds.true.bottom);
  assert.ok(outer.mergeY > outer.branchBounds.false.bottom);
  assert.ok(shared.y > outer.mergeY);
  assert.equal(outer.branchBounds.false.top, outer.branchBounds.true.top);
  const keys = layout.insertionPoints.map((entry) => insertionKey(entry.insertion));
  assert.equal(new Set(keys).size, keys.length);
  assert.equal(
    keys.filter((key) => key === insertionKey({
      sequenceRef: [{ conditionId: "outer", outcome: "false" }],
      index: 0,
    })).length,
    1,
  );
});

test("nested merges return to the parent lane before the parent merge", async () => {
  const { deriveWorkflowLayout } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [
        condition("inner", [step("a")], [step("b")]),
        step("inner-continuation"),
      ], [step("outer-false")]),
      step("outer-continuation"),
    ),
  };
  const layout = deriveWorkflowLayout(flow, [
    node("trigger", "issue-trigger"),
    node("outer", "condition"),
    node("inner", "condition"),
    node("a"),
    node("b", "skill", 180),
    node("inner-continuation"),
    node("outer-false"),
    node("outer-continuation"),
  ]);
  const inner = layout.conditions.inner;
  const outer = layout.conditions.outer;

  assert.ok(inner.mergeY < layout.positions["inner-continuation"].y);
  assert.ok(layout.positions["inner-continuation"].y < outer.mergeY);
  assert.ok(outer.mergeY > inner.bounds.bottom);
  assert.equal(inner.mergeX, outer.branchCenters.true);
  assert.ok(layout.positions["outer-continuation"].y > outer.mergeY);
});

test("both nested sides and three levels measure disjoint sibling bounds", async () => {
  const { deriveWorkflowLayout } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [
        condition("left-2", [
          condition("left-3", [step("a")], [step("b")]),
        ], [step("c")]),
      ], [
        condition("right-2", [step("d")], [
          condition("right-3", [step("e")], [step("f")]),
        ]),
      ]),
      step("shared"),
    ),
  };
  const nodes = [
    node("trigger", "issue-trigger"),
    node("outer", "condition"),
    node("left-2", "condition"),
    node("left-3", "condition"),
    node("right-2", "condition"),
    node("right-3", "condition"),
    ...["a", "b", "c", "d", "e", "f", "shared"].map((id) => node(id)),
  ];
  const layout = deriveWorkflowLayout(flow, nodes);
  const outer = layout.conditions.outer;

  assert.ok(outer.branchBounds.true.right < outer.branchBounds.false.left);
  assert.ok(layout.conditions["left-3"].mergeY < layout.conditions["left-2"].mergeY);
  assert.ok(layout.conditions["right-3"].mergeY < layout.conditions["right-2"].mergeY);
  assert.ok(layout.conditions["left-2"].mergeY < outer.mergeY);
  assert.ok(layout.conditions["right-2"].mergeY < outer.mergeY);
  assert.ok(layout.positions.shared.y > outer.mergeY);
});

test("all derived routes are orthogonal, downward and use explicit local split or merge rails", async () => {
  const { deriveWorkflowLayout } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [
        condition("inner", [step("a")], []),
      ], [step("b")]),
      step("tail"),
    ),
  };
  const layout = deriveWorkflowLayout(flow, [
    node("trigger", "issue-trigger"),
    node("outer", "condition"),
    node("inner", "condition"),
    node("a"),
    node("b"),
    node("tail"),
  ]);

  for (const edge of layout.edges) assertOrthogonalMonotone(edge.data.points);
  for (const conditionLayout of Object.values(layout.conditions)) {
    assert.ok(conditionLayout.splitY > conditionLayout.nodeBottom);
    assert.ok(conditionLayout.mergeRailY < conditionLayout.mergeY);
    assert.ok(conditionLayout.mergeY > conditionLayout.branchBounds.true.bottom);
    assert.ok(conditionLayout.mergeY > conditionLayout.branchBounds.false.bottom);
  }
});

test("virtual split, empty and merge anchors are derived and never overlap real ids", async () => {
  const { deriveWorkflowLayout } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(step("trigger"), condition("outer"), step("tail")),
  };
  const layout = deriveWorkflowLayout(flow, [
    node("trigger", "issue-trigger"),
    node("outer", "condition"),
    node("tail"),
  ]);
  const virtualIds = layout.virtualNodes.map((entry) => entry.id);

  assert.deepEqual(virtualIds.filter((id) => id.includes("outer")).sort(), [
    "__flow-empty-outer-false",
    "__flow-empty-outer-true",
    "__flow-merge-outer",
    "__flow-split-outer",
  ]);
  assert.equal(new Set(virtualIds).size, virtualIds.length);
  assert.equal(virtualIds.some((id) => ["trigger", "outer", "tail"].includes(id)), false);
});

test("shared continuation has one incoming merge edge and one insertion point", async () => {
  const { deriveWorkflowLayout } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [step("yes")], [step("no")]),
      step("shared"),
    ),
  };
  const layout = deriveWorkflowLayout(flow, [
    node("trigger", "issue-trigger"),
    node("outer", "condition"),
    node("yes"),
    node("no"),
    node("shared"),
  ]);
  const mergeId = "__flow-merge-outer";
  const incomingShared = layout.edges.filter((edge) => edge.target === "shared");

  assert.equal(incomingShared.length, 1);
  assert.equal(incomingShared[0].source, mergeId);
  assert.deepEqual(incomingShared[0].data.insertion, {
    sequenceRef: [],
    index: 2,
  });
  assert.equal(
    layout.insertionPoints.filter((entry) => (
      insertionKey(entry.insertion) === insertionKey({ sequenceRef: [], index: 2 })
    )).length,
    1,
  );
});
