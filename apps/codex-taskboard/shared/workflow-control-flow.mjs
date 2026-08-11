import {
  normalizeWorkflowConditionBranches,
} from "./workflow-sequence.mjs";

const NODE_WIDTH = 250;
const DEFAULT_NODE_HEIGHT = 138;
const SEQUENCE_GAP = 58;
const BRANCH_GAP = 110;
const SPLIT_OFFSET = 28;
const BRANCH_TOP_OFFSET = 94;
const MERGE_RAIL_OFFSET = 36;
const MERGE_OFFSET = 28;
const ROOT_TOP = 48;

export const WORKFLOW_TRIGGER_KINDS = Object.freeze([
  "issue-trigger",
  "rss-trigger",
  "pull-request-submitted-trigger",
  "repository-issue-submitted-trigger",
  "git-status-trigger",
]);

const WORKFLOW_TRIGGER_KIND_SET = new Set(WORKFLOW_TRIGGER_KINDS);

export function isWorkflowTriggerKind(kind) {
  return WORKFLOW_TRIGGER_KIND_SET.has(kind);
}

function stepItem(nodeId) {
  return { type: "step", nodeId };
}

function conditionItem(nodeId, trueItems = [], falseItems = []) {
  return {
    type: "condition",
    nodeId,
    branches: {
      true: { items: trueItems },
      false: { items: falseItems },
    },
  };
}

function legacySequenceItems(nodeIds, nodesById, persistedBranchesByCondition) {
  const items = [];
  for (let index = 0; index < nodeIds.length; index += 1) {
    const nodeId = nodeIds[index];
    if (nodesById.get(nodeId)?.data?.kind !== "condition") {
      items.push(stepItem(nodeId));
      continue;
    }
    const persistedBranches = persistedBranchesByCondition.get(nodeId);
    if (persistedBranches) {
      items.push(conditionItem(
        nodeId,
        legacySequenceItems(
          persistedBranches.true,
          nodesById,
          persistedBranchesByCondition,
        ),
        legacySequenceItems(
          persistedBranches.false,
          nodesById,
          persistedBranchesByCondition,
        ),
      ));
      continue;
    }
    items.push(conditionItem(
      nodeId,
      legacySequenceItems(
        nodeIds.slice(index + 1),
        nodesById,
        persistedBranchesByCondition,
      ),
      [],
    ));
    break;
  }
  return items;
}

function cloneFlow(flow) {
  return JSON.parse(JSON.stringify(flow));
}

function virtualNodeId(nodeId) {
  return nodeId.startsWith("__flow-")
    || nodeId.startsWith("__condition-")
    || nodeId === "__workflow-sequence-end__";
}

function visitItems(sequence, visitor, sequenceRef = []) {
  sequence.items.forEach((item, index) => {
    visitor(item, sequenceRef, index);
    if (item.type !== "condition") return;
    visitItems(
      item.branches.true,
      visitor,
      [...sequenceRef, { conditionId: item.nodeId, outcome: "true" }],
    );
    visitItems(
      item.branches.false,
      visitor,
      [...sequenceRef, { conditionId: item.nodeId, outcome: "false" }],
    );
  });
}

function itemNodeIds(item) {
  const ids = [item.nodeId];
  if (item.type === "condition") {
    for (const branch of [item.branches.true, item.branches.false]) {
      for (const child of branch.items) ids.push(...itemNodeIds(child));
    }
  }
  return ids;
}

function isTriggerNode(node) {
  return isWorkflowTriggerKind(node?.data?.kind);
}

function removeWorkflowItem(sequence, nodeId) {
  const index = sequence.items.findIndex((item) => item.nodeId === nodeId);
  if (index >= 0) return sequence.items.splice(index, 1)[0];
  for (const item of sequence.items) {
    if (item.type !== "condition") continue;
    for (const branch of [item.branches.true, item.branches.false]) {
      const removed = removeWorkflowItem(branch, nodeId);
      if (removed) return removed;
    }
  }
  return null;
}

export function createWorkflowFlow(stepIds = []) {
  return {
    version: 2,
    root: {
      items: stepIds.map(stepItem),
    },
  };
}

