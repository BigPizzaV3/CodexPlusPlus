import type { CSSProperties, MouseEvent } from "react";
import type { HierarchyTrailItem } from "../taskHierarchy";
import { TASK_STATUSES, type Task, type TaskPriority, type TaskStatus } from "../types";
import { ActorAvatar } from "./ActorAvatar";
import { LinearIcon, LinearPriorityIcon } from "./LinearIcon";

const PRIORITY_LABELS: Record<TaskPriority, string> = {
  none: "无优先级",
  urgent: "紧急",
  high: "高优先级",
  medium: "中优先级",
  low: "低优先级",
};

interface TaskCardProps {
  task: Task;
  statusIndex: number;
  hierarchyDepth?: number;
  parentTrail?: readonly HierarchyTrailItem[];
  descendantCount?: number;
  isDragging: boolean;
  isSelected: boolean;
  dragShift: number;
  isMoving: boolean;
  isSettling: boolean;
  isContextMenuOpen: boolean;
  onEdit: (task: Task) => void;
  onSelectionChange: (task: Task, selected: boolean) => void;
  onContextMenu: (task: Task, position: { x: number; y: number }) => void;
  onMove: (task: Task, status: TaskStatus) => void;
  onDragStart: (task: Task, height: number) => void;
  onDragEnd: () => void;
  onOpenThread: (threadId: string, workspacePath?: string, threadTitle?: string) => void;
}

export function TaskCard({
  task,
  statusIndex,
  hierarchyDepth = 0,
  parentTrail = [],
  descendantCount = 0,
  isDragging,
  isSelected,
  dragShift,
  isMoving,
  isSettling,
  isContextMenuOpen,
  onEdit,
  onSelectionChange,
  onContextMenu,
  onMove,
  onDragStart,
  onDragEnd,
  onOpenThread,
}: TaskCardProps) {
  const dueDate = task.dueDate
    ? new Intl.DateTimeFormat("zh-CN", { month: "short", day: "numeric" }).format(new Date(`${task.dueDate}T12:00:00`))
    : null;
  const subIssueTotal = task.relations.subIssues.length;
  const completedSubIssues = task.relations.subIssues.filter((issue) => issue.status === "done").length;
  const activeBlockers = task.relations.blockedBy.filter((issue) => (
    issue.status !== "done" && issue.status !== "canceled"
  )).length;
  const visualHierarchyDepth = Math.min(Math.max(hierarchyDepth, 0), 4);
  const parentTrailLabel = parentTrail.map((item) => item.identifier).join(" / ");
  const parentTrailTitle = parentTrail.map((item) => `${item.identifier} ${item.title}`).join(" / ");
  const cardStyle = {
    transform: dragShift ? `translate3d(0, ${dragShift}px, 0)` : undefined,
    "--hierarchy-indent": visualHierarchyDepth > 0 ? `${visualHierarchyDepth * 10}px` : undefined,
  } as CSSProperties;

  function stopThen(callback: () => void) {
    return (event: MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      callback();
    };
  }

  return (
    <article
      className={`task-card priority-${task.priority}${visualHierarchyDepth > 0 ? " is-hierarchy-child" : ""}${isSelected ? " is-selected" : ""}${isDragging ? " is-dragging" : ""}${dragShift ? " is-drag-shifted" : ""}${isMoving ? " is-moving" : ""}${isSettling ? " is-settling" : ""}${isContextMenuOpen ? " is-context-open" : ""}`}
      style={cardStyle}
      draggable={!isMoving}
      aria-labelledby={`task-${task.id}-title`}
      aria-selected={isSelected}
      data-task-id={task.id}
      data-drag-shift={dragShift || undefined}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onContextMenu(task, { x: event.clientX, y: event.clientY });
      }}
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", task.id);
        event.dataTransfer.setData("application/x-taskboard-task", task.id);
        onDragStart(task, event.currentTarget.offsetHeight);
      }}
      onDragEnd={onDragEnd}
    >
      <button
        className="task-card-open"
        type="button"
        aria-label={`打开 ${task.identifier}: ${task.title}`}
        onClick={(event) => {
          if (event.metaKey || event.ctrlKey) {
            onSelectionChange(task, !isSelected);
            return;
          }
          onEdit(task);
        }}
      />

      <div className="card-topline">
        <span className="card-reference">
          <span className="task-identifier">{task.identifier}</span>
          {task.relations.parent && (
            <>
              <LinearIcon name="chevronRight" />
              <span className="card-parent-title" title={task.relations.parent.title}>
                {task.relations.parent.title}
              </span>
            </>
          )}
        </span>
        <ActorAvatar actor={task.assignee} className="card-assignee-avatar" />
        <div className="card-actions" aria-label="移动任务">
          <button
            className="icon-button compact"
            type="button"
            disabled={statusIndex === 0 || isMoving}
            aria-label="移到上一状态"
            title="移到上一状态"
            onClick={stopThen(() => onMove(task, TASK_STATUSES[statusIndex - 1]))}
          >
            <LinearIcon name="chevronLeft" />
          </button>
          <button
            className="icon-button compact"
            type="button"
            disabled={statusIndex === TASK_STATUSES.length - 1 || isMoving}
            aria-label="移到下一状态"
            title="移到下一状态"
            onClick={stopThen(() => onMove(task, TASK_STATUSES[statusIndex + 1]))}
          >
            <LinearIcon name="chevronRight" />
          </button>
        </div>
      </div>

      <h3 id={`task-${task.id}-title`}>{task.title}</h3>

      {parentTrail.length > 0 && (
        <div className="card-hierarchy-trail" title={parentTrailTitle}>
          <LinearIcon name="branch" />
          <span>上级 {parentTrailLabel}</span>
        </div>
      )}

      <div className="card-properties" aria-label="任务属性">
        <span className={`priority-icon priority-icon-${task.priority}`} title={PRIORITY_LABELS[task.priority]}>
          <LinearPriorityIcon priority={task.priority} />
        </span>
        {activeBlockers > 0 && (
          <span className="blocked-by-count" title={`被 ${activeBlockers} 个未完成任务阻塞`}>
            <LinearIcon name="alert" />
            {activeBlockers}
          </span>
        )}
        {subIssueTotal > 0 && (
          <span className="sub-issue-progress-chip" title={`${completedSubIssues}/${subIssueTotal} 个子任务已完成`}>
            <span className="sub-issue-progress" aria-hidden="true" />
            {completedSubIssues}/{subIssueTotal}
          </span>
        )}
        {descendantCount > subIssueTotal && (
          <span className="sub-issue-depth-chip" title={`共 ${descendantCount} 个下级任务`}>
            <LinearIcon name="branch" />
            {descendantCount}
          </span>
        )}
        {task.labels.slice(0, 2).map((label) => (
          <span className="label-chip" key={label}>{label}</span>
        ))}
        {task.labels.length > 2 && (
          <span className="label-more" title={task.labels.slice(2).join(", ")}>+{task.labels.length - 2}</span>
        )}
        {dueDate && (
          <span className="due-date-chip" title={`截止日期 ${task.dueDate}`}>
            <LinearIcon name="calendar" /> {dueDate}
          </span>
        )}
        {task.threadId && (
          <button
            className="thread-link"
            type="button"
            aria-label={`查看对话 ${task.threadId}`}
            title={`查看对话 ${task.threadId}`}
            onClick={stopThen(() => onOpenThread(task.threadId!, undefined, task.title))}
          >
            <LinearIcon name="conversation" />
          </button>
        )}
      </div>
    </article>
  );
}
