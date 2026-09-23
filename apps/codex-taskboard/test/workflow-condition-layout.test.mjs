import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const board = await readFile(new URL("../web/src/components/WorkflowBoard.tsx", import.meta.url), "utf8");
const edge = await readFile(new URL("../web/src/components/WorkflowInsertEdge.tsx", import.meta.url), "utf8");
const node = await readFile(new URL("../web/src/components/WorkflowNode.tsx", import.meta.url), "utf8");
const styles = await readFile(new URL("../web/src/components/workflow.css", import.meta.url), "utf8");
const control = await readFile(new URL("../shared/workflow-control-flow.mjs", import.meta.url), "utf8");

test("condition branch starts use a shared explicit orthogonal trunk and rail", () => {
  assert.doesNotMatch(edge, /getSmoothStepPath/);
  assert.match(edge, /data\?\.points/);
  assert.match(
    control,
    /const splitId = `__flow-split-\$\{conditionId\}`[\s\S]*?addEdge\([\s\S]*?conditionId,[\s\S]*?splitId/,
  );
  assert.match(
    control,
    /addEdge\([\s\S]*?splitId,[\s\S]*?branchInput\.id,[\s\S]*?branchStart: true/,
  );
});

test("condition path labels live on the branch edge instead of stretching inside the card", () => {
  assert.doesNotMatch(node, /workflow-condition-outcomes/);
  assert.match(node, /data\.kind\.startsWith\("flow-"\)/);
  assert.match(
    edge,
    /data\?\.branchStart[\s\S]*?workflow-condition-branch-label[\s\S]*?成立[\s\S]*?不成立/,
  );
  assert.match(
    styles,
    /\.workflow-condition-branch-label \{[\s\S]*?width: max-content;[\s\S]*?background: var\(--text-primary\)/,
  );
});

test("derived split, empty and merge anchors never render duplicate add-step buttons", () => {
  const anchors = node.slice(
    node.indexOf('if (data.kind.startsWith("flow-"))'),
    node.indexOf("if (parentId)"),
  );
  assert.match(anchors, /workflow-flow-anchor[\s\S]*?<Handle/);
  assert.doesNotMatch(anchors, /<button|添加步骤|onAddBranch/);
  assert.match(
    board,
    /layout\.virtualNodes\.map[\s\S]*?style: \{ width: END_STEP_HEIGHT, height: END_STEP_HEIGHT \}/,
  );
  assert.doesNotMatch(styles, /\.workflow-flow-anchor > button/);
});

test("empty condition anchors and populated first steps share one branch top", () => {
  assert.match(
    control,
    /const branchTop = nodeBottom \+ BRANCH_TOP_OFFSET/,
  );
  assert.match(
    control,
    /placeSequence\(branch, branchRef, branchX, branchTop\)/,
  );
  assert.match(
    control,
    /addVirtual\(emptyId, "flow-empty", branchX, branchTop\)/,
  );
});