export function workflowNodeIds(flow) {
  const ids = [];
  visitItems(flow.root, (item) => ids.push(item.nodeId));
  return ids;
}

export function getWorkflowSequence(flow, sequenceRef) {
  let sequence = flow.root;
  for (const segment of sequenceRef) {
    const item = sequence.items.find((candidate) => (
      candidate.type === "condition" && candidate.nodeId === segment.conditionId
    ));
    if (!item) throw new Error(`Workflow condition ${segment.conditionId} is not in this sequence`);
    sequence = item.branches[segment.outcome];
  }
  return sequence;
}

export function findWorkflowItem(flow, nodeId) {
  let found = null;
  visitItems(flow.root, (item, sequenceRef, index) => {
    if (!found && item.nodeId === nodeId) {
      found = { sequenceRef, index, item };
    }
  });
  return found;
}

export function assertWorkflowFlow(flow, nodes) {
  if (!flow || flow.version !== 2 || !flow.root || !Array.isArray(flow.root.items)) {
    throw new Error("Workflow control flow must use version 2");
  }
  const nodeIds = new Set(nodes.map((node) => node.id));
  if (nodeIds.size !== nodes.length) throw new Error("Workflow node ids must be unique");
  const nodesById = new Map(nodes.map((node) => [node.id, node]));
  const visited = new Set();
  let rootTriggerCount = 0;
  visitItems(flow.root, (item, sequenceRef, index) => {
    if (
      !item
      || (item.type !== "step" && item.type !== "condition")
      || typeof item.nodeId !== "string"
    ) {
      throw new Error("Workflow items must be steps or conditions");
    }
    if (virtualNodeId(item.nodeId)) throw new Error("Workflow flow cannot persist virtual nodes");
    if (visited.has(item.nodeId)) throw new Error("Workflow node ids must be unique");
    if (!nodeIds.has(item.nodeId)) throw new Error(`Workflow flow references missing node ${item.nodeId}`);
    const referencedNode = nodesById.get(item.nodeId);
    if (referencedNode.parentId) {
      throw new Error(`Workflow flow item ${item.nodeId} must reference a root node`);
    }
    const referencedKind = referencedNode?.data?.kind;
    if (
      typeof referencedKind === "string"
      && referencedKind.endsWith("-trigger")
      && !isWorkflowTriggerKind(referencedKind)
    ) {
      throw new Error(`Unsupported workflow trigger kind ${referencedKind}`);
    }
    visited.add(item.nodeId);
    const referencesConditionNode = referencedNode?.data?.kind === "condition";
    if (item.type === "condition" && !referencesConditionNode) {
      throw new Error(`Workflow condition item ${item.nodeId} must reference a condition node`);
    }
    if (item.type === "step" && referencesConditionNode) {
      throw new Error(`Workflow condition node ${item.nodeId} must use a condition item`);
    }
    if (item.type === "condition") {
      if (
        !item.branches
        || !item.branches.true
        || !item.branches.false
        || !Array.isArray(item.branches.true.items)
        || !Array.isArray(item.branches.false.items)
      ) {
        throw new Error("Workflow conditions must own true and false sequences");
      }
    }
    if (isTriggerNode(referencedNode)) {
      if (sequenceRef.length > 0 || index !== 0) {
        throw new Error("Workflow trigger must stay first in the root sequence");
      }
      rootTriggerCount += 1;
    }
  });
  if (flow.root.items.length > 0 && rootTriggerCount !== 1) {
    throw new Error("A non-empty workflow must contain exactly one root-first trigger");
  }
  for (const node of nodes) {
    if (!node.parentId) {
      if (!virtualNodeId(node.id) && !visited.has(node.id)) {
        throw new Error(`Workflow node ${node.id} is missing from control flow`);
      }
      continue;
    }
    const parent = nodesById.get(node.parentId);
    if (!parent) {
      throw new Error(`Workflow child node ${node.id} references missing parent ${node.parentId}`);
    }
    if (parent.parentId) {
      throw new Error(`Workflow child node ${node.id} must reference a root parent`);
    }
    if (parent.data?.acceptsChildren !== true) {
      throw new Error(`Workflow parent node ${parent.id} must set data.acceptsChildren to true`);
    }
  }
  return flow;
}

