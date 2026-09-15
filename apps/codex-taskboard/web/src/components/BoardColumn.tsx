import { useEffect, useMemo, useState } from "react";
import type { DragEvent } from "react";
import {
  countTaskDescendants,
  createTaskLookup,
  getAncestorTrail,
  type HierarchyTrailItem,
} from "../taskHierarchy";
import type { Task, TaskStatus } from "../types";
import { ColumnVisibilityMenu } from "./ColumnVisibilityMenu";
import { LinearIcon, LinearStatusIcon } from "./LinearIcon";
import { TaskCard } from "./TaskCard";

export const STATUS_DETAILS: Record<
  TaskStatus,
  { label: string; tone: string }
> = {
  backlog: { label: "积压事项", tone: "backlog" },
  todo: { label: "待办事项", tone: "todo" },
  in_progress: { label: "进行中", tone: "progress" },
  in_review: { label: "审核中", tone: "review" },
  blocked: { label: "已阻塞", tone: "blocked" },
  done: { label: "完成", tone: "done" },
  canceled: { label: "已取消", tone: "canceled" },
};

export function StatusIcon({ status }: { status: TaskStatus }) {
  return <LinearStatusIcon status={status} />;
}

interface BoardColumnProps {
  status: TaskStatus;
  statusIndex: number;
  tasks: Task[];
  allTasks: Task[];
  isDropTarget: boolean;
  draggedTaskId: string | null;
  draggedTaskIds: readonly string[];
  selectedTaskIds: readonly string[];
  draggedTaskHeight: number;
  movingTaskId: string | null;
  settlingTaskId: string | null;
  contextMenuTaskId: string | null;
  onCreate: (status: TaskStatus) => void;
  onEdit: (task: Task) => void;
  onSelectionChange: (task: Task, selected: boolean) => void;
  onContextMenu: (task: Task, position: { x: number; y: number }) => void;
  onMove: (task: Task, status: TaskStatus) => void;
  onDragStart: (task: Task, height: number) => void;
  onDragEnd: () => void;
  onDragEnter: (status: TaskStatus) => void;
  onDrop: (status: TaskStatus, taskId: string, beforeTaskId: string | null) => void;
  onOpenThread: (threadId: string, workspacePath?: string, threadTitle?: string) => void;
  onHide: (status: TaskStatus) => void;
}

interface BoardTaskView {
  task: Task;
  trail: HierarchyTrailItem[];
  depth: number;
  descendantCount: number;
}

interface BoardTaskGroup {
  id: string;
  root: HierarchyTrailItem;
  rootTask: Task | null;
  items: BoardTaskView[];
  order: number;
}

type BoardColumnEntry =
  | { kind: "group"; key: string; group: BoardTaskGroup; order: number }
  | { kind: "task"; key: string; item: BoardTaskView; order: number };

function orderGroupItems(
  items: BoardTaskView[],
  orderById: ReadonlyMap<string, number>,
) {
  const itemById = new Map(items.map((item) => [item.task.id, item]));
  const childrenByParentId = new Map<string, BoardTaskView[]>();
  const roots: BoardTaskView[] = [];

  for (const item of items) {
    const parentId = item.task.relations.parent?.id;
    if (parentId && itemById.has(parentId)) {
      const children = childrenByParentId.get(parentId) ?? [];
      children.push(item);
      childrenByParentId.set(parentId, children);
    } else {
      roots.push(item);
    }
  }

  const compareByColumnOrder = (a: BoardTaskView, b: BoardTaskView) => (
    (orderById.get(a.task.id) ?? 0) - (orderById.get(b.task.id) ?? 0)
  );
  roots.sort(compareByColumnOrder);
  for (const children of childrenByParentId.values()) children.sort(compareByColumnOrder);

  const ordered: BoardTaskView[] = [];
  const visited = new Set<string>();

  function visit(item: BoardTaskView) {
    if (visited.has(item.task.id)) return;
    visited.add(item.task.id);
    ordered.push(item);
    for (const child of childrenByParentId.get(item.task.id) ?? []) visit(child);
  }

  for (const root of roots) visit(root);
  for (const item of items.sort(compareByColumnOrder)) visit(item);

  return ordered;
}

