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
const node = (id, kind = "skill", x = 0, y = 0) => ({
  id,
  position: { x, y },
  data: { kind },
});
const branchRef = (conditionId, outcome, parent = []) => [
  ...parent,
  { conditionId, outcome },
];

test("V2 stores a pure linear workflow as one root sequence", async () => {
  const { createWorkflowFlow, workflowNodeIds } = await loadControlFlow();
  const flow = createWorkflowFlow(["trigger", "one", "two"]);

  assert.deepEqual(flow, {
    version: 2,
    root: sequence(step("trigger"), step("one"), step("two")),
  });
  assert.deepEqual(workflowNodeIds(flow), ["trigger", "one", "two"]);
});

test("a condition owns two branches while following root items are its shared continuation", async () => {
  const { findWorkflowItem, getWorkflowSequence, workflowNodeIds } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [step("accepted")], [step("rejected")]),
      step("shared-tail"),
    ),
  };

  assert.deepEqual(getWorkflowSequence(flow, []).items.map((item) => item.nodeId), [
    "trigger",
    "outer",
    "shared-tail",
  ]);
  assert.deepEqual(getWorkflowSequence(flow, branchRef("outer", "true")).items, [step("accepted")]);
  assert.deepEqual(getWorkflowSequence(flow, branchRef("outer", "false")).items, [step("rejected")]);
  assert.deepEqual(findWorkflowItem(flow, "shared-tail"), {
    sequenceRef: [],
    index: 2,
    item: step("shared-tail"),
  });
  assert.deepEqual(workflowNodeIds(flow), [
    "trigger",
    "outer",
    "accepted",
    "rejected",
    "shared-tail",
  ]);
});

test("conditions can nest in either branch and keep local continuations", async () => {
  const { getWorkflowSequence, workflowNodeIds } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [
        condition("inner-true", [step("a")], [step("b")]),
        step("inner-shared"),
      ], [
        condition("inner-false", [step("c")], [step("d")]),
      ]),
      step("outer-shared"),
    ),
  };

  const outerTrue = branchRef("outer", "true");
  assert.deepEqual(getWorkflowSequence(flow, outerTrue).items.map((item) => item.nodeId), [
    "inner-true",
    "inner-shared",
  ]);
  assert.deepEqual(
    getWorkflowSequence(flow, branchRef("inner-true", "false", outerTrue)).items,
    [step("b")],
  );
  assert.deepEqual(workflowNodeIds(flow), [
    "trigger",
    "outer",
    "inner-true",
    "a",
    "b",
    "inner-shared",
    "inner-false",
    "c",
    "d",
    "outer-shared",
  ]);
});

test("at least three levels of nested conditions remain addressable", async () => {
  const { getWorkflowSequence } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("level-1", [
        condition("level-2", [
          condition("level-3", [step("deep-true")], [step("deep-false")]),
          step("level-2-tail"),
        ], []),
        step("level-1-tail"),
      ], []),
      step("root-tail"),
    ),
  };
  const level1True = branchRef("level-1", "true");
  const level2True = branchRef("level-2", "true", level1True);
  const level3False = branchRef("level-3", "false", level2True);

  assert.deepEqual(getWorkflowSequence(flow, level3False).items, [step("deep-false")]);
  assert.deepEqual(getWorkflowSequence(flow, level2True).items.map((item) => item.nodeId), [
    "level-3",
    "level-2-tail",
  ]);
  assert.equal(flow.root.items.at(-1).nodeId, "root-tail");
});

test("V1 linear snapshots migrate without dropping nodes", async () => {
  const { migrateWorkflowSnapshotV1, workflowNodeIds } = await loadControlFlow();
  const nodes = [
    node("trigger", "issue-trigger", 0, 0),
    node("one", "skill", 0, 100),
    node("two", "mcp", 0, 200),
  ];
  const migrated = migrateWorkflowSnapshotV1(nodes, [
    { id: "a", source: "trigger", target: "one" },
    { id: "b", source: "one", target: "two" },
  ]);

  assert.equal(migrated.flow.version, 2);
  assert.deepEqual(workflowNodeIds(migrated.flow), ["trigger", "one", "two"]);
  assert.deepEqual(migrated.nodes.map((entry) => entry.id), ["trigger", "one", "two"]);
});