export function migrateWorkflowSnapshotV1(nodes, edges, selectedNodeId = null) {
  let persistedNodes = nodes.filter((node) => !virtualNodeId(node.id));
  const rootNodes = persistedNodes.filter((node) => !node.parentId);
  const graph = normalizeWorkflowConditionBranches(rootNodes, edges);
  const nodesById = new Map(persistedNodes.map((node) => [node.id, node]));
  const persistedBranchesByCondition = new Map();
  if (graph.conditionId) {
    persistedBranchesByCondition.set(graph.conditionId, graph.branches);
  }
  const rootItems = legacySequenceItems(
    graph.trunkStepIds,
    nodesById,
    persistedBranchesByCondition,
  );
  const flow = { version: 2, root: { items: rootItems } };
  const triggerIds = workflowNodeIds(flow).filter((nodeId) => isTriggerNode(nodesById.get(nodeId)));
  const primaryTriggerId = triggerIds[0] ?? null;
  for (const duplicateTriggerId of triggerIds.slice(1)) {
    removeWorkflowItem(flow.root, duplicateTriggerId);
  }
  if (primaryTriggerId && flow.root.items[0]?.nodeId !== primaryTriggerId) {
    const primaryTrigger = removeWorkflowItem(flow.root, primaryTriggerId);
    if (primaryTrigger) flow.root.items.unshift(primaryTrigger);
  }
  const discardedTriggerIds = new Set(triggerIds.slice(1));
  persistedNodes = persistedNodes.filter((node) => !discardedTriggerIds.has(node.id));
  assertWorkflowFlow(flow, persistedNodes);
  return {
    nodes: persistedNodes,
    flow,
    selectedNodeId: persistedNodes.some((node) => node.id === selectedNodeId)
      ? selectedNodeId
      : null,
  };
}

export function normalizeWorkflowSnapshot(snapshot) {
  if (snapshot?.flow !== undefined) {
    if (snapshot.flow?.version !== 2) {
      throw new Error("Workflow control flow must use version 2");
    }
    const nodes = snapshot.nodes.filter((node) => !virtualNodeId(node.id));
    assertWorkflowFlow(snapshot.flow, nodes);
    return {
      nodes,
      flow: cloneFlow(snapshot.flow),
      selectedNodeId: nodes.some((node) => node.id === snapshot.selectedNodeId)
        ? snapshot.selectedNodeId
      : null,
    };
  }
  if (!Array.isArray(snapshot?.edges)) {
    throw new Error("Legacy workflow snapshots must provide edges");
  }
  return migrateWorkflowSnapshotV1(
    snapshot?.nodes ?? [],
    snapshot?.edges ?? [],
    snapshot?.selectedNodeId ?? null,
  );
}

export function serializeWorkflowSnapshot(nodes, flow, selectedNodeId = null) {
  const persistedNodes = nodes.filter((node) => !virtualNodeId(node.id));
  assertWorkflowFlow(flow, persistedNodes);
  return {
    nodes: persistedNodes,
    flow: cloneFlow(flow),
    selectedNodeId: persistedNodes.some((node) => node.id === selectedNodeId)
      ? selectedNodeId
      : null,
  };
}

export function insertWorkflowNode(flow, sequenceRef, index, nodeId, kind) {
  const next = cloneFlow(flow);
  const sequence = getWorkflowSequence(next, sequenceRef);
  const insertionIndex = Math.max(0, Math.min(Math.trunc(index), sequence.items.length));
  if (kind === "condition") {
    const tail = sequence.items.splice(insertionIndex);
    sequence.items.push(conditionItem(nodeId, tail, []));
  } else {
    sequence.items.splice(insertionIndex, 0, stepItem(nodeId));
  }
  return next;
}

