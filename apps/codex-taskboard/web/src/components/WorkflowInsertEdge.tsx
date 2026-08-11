import {
  BaseEdge,
  EdgeLabelRenderer,
  type Edge,
  type EdgeProps,
} from "@xyflow/react";
import { LinearIcon } from "./LinearIcon";

export interface WorkflowInsertEdgeData extends Record<string, unknown> {
  onInsert?: () => void;
  conditionId?: string;
  conditionOutcome?: "true" | "false";
  branchStart?: boolean;
  points?: Array<{ x: number; y: number }>;
  buttonX?: number;
  buttonY?: number;
  labelX?: number;
  labelY?: number;
}

export type WorkflowSequenceEdge = Edge<WorkflowInsertEdgeData, "workflowInsert">;

export function WorkflowInsertEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  markerEnd,
  data,
}: EdgeProps<WorkflowSequenceEdge>) {
  const points = data?.points?.length
    ? data.points
    : [{ x: sourceX, y: sourceY }, { x: targetX, y: targetY }];
  const path = points
    .map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`)
    .join(" ");
  const labelX = data?.labelX;
  const labelY = data?.labelY;
  const buttonX = data?.buttonX;
  const buttonY = data?.buttonY;

  return (
    <>
      <BaseEdge
        id={id}
        path={path}
        markerEnd={markerEnd}
        className={`workflow-sequence-edge-path${data?.conditionOutcome ? " workflow-condition-branch-edge" : ""}`}
      />
      <EdgeLabelRenderer>
        {data?.branchStart
          && data.conditionOutcome
          && Number.isFinite(labelX)
          && Number.isFinite(labelY) && (
          <span
            className={`workflow-condition-branch-label is-${data.conditionOutcome} nodrag nopan`}
            style={{
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
            }}
          >
            {data.conditionOutcome === "true" ? "成立" : "不成立"}
          </span>
        )}
        {data?.onInsert && Number.isFinite(buttonX) && Number.isFinite(buttonY) && (
          <button
            className="workflow-sequence-add nodrag nopan"
            type="button"
            aria-label="在此处添加步骤"
            title="添加步骤"
            style={{
              transform: `translate(-50%, -50%) translate(${buttonX}px, ${buttonY}px)`,
            }}
            onClick={(event) => {
              event.stopPropagation();
              data.onInsert?.();
            }}
          >
            <LinearIcon name="plus" />
          </button>
        )}
      </EdgeLabelRenderer>
    </>
  );
}
