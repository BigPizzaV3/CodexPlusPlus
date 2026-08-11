export interface WorkflowSequenceNode {
  id: string;
  parentId?: string;
  position: { x: number; y: number };
  data?: { kind?: string };
}

export interface WorkflowSequenceEdge {
  id: string;
  source: string;
  target: string;
  sourceHandle?: string | null;
  data?: {
    conditionId?: string;
    conditionOutcome?: "true" | "false";
  };
}

export interface WorkflowConditionGraph {
  trunkStepIds: string[];
  conditionId: string | null;
  branches: {
    true: string[];
    false: string[];
  };
  migrated: boolean;
}

export function orderedWorkflowStepIds(
  nodes: WorkflowSequenceNode[],
  edges: WorkflowSequenceEdge[],
): string[];

export function insertWorkflowStep(
  stepIds: string[],
  stepId: string,
  afterStepId: string | null,
): string[];

export function reorderWorkflowStep(
  stepIds: string[],
  stepId: string,
  targetIndex: number,
  pinnedStepId?: string,
): string[];

export function workflowSequenceEdges(stepIds: string[]): WorkflowSequenceEdge[];

export function workflowConditionEdges(
  trunkStepIds: string[],
  conditionId: string,
  branches: WorkflowConditionGraph["branches"],
): WorkflowSequenceEdge[];

export function normalizeWorkflowConditionBranches(
  nodes: WorkflowSequenceNode[],
  edges: WorkflowSequenceEdge[],
): WorkflowConditionGraph;

export function layoutWorkflowSteps<T extends WorkflowSequenceNode>(
  nodes: T[],
  stepIds: string[],
  heights?: Record<string, number>,
  options?: { top?: number; gap?: number },
): T[];