function buildColumnEntries(tasks: Task[], allTasks: Task[]) {
  const taskById = createTaskLookup(allTasks);
  const orderById = new Map(tasks.map((task, index) => [task.id, index]));
  const groups = new Map<string, BoardTaskGroup>();
  const entries: BoardColumnEntry[] = [];

  for (const task of tasks) {
    const trail = getAncestorTrail(task, taskById);
    const descendantCount = countTaskDescendants(task, taskById);
    const item: BoardTaskView = {
      task,
      trail,
      depth: trail.length,
      descendantCount,
    };
    const hasHierarchy = trail.length > 0 || descendantCount > 0 || task.relations.parent !== null;

    if (!hasHierarchy) {
      entries.push({ kind: "task", key: task.id, item, order: orderById.get(task.id) ?? 0 });
      continue;
    }

    const root = trail[0] ?? task;
    const group = groups.get(root.id) ?? {
      id: root.id,
      root,
      rootTask: taskById.get(root.id) ?? null,
      items: [],
      order: orderById.get(task.id) ?? 0,
    };
    group.items.push(item);
    group.order = Math.min(group.order, orderById.get(task.id) ?? group.order);
    groups.set(root.id, group);
  }

  for (const group of groups.values()) {
    group.items = orderGroupItems(group.items, orderById);
    entries.push({ kind: "group", key: `group-${group.id}`, group, order: group.order });
  }

  entries.sort((a, b) => a.order - b.order);
  const orderedTasks = entries.flatMap((entry) => (
    entry.kind === "group"
      ? entry.group.items.map((item) => item.task)
      : [entry.item.task]
  ));

  return { entries, orderedTasks };
}