test("V1 snapshots created before trigger restrictions keep the first trigger and compatible steps", async () => {
  const { migrateWorkflowSnapshotV1, workflowNodeIds } = await loadControlFlow();
  const nodes = [
    node("trigger-one", "issue-trigger", 0, 0),
    node("trigger-two", "issue-trigger", 0, 100),
    node("compatible-step", "skill", 0, 200),
  ];
  const migrated = migrateWorkflowSnapshotV1(nodes, [
    { id: "a", source: "trigger-one", target: "trigger-two" },
    { id: "b", source: "trigger-two", target: "compatible-step" },
  ]);

  assert.deepEqual(workflowNodeIds(migrated.flow), ["trigger-one", "compatible-step"]);
  assert.deepEqual(migrated.nodes.map((entry) => entry.id), ["trigger-one", "compatible-step"]);
});

test("V1 persisted condition paths migrate to true and false branch sequences", async () => {
  const { migrateWorkflowSnapshotV1, workflowNodeIds } = await loadControlFlow();
  const nodes = [
    node("trigger", "issue-trigger", 0, 0),
    node("outer", "condition", 0, 100),
    node("yes", "skill", -180, 300),
    node("no", "mcp", 180, 300),
  ];
  const edges = [
    { id: "trunk", source: "trigger", target: "outer" },
    {
      id: "yes",
      source: "outer",
      target: "yes",
      data: { conditionId: "outer", conditionOutcome: "true" },
    },
    {
      id: "no",
      source: "outer",
      target: "no",
      data: { conditionId: "outer", conditionOutcome: "false" },
    },
  ];
  const migrated = migrateWorkflowSnapshotV1(nodes, edges);

  assert.deepEqual(migrated.flow.root.items, [
    step("trigger"),
    condition("outer", [step("yes")], [step("no")]),
  ]);
  assert.deepEqual(workflowNodeIds(migrated.flow), ["trigger", "outer", "yes", "no"]);
});

test("legacy linear items following a condition migrate into its true branch", async () => {
  const { migrateWorkflowSnapshotV1 } = await loadControlFlow();
  const nodes = [
    node("trigger", "issue-trigger", 0, 0),
    node("outer", "condition", 0, 100),
    node("first", "skill", 0, 200),
    node("second", "mcp", 0, 300),
  ];
  const migrated = migrateWorkflowSnapshotV1(nodes, [
    { id: "a", source: "trigger", target: "outer" },
    { id: "b", source: "outer", target: "first" },
    { id: "c", source: "first", target: "second" },
  ]);

  assert.deepEqual(migrated.flow.root.items, [
    step("trigger"),
    condition("outer", [step("first"), step("second")], []),
  ]);
});

test("V1 linear snapshots recursively migrate later conditions into true branches", async () => {
  const { migrateWorkflowSnapshotV1 } = await loadControlFlow();
  const nodes = [
    node("trigger", "issue-trigger", 0, 0),
    node("first-condition", "condition", 0, 100),
    node("first-action", "skill", 0, 200),
    node("second-condition", "condition", 0, 300),
    node("second-action", "mcp", 0, 400),
    node("third-condition", "condition", 0, 500),
    node("tail", "skill", 0, 600),
  ];
  const migrated = migrateWorkflowSnapshotV1(nodes, [
    { id: "a", source: "trigger", target: "first-condition" },
    { id: "b", source: "first-condition", target: "first-action" },
    { id: "c", source: "first-action", target: "second-condition" },
    { id: "d", source: "second-condition", target: "second-action" },
    { id: "e", source: "second-action", target: "third-condition" },
    { id: "f", source: "third-condition", target: "tail" },
  ]);

  assert.deepEqual(migrated.flow.root.items, [
    step("trigger"),
    condition("first-condition", [
      step("first-action"),
      condition("second-condition", [
        step("second-action"),
        condition("third-condition", [step("tail")], []),
      ], []),
    ], []),
  ]);
});

