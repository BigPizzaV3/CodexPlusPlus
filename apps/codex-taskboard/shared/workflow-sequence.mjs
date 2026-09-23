function nodePosition(node) {
  return {
    x: Number.isFinite(node.position?.x) ? node.position.x : 0,
    y: Number.isFinite(node.position?.y) ? node.position.y : 0,
  };
}

function compareNodes(left, right) {
  const leftPosition = nodePosition(left);
  const rightPosition = nodePosition(right);
  return leftPosition.y - rightPosition.y
    || leftPosition.x - rightPosition.x
    || left.id.localeCompare(right.id);
}

export function orderedWorkflowStepIds(nodes, edges) {
  const roots = nodes.filter((node) => !node.parentId);
  const rootsById = new Map(roots.map((node) => [node.id, node]));
  const outgoing = new Map(roots.map((node) => [node.id, []]));
  const incoming = new Map(roots.map((node) => [node.id, 0]));

  for (const edge of edges) {
    if (
      edge.source === edge.target
      || !rootsById.has(edge.source)
      || !rootsById.has(edge.target)
    ) {
      continue;
    }
    const targets = outgoing.get(edge.source);
    if (targets.includes(edge.target)) continue;
    targets.push(edge.target);
    incoming.set(edge.target, incoming.get(edge.target) + 1);
  }

  for (const targets of outgoing.values()) {
    targets.sort((leftId, rightId) => (
      compareNodes(rootsById.get(leftId), rootsById.get(rightId))
    ));
  }

  const ordered = [];
  const visited = new Set();
  const visit = (id) => {
    if (visited.has(id)) return;
    visited.add(id);
    ordered.push(id);
    for (const targetId of outgoing.get(id)) visit(targetId);
  };

  roots
    .filter((node) => incoming.get(node.id) === 0)
    .sort(compareNodes)
    .forEach((node) => visit(node.id));
  roots.sort(compareNodes).forEach((node) => visit(node.id));
  return ordered;
}

export function insertWorkflowStep(stepIds, stepId, afterStepId) {
  const next = stepIds.filter((id) => id !== stepId);
  if (afterStepId === null) {
    next.push(stepId);
    return next;
  }
  const afterIndex = next.indexOf(afterStepId);
  next.splice(afterIndex < 0 ? next.length : afterIndex + 1, 0, stepId);
  return next;
}

export function reorderWorkflowStep(stepIds, stepId, targetIndex, pinnedStepId) {
  if (stepId === pinnedStepId || !stepIds.includes(stepId)) return [...stepIds];
  const next = stepIds.filter((id) => id !== stepId);
  const minimumIndex = next[0] === pinnedStepId ? 1 : 0;
  const insertionIndex = Math.max(
    minimumIndex,
    Math.min(Number.isFinite(targetIndex) ? Math.trunc(targetIndex) : next.length, next.length),
  );
  next.splice(insertionIndex, 0, stepId);
  return next;
}

export function workflowSequenceEdges(stepIds) {
  return stepIds.slice(1).map((target, index) => {
    const source = stepIds[index];
    return {
      id: `sequence-${source}-${target}`,
      source,
      target,
    };
  });
}

function conditionEdgeOutcome(edge) {
  const outcome = edge.data?.conditionOutcome;
  return outcome === "true" || outcome === "false" ? outcome : null;
}

function orderedConditionBranchIds(nodes, edges, conditionId, outcome) {
  const nodeIds = new Set(nodes.map((node) => node.id));
  const branchEdges = edges.filter((edge) => (
    edge.data?.conditionId === conditionId
    && conditionEdgeOutcome(edge) === outcome
    && nodeIds.has(edge.source)
    && nodeIds.has(edge.target)
  ));
  const outgoing = new Map();
  for (const edge of branchEdges) {
    if (!outgoing.has(edge.source)) outgoing.set(edge.source, edge.target);
  }
  const ordered = [];
  const visited = new Set([conditionId]);
  let current = conditionId;
  while (outgoing.has(current)) {
    const next = outgoing.get(current);
    if (visited.has(next)) break;
    visited.add(next);
    ordered.push(next);
    current = next;
  }
  const branchNodeIds = new Set();
  for (const edge of branchEdges) {
    if (edge.source !== conditionId) branchNodeIds.add(edge.source);
    if (edge.target !== conditionId) branchNodeIds.add(edge.target);
  }
  nodes
    .filter((node) => branchNodeIds.has(node.id) && !visited.has(node.id))
    .sort(compareNodes)
    .forEach((node) => ordered.push(node.id));
  return ordered;
}

