import { useState, type DragEvent } from "react";
import type { TaskStatus } from "../types";
import { STATUS_DETAILS, StatusIcon } from "./BoardColumn";
import { ColumnVisibilityMenu } from "./ColumnVisibilityMenu";
import { LinearIcon } from "./LinearIcon";

interface HiddenColumnsProps {
  statuses: TaskStatus[];
  counts: Record<TaskStatus, number>;
  dropTarget: TaskStatus | null;
  onDragTargetChange: (status: TaskStatus | null) => void;
  onDrop: (status: TaskStatus, taskId: string) => void;
  onShow: (status: TaskStatus) => void;
}

export function HiddenColumns({
  statuses,
  counts,
  dropTarget,
  onDragTargetChange,
  onDrop,
  onShow,
}: HiddenColumnsProps) {
  const [open, setOpen] = useState(true);

  function dropTask(event: DragEvent<HTMLElement>, status: TaskStatus) {
    event.preventDefault();
    const taskId = event.dataTransfer.getData("application/x-taskboard-task")
      || event.dataTransfer.getData("text/plain");
    onDragTargetChange(null);
    if (taskId) onDrop(status, taskId);
  }

  return (
    <section className={`hidden-columns${open ? " is-open" : ""}`} aria-label="隐藏列">
      <button
        type="button"
        className="hidden-columns-heading"
        aria-expanded={open}
        aria-controls="hidden-columns-list"
        onClick={() => setOpen((current) => !current)}
      >
        <LinearIcon name="chevronDown" />
        <span>隐藏列</span>
      </button>

      <div className="hidden-columns-list" id="hidden-columns-list" hidden={!open}>
        {statuses.map((status) => {
          const details = STATUS_DETAILS[status];
          const canShow = counts[status] > 0;
          return (
            <div
              className={`hidden-column-item status-${status}${dropTarget === status ? " is-drop-target" : ""}`}
              key={status}
              onDragEnter={(event) => {
                event.preventDefault();
                onDragTargetChange(status);
              }}
              onDragOver={(event) => {
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                onDragTargetChange(status);
              }}
              onDragLeave={(event) => {
                if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) {
                  onDragTargetChange(null);
                }
              }}
              onDrop={(event) => dropTask(event, status)}
            >
              {canShow ? (
                <button
                  type="button"
                  className="hidden-column-show"
                  aria-label={`显示${details.label}列`}
                  onClick={() => onShow(status)}
                >
                  <span className={`status-icon status-icon-${details.tone}`}>
                    <StatusIcon status={status} />
                  </span>
                  <strong>{details.label}</strong>
                  <span className="hidden-column-count">{counts[status]}</span>
                </button>
              ) : (
                <div className="hidden-column-show is-empty">
                  <span className={`status-icon status-icon-${details.tone}`}>
                    <StatusIcon status={status} />
                  </span>
                  <strong>{details.label}</strong>
                  <span className="hidden-column-count">{counts[status]}</span>
                </div>
              )}
              {canShow && (
                <ColumnVisibilityMenu
                  label={details.label}
                  action="show"
                  className="icon-button hidden-column-menu"
                  onAction={() => onShow(status)}
                />
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
