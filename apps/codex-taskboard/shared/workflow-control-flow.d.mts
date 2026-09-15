export type WorkflowConditionOutcome = "true" | "false";

export const WORKFLOW_TRIGGER_KINDS: readonly [
  "issue-trigger",
  "rss-trigger",
  "pull-request-submitted-trigger",
  "repository-issue-submitted-trigger",
  "git-status-trigger",
];
export function isWorkflowTriggerKind(kind: string): boolean;

export interface WorkflowSequenceRefSegment {
  conditionId: string;
  outcome: WorkflowConditionOutcome;
}

export type WorkflowSequenceRef = WorkflowSequenceRefSegment[];

export interface WorkflowStepItem {
  type: "step";
  nodeId: string;
}

export interface WorkflowConditionItem {
  type: "condition";
  nodeId: string;
  branches: {
    true: WorkflowSequence;
    false: WorkflowSequence;
  };
}

export type WorkflowItem = WorkflowStepItem | WorkflowConditionItem;

export interface WorkflowSequence {
  items: WorkflowItem[];
}

export interface WorkflowFlow {
  version: 2;
  root: WorkflowSequence;
}

export interface WorkflowControlNode {
  id: string;
  parentId?: string;
  position?: { x: number; y: number };
  measured?: { width?: number; height?: number };
  style?: { width?: number | string; height?: number | string };
  data?: { kind?: string; acceptsChildren?: boolean };
}

export interface WorkflowInsertion {
  sequenceRef: WorkflowSequenceRef;
  index: number;
}

export interface WorkflowDerivedEdge {
  id: string;
  source: string;
  target: string;
  type: "workflowInsert";
  data: {
    points: Array<{ x: number; y: number }>;
    insertion?: WorkflowInsertion;
    buttonX?: number;
    buttonY?: number;
    conditionId?: string;
    conditionOutcome?: WorkflowConditionOutcome;
    branchStart?: boolean;
    labelX?: number;
    labelY?: number;
  };
}

export function createWorkflowFlow(stepIds?: string[]): WorkflowFlow;
export function workflowNodeIds(flow: WorkflowFlow): string[];
export function getWorkflowSequence(
  flow: WorkflowFlow,
  sequenceRef: WorkflowSequenceRef,
): WorkflowSequence;
export function findWorkflowItem(
  flow: WorkflowFlow,
  nodeId: string,
): { sequenceRef: WorkflowSequenceRef; index: number; item: WorkflowItem } | null;
export function assertWorkflowFlow<T extends WorkflowControlNode>(
  flow: WorkflowFlow,
  nodes: T[],
): WorkflowFlow;
export function migrateWorkflowSnapshotV1<T extends WorkflowControlNode>(
  nodes: T[],
  edges: unknown[],
  selectedNodeId?: string | null,
): { nodes: T[]; flow: WorkflowFlow; selectedNodeId: string | null };
export function normalizeWorkflowSnapshot<T extends WorkflowControlNode>(
  snapshot: {
    nodes: T[];
    flow?: WorkflowFlow;
    edges?: unknown[];
    selectedNodeId?: string | null;
  },
): { nodes: T[]; flow: WorkflowFlow; selectedNodeId: string | null };
export function serializeWorkflowSnapshot<T extends WorkflowControlNode>(
  nodes: T[],
  flow: WorkflowFlow,
  selectedNodeId?: string | null,
): { nodes: T[]; flow: WorkflowFlow; selectedNodeId: string | null };
export function insertWorkflowNode(
  flow: WorkflowFlow,
  sequenceRef: WorkflowSequenceRef,
  index: number,
  nodeId: string,
  kind: string,
): WorkflowFlow;
export function deleteWorkflowNode(
  flow: WorkflowFlow,
  nodeId: string,
): { flow: WorkflowFlow; removedNodeIds: string[] };
export function moveWorkflowNode(
  flow: WorkflowFlow,
  nodeId: string,
  targetSequenceRef: WorkflowSequenceRef,
  targetIndex: number,
): WorkflowFlow;
export function deriveWorkflowLayout<T extends WorkflowControlNode>(
  flow: WorkflowFlow,
  nodes: T[],
): {
  positions: Record<string, { x: number; y: number }>;
  conditions: Record<string, {
    nodeBottom: number;
    splitY: number;
    mergeRailY: number;
    mergeY: number;
    mergeX: number;
    branchCenters: Record<WorkflowConditionOutcome, number>;
    branchBounds: Record<WorkflowConditionOutcome, {
      left: number;
      right: number;
      top: number;
      bottom: number;
    }>;
    bounds: { left: number; right: number; top: number; bottom: number };
  }>;
  virtualNodes: Array<{
    id: string;
    kind: string;
    position: { x: number; y: number };
  }>;
  edges: WorkflowDerivedEdge[];
  insertionPoints: Array<{
    id: string;
    edgeId: string | null;
    x: number;
    y: number;
    insertion: WorkflowInsertion;
  }>;
  bounds: { left: number; right: number; top: number; bottom: number };
};