test("V1 persisted condition branches recursively migrate nested linear conditions", async () => {
  const { migrateWorkflowSnapshotV1 } = await loadControlFlow();
  const nodes = [
    node("trigger", "issue-trigger", 0, 0),
    node("outer", "condition", 0, 100),
    node("yes", "skill", -180, 300),
    node("true-condition", "condition", -180, 400),
    node("true-tail", "mcp", -180, 500),
    node("false-condition", "condition", 180, 300),
    node("false-tail", "skill", 180, 400),
  ];
  const branchEdge = (id, source, target, outcome) => ({
    id,
    source,
    target,
    data: { conditionId: "outer", conditionOutcome: outcome },
  });
  const migrated = migrateWorkflowSnapshotV1(nodes, [
    { id: "root", source: "trigger", target: "outer" },
    branchEdge("a", "outer", "yes", "true"),
    branchEdge("b", "yes", "true-condition", "true"),
    branchEdge("c", "true-condition", "true-tail", "true"),
    branchEdge("d", "outer", "false-condition", "false"),
    branchEdge("e", "false-condition", "false-tail", "false"),
  ]);

  assert.deepEqual(migrated.flow.root.items, [
    step("trigger"),
    condition("outer", [
      step("yes"),
      condition("true-condition", [step("true-tail")], []),
    ], [
      condition("false-condition", [step("false-tail")], []),
    ]),
  ]);
});

test("snapshot normalization rejects unknown flow versions instead of reinterpreting them as V1", async () => {
  const { normalizeWorkflowSnapshot } = await loadControlFlow();
  const trigger = node("trigger", "issue-trigger");

  assert.throws(
    () => normalizeWorkflowSnapshot({
      nodes: [trigger],
      flow: {
        version: 3,
        root: sequence(step("trigger")),
      },
      selectedNodeId: null,
    }),
    /version 2/i,
  );
  assert.throws(
    () => normalizeWorkflowSnapshot({
      nodes: [trigger],
      selectedNodeId: null,
    }),
    /legacy.*edges/i,
  );
});

test("flow validation rejects duplicate, missing and virtual node ids", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();
  const nodes = [node("trigger", "issue-trigger"), node("same"), node("outer", "condition")];

  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger"), step("same"), step("same")),
    }, nodes),
    /unique/,
  );
  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger"), step("missing")),
    }, nodes),
    /missing/,
  );
  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger"), step("__flow-merge-outer")),
    }, [...nodes, node("__flow-merge-outer")]),
    /virtual/,
  );
});

test("V2 validation rejects duplicate node records even when flow references the id once", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();

  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger")),
    }, [
      node("trigger", "issue-trigger"),
      node("trigger", "issue-trigger"),
    ]),
    /node ids must be unique/i,
  );
});

test("V2 validation rejects flow items that reference parented plan children", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();

  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger"), step("plan"), step("plan-child")),
    }, [
      node("trigger", "issue-trigger"),
      node("plan", "planning"),
      { ...node("plan-child", "skill"), parentId: "plan" },
    ]),
    /root node/i,
  );
});

