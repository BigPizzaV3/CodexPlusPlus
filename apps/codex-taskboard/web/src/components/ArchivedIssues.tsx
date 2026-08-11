import type { Task, TaskPriority } from "../types";
import { STATUS_DETAILS } from "./BoardColumn";
import { LinearIcon, LinearPriorityIcon, LinearStatusIcon } from "./LinearIcon";

const PRIORITY_LABELS: Record<TaskPriority, string> = {
  none: "无优先级",
  urgent: "紧急",
  high: "高优先级",
  medium: "中优先级",
  low: "低优先级",
};

const archiveDateFormatter = new Intl.DateTimeFormat("zh-CN", {
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

interface ArchivedIssuesProps {
  tasks: Task[];
  totalTasks: number;
  hasQuery: boolean;
  restoringTaskId: string | null;
  deletingTaskId: string | null;
  onClearQuery: () => void;
  onOpenTask: (task: Task) => void;
  onRestore: (task: Task) => void;
  onRequestDelete: (task: Task) => void;
}

function archivedAtLabel(task: Task) {
  return task.archivedAt
    ? archiveDateFormatter.format(new Date(task.archivedAt))
    : "未知时间";
}

export function ArchivedIssues({
  tasks,
  totalTasks,
  hasQuery,
  restoringTaskId,
  deletingTaskId,
  onClearQuery,
  onOpenTask,
  onRestore,
  onRequestDelete,
}: ArchivedIssuesProps) {
  if (totalTasks === 0) {
    return (
      <section className="page-empty archive-empty">
        <span className="empty-search" aria-hidden="true"><LinearIcon name="folder" /></span>
        <h2>没有已归档任务</h2>
        <p>归档任务会在这里集中显示，可以恢复，也可以永久删除。</p>
      </section>
    );
  }

  if (tasks.length === 0 && hasQuery) {
    return (
      <section className="page-empty archive-empty">
        <span className="empty-search" aria-hidden="true"><LinearIcon name="search" /></span>
        <h2>没有匹配的归档任务</h2>
        <p>请更换搜索词，或移除一个筛选条件。</p>
        <button className="button secondary" type="button" onClick={onClearQuery}>
          清除筛选
        </button>
      </section>
    );
  }

  return (
    <section className="archive-view" aria-label="已归档任务">
      <div className="archive-view-heading">
        <div>
          <span>归档箱</span>
          <h2>已归档任务</h2>
        </div>
        <strong>{tasks.length}/{totalTasks}</strong>
      </div>
      <div className="archive-list">
        {tasks.map((task) => (
          <article className="archived-issue-row" key={task.id}>
            <div className="archived-issue-main">
              <button
                className="archived-issue-title"
                type="button"
                onClick={() => onOpenTask(task)}
              >
                <span>{task.identifier}</span>
                {task.title}
              </button>
              <div className="archived-issue-meta" aria-label="任务属性">
                <span className={`archive-status-chip status-${task.status}`}>
                  <LinearStatusIcon status={task.status} />
                  {STATUS_DETAILS[task.status].label}
                </span>
                <span className={`archive-priority-chip priority-${task.priority}`} title={PRIORITY_LABELS[task.priority]}>
                  <LinearPriorityIcon priority={task.priority} />
                  {PRIORITY_LABELS[task.priority]}
                </span>
                <span>归档于 {archivedAtLabel(task)}</span>
              </div>
            </div>
            <div className="archived-issue-actions">
              <button
                className="button secondary"
                type="button"
                disabled={restoringTaskId === task.id || deletingTaskId === task.id}
                onClick={() => onRestore(task)}
              >
                <LinearIcon name="recurrence" />
                {restoringTaskId === task.id ? "恢复中..." : "恢复"}
              </button>
              <button
                className="button danger"
                type="button"
                disabled={restoringTaskId === task.id || deletingTaskId === task.id}
                onClick={() => onRequestDelete(task)}
              >
                <LinearIcon name="trash" />
                永久删除
              </button>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