export function BoardColumn({
  status,
  statusIndex,
  tasks,
  allTasks,
  isDropTarget,
  draggedTaskId,
  draggedTaskIds,
  selectedTaskIds,
  draggedTaskHeight,
  movingTaskId,
  settlingTaskId,
  contextMenuTaskId,
  onCreate,
  onEdit,
  onSelectionChange,
  onContextMenu,
  onMove,
  onDragStart,
  onDragEnd,
  onDragEnter,
  onDrop,
  onOpenThread,
  onHide,
}: BoardColumnProps) {
  const details = STATUS_DETAILS[status];
  const [dropBeforeTaskId, setDropBeforeTaskId] = useState<string | null | undefined>();
  const { entries, orderedTasks } = useMemo(
    () => buildColumnEntries(tasks, allTasks),
    [allTasks, tasks],
  );
  const activeDraggedTaskIds = draggedTaskIds.length > 0 ? draggedTaskIds : draggedTaskId ? [draggedTaskId] : [];
  const draggedTaskIdSet = new Set(activeDraggedTaskIds);
  const selectedTaskIdSet = new Set(selectedTaskIds);
  const taskIndexes = new Map(orderedTasks.map((task, index) => [task.id, index]));
  const remainingTasks = orderedTasks.filter((task) => !draggedTaskIdSet.has(task.id));
  const remainingIndexes = new Map(remainingTasks.map((task, index) => [task.id, index]));
  const beforeIndex = dropBeforeTaskId
    ? remainingIndexes.get(dropBeforeTaskId) ?? remainingTasks.length
    : remainingTasks.length;
  const previewIndex = isDropTarget && dropBeforeTaskId !== undefined ? beforeIndex : -1;
  const dragDistance = draggedTaskHeight + 8;
  const draggedGroupSize = Math.max(1, activeDraggedTaskIds.length);

  useEffect(() => {
    if (!isDropTarget || !draggedTaskId) setDropBeforeTaskId(undefined);
  }, [draggedTaskId, isDropTarget]);

  function findDropBefore(container: HTMLElement, clientY: number): string | null {
    const cards = Array.from(container.querySelectorAll<HTMLElement>("[data-task-id]"))
      .filter((card) => !draggedTaskIdSet.has(card.dataset.taskId ?? ""));
    return cards.find((card) => clientY < card.getBoundingClientRect().top + card.offsetHeight / 2)
      ?.dataset.taskId ?? null;
  }

  function handleDrop(event: DragEvent<HTMLElement>) {
    event.preventDefault();
    const taskId =
      event.dataTransfer.getData("application/x-taskboard-task") ||
      event.dataTransfer.getData("text/plain");
    if (taskId) onDrop(status, taskId, findDropBefore(event.currentTarget, event.clientY));
    setDropBeforeTaskId(undefined);
  }

  function getTaskDragShift(task: Task): number {
    if (!draggedTaskId || draggedTaskIdSet.has(task.id)) return 0;
    let shift = 0;
    const taskIndex = taskIndexes.get(task.id) ?? -1;
    const remainingIndex = remainingIndexes.get(task.id) ?? -1;
    const removedBeforeCount = taskIndex < 0
      ? 0
      : orderedTasks.slice(0, taskIndex).filter((candidate) => draggedTaskIdSet.has(candidate.id)).length;

    for (let index = 0; index < removedBeforeCount; index += 1) shift -= dragDistance;
    if (previewIndex >= 0 && remainingIndex >= previewIndex) {
      for (let index = 0; index < draggedGroupSize; index += 1) shift += dragDistance;
    }
    return shift;
  }

  function renderTaskCard(item: BoardTaskView) {
    const task = item.task;
    const dragShift = getTaskDragShift(task);
    return (
      <TaskCard
        key={task.id}
        task={task}
        statusIndex={statusIndex}
        hierarchyDepth={item.depth}
        parentTrail={item.trail}
        descendantCount={item.descendantCount}
        isDragging={draggedTaskIdSet.has(task.id)}
        isSelected={selectedTaskIdSet.has(task.id)}
        dragShift={dragShift}
        isMoving={movingTaskId === task.id}
        isSettling={settlingTaskId === task.id}
        isContextMenuOpen={contextMenuTaskId === task.id}
        onEdit={onEdit}
        onSelectionChange={onSelectionChange}
        onContextMenu={onContextMenu}
        onMove={onMove}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
        onOpenThread={onOpenThread}
      />
    );
  }

  return (
    <section
      className={`board-column status-${status}${isDropTarget ? " is-drop-target" : ""}`}
      aria-labelledby={`column-${status}`}
      onDragEnter={() => onDragEnter(status)}
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        onDragEnter(status);
        setDropBeforeTaskId(findDropBefore(event.currentTarget, event.clientY));
      }}
      onDragLeave={(event) => {
        if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) {
          setDropBeforeTaskId(undefined);
        }
      }}
      onDrop={handleDrop}
    >
      <header className="column-header">
        <div className="column-heading">
          <span className={`status-icon status-icon-${details.tone}`}>
            <StatusIcon status={status} />
          </span>
          <h2 id={`column-${status}`}>{details.label}</h2>
          <span className="task-count" aria-label={`${tasks.length} 个任务`}>{tasks.length}</span>
        </div>
        <div className="column-actions">
          {tasks.length > 0 && (
            <ColumnVisibilityMenu
              label={details.label}
              action="hide"
              className="icon-button column-menu"
              onAction={() => onHide(status)}
            />
          )}
          <button
            type="button"
            className="icon-button add-task-button"
            onClick={() => onCreate(status)}
            aria-label={`在${details.label}中新建任务`}
            title={`添加到${details.label}`}
          >
            <LinearIcon name="plus" />
          </button>
        </div>
      </header>

      <div className={`column-list${entries.some((entry) => entry.kind === "group") ? " has-hierarchy-groups" : ""}`}>
        {entries.map((entry) => {
          if (entry.kind === "task") return renderTaskCard(entry.item);
          const { group } = entry;
          const maxDepth = Math.max(...group.items.map((item) => item.depth), 0);
          const groupRoot = (
            <>
              <span className="board-task-group-icon" aria-hidden="true">
                <LinearIcon name="branch" />
              </span>
              <span className="board-task-group-id">{group.root.identifier}</span>
              <span className="board-task-group-title" title={group.root.title}>{group.root.title}</span>
              <span className="board-task-group-root-status" title={STATUS_DETAILS[group.root.status].label}>
                <LinearStatusIcon status={group.root.status} />
              </span>
            </>
          );
          return (
            <section className="board-task-group" key={entry.key} aria-label={`${group.root.identifier} 子任务`}>
              <header className="board-task-group-header">
                {group.rootTask ? (
                  <button
                    className="board-task-group-root"
                    type="button"
                    onClick={() => onEdit(group.rootTask!)}
                  >
                    {groupRoot}
                  </button>
                ) : (
                  <div className="board-task-group-root">{groupRoot}</div>
                )}
                <span className="board-task-group-count">{group.items.length}</span>
                {maxDepth > 1 && <span className="board-task-group-depth">{maxDepth + 1}层</span>}
              </header>
              <div className="board-task-group-list">
                {group.items.map(renderTaskCard)}
              </div>
            </section>
          );
        })}
      </div>
    </section>
  );
}