test("V2 validation requires parented nodes to belong to one root child container", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();
  const trigger = node("trigger", "issue-trigger");
  const flow = (...ids) => ({
    version: 2,
    root: sequence(...ids.map(step)),
  });

  assert.throws(
    () => assertWorkflowFlow(flow("trigger"), [
      trigger,
      { ...node("orphan", "skill"), parentId: "missing-plan" },
    ]),
    /missing parent/i,
  );

  assert.throws(
    () => assertWorkflowFlow(flow("trigger", "root-plan"), [
      trigger,
      {
        ...node("root-plan", "basic-planning"),
        data: { kind: "basic-planning", acceptsChildren: true },
      },
      {
        ...node("nested-plan", "basic-planning"),
        parentId: "root-plan",
        data: { kind: "basic-planning", acceptsChildren: true },
      },
      { ...node("nested-child", "skill"), parentId: "nested-plan" },
    ]),
    /root parent/i,
  );

  assert.throws(
    () => assertWorkflowFlow(flow("trigger", "plain-step"), [
      trigger,
      node("plain-step", "skill"),
      { ...node("invalid-child", "mcp"), parentId: "plain-step" },
    ]),
    /acceptsChildren/i,
  );

  assert.doesNotThrow(() => assertWorkflowFlow(flow("trigger", "plan"), [
    trigger,
    {
      ...node("plan", "basic-planning"),
      data: { kind: "basic-planning", acceptsChildren: true },
    },
    { ...node("valid-child", "skill"), parentId: "plan" },
  ]));
});

test("V2 validation requires item types to match their referenced node kinds", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();
  const trigger = node("trigger", "issue-trigger");

  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger"), step("wrong-step")),
    }, [trigger, node("wrong-step", "condition")]),
    /condition/i,
  );
  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger"), condition("wrong-condition")),
    }, [trigger, node("wrong-condition", "skill")]),
    /condition/i,
  );
});

test("V2 validation accepts an empty root but requires one root-first trigger once non-empty", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();

  assert.doesNotThrow(() => assertWorkflowFlow({
    version: 2,
    root: sequence(),
  }, []));
  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("ordinary-step")),
    }, [node("ordinary-step", "skill")]),
    /trigger/i,
  );
});

test("V2 validation rejects unknown node kinds that only look like triggers", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();

  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("unknown-trigger")),
    }, [node("unknown-trigger", "made-up-trigger")]),
    /trigger kind/i,
  );
});

test("V2 validation rejects repeated triggers and triggers outside root index zero", async () => {
  const { assertWorkflowFlow } = await loadControlFlow();
  const nodes = [
    node("trigger-one", "issue-trigger"),
    node("trigger-two", "git-trigger"),
    node("step"),
  ];

  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("trigger-one"), step("trigger-two"), step("step")),
    }, nodes),
    /trigger/i,
  );
  assert.throws(
    () => assertWorkflowFlow({
      version: 2,
      root: sequence(step("step"), step("trigger-one")),
    }, nodes.filter((entry) => entry.id !== "trigger-two")),
    /trigger/i,
  );
});

test("serialized V2 snapshots persist flow but never derived edges or virtual nodes", async () => {
  const { serializeWorkflowSnapshot } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(step("trigger"), condition("outer"), step("tail")),
  };
  const serialized = serializeWorkflowSnapshot([
    node("trigger", "issue-trigger"),
    node("outer", "condition"),
    node("tail"),
    node("__flow-merge-outer"),
  ], flow);

  assert.deepEqual(Object.keys(serialized).sort(), ["flow", "nodes", "selectedNodeId"]);
  assert.equal("edges" in serialized, false);
  assert.deepEqual(serialized.nodes.map((entry) => entry.id), ["trigger", "outer", "tail"]);
  assert.deepEqual(serialized.flow, flow);
});

test("inserting a condition cuts the sequence tail into its true branch", async () => {
  const { createWorkflowFlow, insertWorkflowNode } = await loadControlFlow();
  const flow = createWorkflowFlow(["trigger", "one", "two"]);
  const next = insertWorkflowNode(flow, [], 1, "outer", "condition");

  assert.deepEqual(next.root.items, [
    step("trigger"),
    condition("outer", [step("one"), step("two")], []),
  ]);
});

