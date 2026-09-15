import type { CSSProperties } from "react";

import {
  countTaskDescendants,
  createTaskLookup,
  getAncestorTrail,
} from "../taskHierarchy";
import type { Task } from "../types";
import { ActorAvatar } from "./ActorAvatar";
import { STATUS_DETAILS } from "./BoardColumn";
import { LinearIcon, LinearStatusIcon } from "./LinearIcon";

interface IssueHierarchyViewProps {
  tasks: Task[];
  allTasks: Task[];
  totalTasks: number;
  hasQuery: boolean;
  onClearQuery: () => void;
  onOpenTask: (task: Task) => void;
  onOpenThread: (threadId: string, workspacePath?: string, threadTitle?: string) => void;
}

interface HierarchyRow {
  task: Task;
  depth: number;
  isLastSibling: boolean;
  isContext: boolean;
  descendantCount: number;
  directSubIssueCount: number;
  parentPath: string;
}

function buildHierarchyRows(visibleTasks: Task[], allTasks: Task[]) {
  const sourceTasks = allTasks.length > 0 ? allTasks : visibleTasks;
  const taskById = createTaskLookup(sourceTasks);
  const visibleTaskIds = new Set(visibleTasks.map((task) => task.id));
  const displayTaskIds = new Set(visibleTaskIds);

  for (const task of visibleTasks) {
    for (const ancestor of getAncestorTrail(task, taskById)) {
      if (taskById.has(ancestor.id)) displayTaskIds.add(ancestor.id);
    }
  }

  const displayTasks = sourceTasks.filter((task) => displayTaskIds.has(task.id));
  const orderById = new Map(sourceTasks.map((task, index) => [task.id, index]));
  const childrenByParentId = new Map<string, Task[]>();
  const roots: Task[] = [];

  for (const task of displayTasks) {
    const parentId = task.relations.parent?.id;
    if (parentId && displayTaskIds.has(parentId)) {
      const children = childrenByParentId.get(parentId) ?? [];
      children.push(task);
      childrenByParentId.set(parentId, children);
    } else {
      roots.push(task);
    }
  }

  const compareBySourceOrder = (a: Task, b: Task) => (
    (orderById.get(a.id) ?? 0) - (orderById.get(b.id) ?? 0)
  );
  roots.sort(compareBySourceOrder);
  for (const children of childrenByParentId.values()) children.sort(compareBySourceOrder);

  const rows: HierarchyRow[] = [];
  const visited = new Set<string>();

  function visit(task: Task, depth: number, ancestors: string[], isLastSibling: boolean) {
    if (visited.has(task.id)) return;
    visited.add(task.id);
    rows.push({
      task,
      depth,
      isLastSibling,
      isContext: !visibleTaskIds.has(task.id),
      descendantCount: countTaskDescendants(task, taskById),
      directSubIssueCount: task.relations.subIssues.length,
      parentPath: ancestors.join(" / "),
    });
    const children = childrenByParentId.get(task.id) ?? [];
    children.forEach((child, index) => {
      visit(child, depth + 1, [...ancestors, task.identifier], index === children.length - 1);
    });
  }

  roots.forEach((root, index) => visit(root, 0, [], index === roots.length - 1));
  for (const task of displayTasks.sort(compareBySourceOrder)) visit(task, 0, [], true);

  return rows;
}

function subIssueLabel(row: HierarchyRow) {
  if (row.descendantCount === 0) return "无";
  if (row.descendantCount === row.directSubIssueCount) return `${row.directSubIssueCount}`;
  return `${row.directSubIssueCount}/${row.descendantCount}`;
}

export function IssueHierarchyView({
  tasks,
  allTasks,
  totalTasks,
  hasQuery,
  onClearQuery,
  onOpenTask,
  onOpenThread,
}: IssueHierarchyViewProps) {
  if (totalTasks === 0) {
    return (
      <section className="page-empty hierarchy-empty">
        <span className="empty-search" aria-hidden="true"><LinearIcon name="branch" /></span>
        <h2>还没有任务</h2>
        <p>有父子关系的任务会在这里以层级树展示。</p>
      </section>
    );
  }

  if (tasks.length === 0 && hasQuery) {
    return (
      <section className="page-empty hierarchy-empty">
        <span className="empty-search" aria-hidden="true"><LinearIcon name="search" /></span>
        <h2>没有匹配的任务</h2>
        <p>请更换搜索词，或移除一个筛选条件。</p>
        <button className="button secondary" type="button" onClick={onClearQuery}>
          清除筛选
        </button>
      </section>
    );
  }

  const rows = buildHierarchyRows(tasks, allTasks);

  return (
    <section className="hierarchy-view" aria-label="父子任务层级">
      <div className="hierarchy-view-heading">
        <div>
          <span>层级视图</span>
          <h2>父子任务</h2>
        </div>
        <strong>{tasks.length}/{totalTasks}</strong>
      </div>

      <div className="hierarchy-table" role="tree" aria-label="父子任务树">
        <div className="hierarchy-table-header" aria-hidden="true">
          <span>任务</span>
          <span>状态</span>
          <span>下级</span>
          <span>负责人</span>
        </div>
        {rows.map((row) => {
          const indent = Math.min(row.depth, 8) * 18;
          const rowStyle = { "--hierarchy-indent": `${indent}px` } as CSSProperties;
          return (
            <div
              className={`hierarchy-row${row.isContext ? " is-context" : ""}${row.isLastSibling ? " is-last-sibling" : ""}`}
              key={row.task.id}
              role="treeitem"
              aria-level={row.depth + 1}
              data-depth={row.depth}
              style={rowStyle}
            >
              <button
                className="hierarchy-row-main"
                type="button"
                title={row.parentPath ? `${row.parentPath} / ${row.task.identifier} ${row.task.title}` : `${row.task.identifier} ${row.task.title}`}
                onClick={() => onOpenTask(row.task)}
              >
                <span className="hierarchy-row-indent" aria-hidden="true" />
                <span className="hierarchy-row-id">{row.task.identifier}</span>
                <span className="hierarchy-row-title">{row.task.title}</span>
                {row.isContext && <span className="hierarchy-context-chip">上级</span>}
              </button>
              <span className="hierarchy-row-status">
                <LinearStatusIcon status={row.task.status} />
                {STATUS_DETAILS[row.task.status].label}
              </span>
              <span
                className="hierarchy-row-subissues"
                title={row.descendantCount > row.directSubIssueCount ? "直接下级/全部下级" : "直接下级"}
              >
                {subIssueLabel(row)}
              </span>
              <span className="hierarchy-row-assignee">
                <ActorAvatar actor={row.task.assignee} />
                <span>{row.task.assignee.name}</span>
              </span>
              {row.task.threadId && (
                <button
                  className="icon-button hierarchy-row-thread"
                  type="button"
                  aria-label={`查看对话 ${row.task.threadId}`}
                  title={`查看对话 ${row.task.threadId}`}
                  onClick={() => onOpenThread(row.task.threadId!, undefined, row.task.title)}
                >
                  <LinearIcon name="conversation" />
                </button>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