export function deleteWorkflowNode(flow, nodeId) {
  const next = cloneFlow(flow);
  const found = findWorkflowItem(next, nodeId);
  if (!found) return { flow: next, removedNodeIds: [] };
  const rootFirst = next.root.items[0];
  if (rootFirst?.nodeId === nodeId) throw new Error("Workflow trigger cannot be deleted");
  const sequence = getWorkflowSequence(next, found.sequenceRef);
  const [removed] = sequence.items.splice(found.index, 1);
  return {
    flow: next,
    removedNodeIds: itemNodeIds(removed),
  };
}

export function moveWorkflowNode(flow, nodeId, targetSequenceRef, targetIndex) {
  const found = findWorkflowItem(flow, nodeId);
  if (!found) return cloneFlow(flow);
  if (found.sequenceRef.length === 0 && found.index === 0) {
    throw new Error("Workflow trigger must stay first");
  }
  if (
    found.item.type === "condition"
    && targetSequenceRef.some((segment) => segment.conditionId === nodeId)
  ) {
    throw new Error("Workflow condition cannot move into its own descendant");
  }
  const next = cloneFlow(flow);
  const sourceSequence = getWorkflowSequence(next, found.sequenceRef);
  const [item] = sourceSequence.items.splice(found.index, 1);
  const targetSequence = getWorkflowSequence(next, targetSequenceRef);
  const insertionIndex = Math.max(0, Math.min(Math.trunc(targetIndex), targetSequence.items.length));
  if (targetSequenceRef.length === 0 && insertionIndex === 0) {
    throw new Error("Workflow trigger must stay first");
  }
  targetSequence.items.splice(insertionIndex, 0, item);
  return next;
}

function nodeHeight(node) {
  const height = Number(node?.style?.height ?? node?.measured?.height);
  return Number.isFinite(height) && height > 0 ? height : DEFAULT_NODE_HEIGHT;
}

function measureSequence(sequence, nodesById) {
  let width = NODE_WIDTH;
  let height = 0;
  sequence.items.forEach((item, index) => {
    const measured = measureItem(item, nodesById);
    width = Math.max(width, measured.width);
    height += measured.height;
    if (index < sequence.items.length - 1) height += SEQUENCE_GAP;
  });
  return { width, height };
}

function measureItem(item, nodesById) {
  const height = nodeHeight(nodesById.get(item.nodeId));
  if (item.type === "step") return { width: NODE_WIDTH, height };
  const trueMeasure = measureSequence(item.branches.true, nodesById);
  const falseMeasure = measureSequence(item.branches.false, nodesById);
  return {
    width: Math.max(NODE_WIDTH, trueMeasure.width + BRANCH_GAP + falseMeasure.width),
    height: height
      + BRANCH_TOP_OFFSET
      + Math.max(trueMeasure.height, falseMeasure.height)
      + MERGE_RAIL_OFFSET
      + MERGE_OFFSET,
  };
}

function route(...points) {
  const compact = [];
  for (const point of points) {
    const previous = compact.at(-1);
    if (previous && previous.x === point.x && previous.y === point.y) continue;
    compact.push(point);
  }
  return compact;
}

function boundsForNode(x, y, height) {
  return {
    left: x - NODE_WIDTH / 2,
    right: x + NODE_WIDTH / 2,
    top: y,
    bottom: y + height,
  };
}

function unionBounds(...bounds) {
  return {
    left: Math.min(...bounds.map((entry) => entry.left)),
    right: Math.max(...bounds.map((entry) => entry.right)),
    top: Math.min(...bounds.map((entry) => entry.top)),
    bottom: Math.max(...bounds.map((entry) => entry.bottom)),
  };
}