test("root, branch and continuation insertion all use sequenceRef plus index", async () => {
  const { insertWorkflowNode } = await loadControlFlow();
  const initial = {
    version: 2,
    root: sequence(step("trigger"), condition("outer"), step("root-tail")),
  };
  const withRoot = insertWorkflowNode(initial, [], 1, "before-condition", "skill");
  const withBranch = insertWorkflowNode(
    withRoot,
    branchRef("outer", "true"),
    0,
    "branch-step",
    "skill",
  );
  const withContinuation = insertWorkflowNode(withBranch, [], 3, "after-condition", "skill");

  assert.deepEqual(withContinuation.root.items.map((item) => item.nodeId), [
    "trigger",
    "before-condition",
    "outer",
    "after-condition",
    "root-tail",
  ]);
  assert.deepEqual(
    withContinuation.root.items[2].branches.true.items,
    [step("branch-step")],
  );
});

test("conditions can be inserted inside a branch and own that branch tail", async () => {
  const { insertWorkflowNode } = await loadControlFlow();
  const initial = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [step("one"), step("two")], []),
      step("root-tail"),
    ),
  };
  const next = insertWorkflowNode(
    initial,
    branchRef("outer", "true"),
    1,
    "inner",
    "condition",
  );

  assert.deepEqual(next.root.items[1].branches.true.items, [
    step("one"),
    condition("inner", [step("two")], []),
  ]);
  assert.equal(next.root.items.at(-1).nodeId, "root-tail");
});

test("deleting inner or outer conditions removes only their branch subtrees and preserves continuation", async () => {
  const { deleteWorkflowNode } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [
        condition("inner", [step("inner-yes")], [step("inner-no")]),
        step("outer-true-tail"),
      ], [step("outer-no")]),
      step("root-tail"),
    ),
  };
  const deletedInner = deleteWorkflowNode(flow, "inner");
  assert.deepEqual(deletedInner.removedNodeIds.sort(), ["inner", "inner-no", "inner-yes"]);
  assert.deepEqual(
    deletedInner.flow.root.items[1].branches.true.items,
    [step("outer-true-tail")],
  );
  assert.equal(deletedInner.flow.root.items.at(-1).nodeId, "root-tail");

  const deletedOuter = deleteWorkflowNode(flow, "outer");
  assert.deepEqual(deletedOuter.flow.root.items, [step("trigger"), step("root-tail")]);
  assert.deepEqual(deletedOuter.removedNodeIds.sort(), [
    "inner",
    "inner-no",
    "inner-yes",
    "outer",
    "outer-no",
    "outer-true-tail",
  ]);
});

test("moving a condition carries its subtree but not the source sequence continuation", async () => {
  const { moveWorkflowNode } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [step("yes")], [step("no")]),
      step("continuation"),
      step("last"),
    ),
  };
  const moved = moveWorkflowNode(flow, "outer", [], 4);

  assert.deepEqual(moved.root.items.map((item) => item.nodeId), [
    "trigger",
    "continuation",
    "last",
    "outer",
  ]);
  assert.deepEqual(moved.root.items.at(-1).branches.true.items, [step("yes")]);
  assert.deepEqual(moved.root.items.at(-1).branches.false.items, [step("no")]);
});

test("moving into a condition's descendant is rejected and the trigger stays root-first", async () => {
  const { moveWorkflowNode } = await loadControlFlow();
  const flow = {
    version: 2,
    root: sequence(
      step("trigger"),
      condition("outer", [step("yes")], []),
      step("tail"),
    ),
  };

  assert.throws(
    () => moveWorkflowNode(flow, "outer", branchRef("outer", "true"), 1),
    /descendant/,
  );
  assert.throws(() => moveWorkflowNode(flow, "trigger", [], 2), /trigger/);
  assert.throws(() => moveWorkflowNode(flow, "tail", [], 0), /trigger/);
});