export function workflowConditionEdges(trunkStepIds, conditionId, branches) {
  const edges = workflowSequenceEdges(trunkStepIds);
  for (const outcome of ["true", "false"]) {
    const stepIds = branches[outcome] ?? [];
    const branchSequence = [conditionId, ...stepIds];
    for (let index = 1; index < branchSequence.length; index += 1) {
      const source = branchSequence[index - 1];
      const target = branchSequence[index];
      edges.push({
        id: `condition-${conditionId}-${outcome}-${source}-${target}`,
        source,
        target,
        sourceHandle: source === conditionId ? `condition-${outcome}` : undefined,
        data: {
          conditionId,
          conditionOutcome: outcome,
        },
      });
    }
  }
  return edges;
}

export function normalizeWorkflowConditionBranches(nodes, edges) {
  const persistedConditionEdge = edges.find((edge) => conditionEdgeOutcome(edge));
  if (persistedConditionEdge) {
    const conditionId = persistedConditionEdge.data.conditionId;
    const branches = {
      true: orderedConditionBranchIds(nodes, edges, conditionId, "true"),
      false: orderedConditionBranchIds(nodes, edges, conditionId, "false"),
    };
    const branchNodeIds = new Set([...branches.true, ...branches.false]);
    const trunkNodes = nodes.filter((node) => !node.parentId && !branchNodeIds.has(node.id));
    const trunkEdges = edges.filter((edge) => !conditionEdgeOutcome(edge));
    const trunkStepIds = orderedWorkflowStepIds(trunkNodes, trunkEdges);
    return {
      trunkStepIds,
      conditionId,
      branches,
      migrated: false,
    };
  }

  const linearStepIds = orderedWorkflowStepIds(nodes, edges);
  const conditionIndex = linearStepIds.findIndex((id) => (
    nodes.find((node) => node.id === id)?.data?.kind === "condition"
  ));
  if (conditionIndex < 0) {
    return {
      trunkStepIds: linearStepIds,
      conditionId: null,
      branches: { true: [], false: [] },
      migrated: false,
    };
  }
  return {
    trunkStepIds: linearStepIds.slice(0, conditionIndex + 1),
    conditionId: linearStepIds[conditionIndex],
    branches: {
      true: linearStepIds.slice(conditionIndex + 1),
      false: [],
    },
    migrated: conditionIndex < linearStepIds.length - 1,
  };
}

export function layoutWorkflowSteps(
  nodes,
  stepIds,
  heights = {},
  { top = 0, gap = 0 } = {},
) {
  const positions = new Map();
  let y = top;
  for (const id of stepIds) {
    positions.set(id, { x: 0, y });
    const height = Number.isFinite(heights[id]) ? heights[id] : 0;
    y += height + gap;
  }
  const laidOut = nodes.map((node) => (
    !node.parentId && positions.has(node.id)
      ? { ...node, position: positions.get(node.id) }
      : node
  ));
  const nodesById = new Map(laidOut.map((node) => [node.id, node]));
  const orderedRoots = stepIds
    .map((id) => nodesById.get(id))
    .filter((node) => node && !node.parentId);
  const orderedRootIds = new Set(orderedRoots.map((node) => node.id));
  return [
    ...orderedRoots,
    ...laidOut.filter((node) => !node.parentId && !orderedRootIds.has(node.id)),
    ...laidOut.filter((node) => node.parentId),
  ];
}