export function deriveWorkflowLayout(flow, nodes) {
  const nodesById = new Map(nodes.filter((node) => !node.parentId).map((node) => [node.id, node]));
  const positions = {};
  const conditions = {};
  const virtualNodes = [];
  const edges = [];
  const insertionPoints = [];
  const insertionKeys = new Set();
  let edgeNumber = 0;

  function addVirtual(id, kind, x, y) {
    virtualNodes.push({ id, kind, position: { x, y } });
  }

  function addEdge(source, target, points, options = {}) {
    const id = options.id ?? `__flow-edge-${edgeNumber++}`;
    const data = {
      points: route(...points),
      ...(options.conditionOutcome
        ? {
            conditionId: options.conditionId,
            conditionOutcome: options.conditionOutcome,
            branchStart: options.branchStart === true,
            labelX: options.labelX,
            labelY: options.labelY,
          }
        : {}),
    };
    if (options.insertion) {
      const key = JSON.stringify(options.insertion);
      if (insertionKeys.has(key)) {
        throw new Error(`Duplicate workflow insertion point ${key}`);
      }
      insertionKeys.add(key);
      data.insertion = options.insertion;
      data.buttonX = options.buttonX;
      data.buttonY = options.buttonY;
      insertionPoints.push({
        id: `__flow-insert-${insertionPoints.length}`,
        edgeId: id,
        x: options.buttonX,
        y: options.buttonY,
        insertion: options.insertion,
      });
    }
    edges.push({
      id,
      source,
      target,
      type: "workflowInsert",
      data,
    });
  }

  function placeSequence(sequence, sequenceRef, centerX, startY) {
    let previous = null;
    let firstInput = null;
    let bounds = null;
    sequence.items.forEach((item, index) => {
      const itemY = previous ? previous.y + SEQUENCE_GAP : startY;
      const placed = placeItem(item, sequenceRef, centerX, itemY);
      if (!firstInput) firstInput = placed.input;
      if (previous) {
        const insertion = { sequenceRef, index };
        addEdge(
          previous.id,
          placed.input.id,
          route(previous, placed.input),
          {
            insertion,
            buttonX: centerX,
            buttonY: previous.y + (placed.input.y - previous.y) / 2,
          },
        );
      }
      previous = placed.output;
      bounds = bounds ? unionBounds(bounds, placed.bounds) : placed.bounds;
    });
    const emptyBounds = {
      left: centerX - NODE_WIDTH / 2,
      right: centerX + NODE_WIDTH / 2,
      top: startY,
      bottom: startY,
    };
    return {
      firstInput,
      output: previous,
      bounds: bounds ?? emptyBounds,
      top: startY,
      bottom: previous?.y ?? startY,
    };
  }

  function placeItem(item, sequenceRef, centerX, y) {
    const height = nodeHeight(nodesById.get(item.nodeId));
    positions[item.nodeId] = { x: centerX, y };
    const input = { id: item.nodeId, x: centerX, y };
    const nodeBottom = y + height;
    const ownBounds = boundsForNode(centerX, y, height);
    if (item.type === "step") {
      return {
        input,
        output: { id: item.nodeId, x: centerX, y: nodeBottom },
        bounds: ownBounds,
      };
    }

    const conditionId = item.nodeId;
    const trueMeasure = measureSequence(item.branches.true, nodesById);
    const falseMeasure = measureSequence(item.branches.false, nodesById);
    const totalWidth = trueMeasure.width + BRANCH_GAP + falseMeasure.width;
    const left = centerX - totalWidth / 2;
    const branchCenters = {
      true: left + trueMeasure.width / 2,
      false: left + trueMeasure.width + BRANCH_GAP + falseMeasure.width / 2,
    };
    const splitY = nodeBottom + SPLIT_OFFSET;
    const branchTop = nodeBottom + BRANCH_TOP_OFFSET;
    const splitId = `__flow-split-${conditionId}`;
    addVirtual(splitId, "flow-split", centerX, splitY);
    addEdge(
      conditionId,
      splitId,
      [{ id: conditionId, x: centerX, y: nodeBottom }, { x: centerX, y: splitY }],
      { id: `__flow-${conditionId}-to-split` },
    );

    const branchLayouts = {};
    for (const outcome of ["true", "false"]) {
      const branch = item.branches[outcome];
      const branchRef = [...sequenceRef, { conditionId, outcome }];
      const branchX = branchCenters[outcome];
      const placed = placeSequence(branch, branchRef, branchX, branchTop);
      const emptyId = `__flow-empty-${conditionId}-${outcome}`;
      let branchInput = placed.firstInput;
      let branchOutput = placed.output;
      if (!branchInput) {
        addVirtual(emptyId, "flow-empty", branchX, branchTop);
        branchInput = { id: emptyId, x: branchX, y: branchTop };
        branchOutput = branchInput;
      }
      addEdge(
        splitId,
        branchInput.id,
        [
          { id: splitId, x: centerX, y: splitY },
          { x: branchX, y: splitY },
          branchInput,
        ],
        {
          id: `__flow-${conditionId}-${outcome}-start`,
          conditionId,
          conditionOutcome: outcome,
          branchStart: true,
          labelX: branchX,
          labelY: splitY + 14,
          insertion: { sequenceRef: branchRef, index: 0 },
          buttonX: branchX,
          buttonY: splitY + (branchTop - splitY) * 0.68,
        },
      );
      branchLayouts[outcome] = {
        ...placed,
        input: branchInput,
        output: branchOutput,
        bounds: placed.firstInput
          ? placed.bounds
          : {
              left: branchX - NODE_WIDTH / 2,
              right: branchX + NODE_WIDTH / 2,
              top: branchTop,
              bottom: branchTop,
            },
        sequenceRef: branchRef,
        length: branch.items.length,
      };
    }

    const branchBottom = Math.max(
      branchLayouts.true.output.y,
      branchLayouts.false.output.y,
    );
    const mergeRailY = branchBottom + MERGE_RAIL_OFFSET;
    const mergeY = mergeRailY + MERGE_OFFSET;
    const mergeId = `__flow-merge-${conditionId}`;
    addVirtual(mergeId, "flow-merge", centerX, mergeY);
    for (const outcome of ["true", "false"]) {
      const branch = branchLayouts[outcome];
      addEdge(
        branch.output.id,
        mergeId,
        [
          branch.output,
          { x: branch.output.x, y: mergeRailY },
          { x: centerX, y: mergeRailY },
          { id: mergeId, x: centerX, y: mergeY },
        ],
        branch.length > 0
          ? {
              id: `__flow-${conditionId}-${outcome}-merge`,
              conditionId,
              conditionOutcome: outcome,
              insertion: {
                sequenceRef: branch.sequenceRef,
                index: branch.length,
              },
              buttonX: branch.output.x,
              buttonY: branch.output.y + (mergeRailY - branch.output.y) / 2,
            }
          : {
              id: `__flow-${conditionId}-${outcome}-merge`,
              conditionId,
              conditionOutcome: outcome,
            },
      );
    }
    const branchBounds = {
      true: branchLayouts.true.bounds,
      false: branchLayouts.false.bounds,
    };
    const bounds = unionBounds(
      ownBounds,
      branchBounds.true,
      branchBounds.false,
      {
        left: centerX,
        right: centerX,
        top: splitY,
        bottom: mergeY,
      },
    );
    conditions[conditionId] = {
      nodeBottom,
      splitY,
      mergeRailY,
      mergeY,
      mergeX: centerX,
      branchCenters,
      branchBounds,
      bounds,
    };
    return {
      input,
      output: { id: mergeId, x: centerX, y: mergeY },
      bounds,
    };
  }

  const root = placeSequence(flow.root, [], 0, ROOT_TOP);
  if (flow.root.items.length === 0) {
    const insertion = { sequenceRef: [], index: 0 };
    insertionPoints.push({
      id: "__flow-insert-root-empty",
      edgeId: null,
      x: 0,
      y: ROOT_TOP,
      insertion,
    });
  } else {
    const endId = "__flow-end-root";
    const endY = root.output.y + SEQUENCE_GAP;
    addVirtual(endId, "flow-end", 0, endY);
    addEdge(
      root.output.id,
      endId,
      [root.output, { id: endId, x: 0, y: endY }],
      {
        id: "__flow-root-end",
        insertion: { sequenceRef: [], index: flow.root.items.length },
        buttonX: 0,
        buttonY: root.output.y + SEQUENCE_GAP / 2,
      },
    );
  }

  return {
    positions,
    conditions,
    virtualNodes,
    edges,
    insertionPoints,
    bounds: root.bounds,
  };
}
