/**
 * 任务面板的纯数据部分：状态表、按列分组、搜索。和界面分开，方便用 node --test 直接测。
 * 状态的取值与 core `task_board::TaskStatus` 一致（snake_case）。
 */

export type TaskStatus = "backlog" | "todo" | "in_progress" | "in_review" | "blocked" | "done" | "cancelled";

export type Task = {
  id: number;
  title: string;
  status: TaskStatus;
  project?: string | null;
  body: string;
  createdAt: number;
  updatedAt: number;
  statusAt: number;
};

export type TaskBoard = { dir: string; tasks: Task[]; created?: number };

/** 看板列的顺序。`label` 是中文原文，界面用 t() 翻译。 */
export const TASK_STATUSES: ReadonlyArray<{ id: TaskStatus; label: string; hint: string; tone: "off" | "neutral" | "info" | "warn" | "ok" | "danger" }> = [
  { id: "backlog", label: "收集", hint: "刚记下，还没决定要不要做", tone: "off" },
  { id: "todo", label: "待办", hint: "要做，还没开始", tone: "neutral" },
  { id: "in_progress", label: "进行中", hint: "正在做", tone: "info" },
  { id: "in_review", label: "待验收", hint: "做完了，等你确认", tone: "warn" },
  { id: "blocked", label: "卡住", hint: "暂时没法继续", tone: "danger" },
  { id: "done", label: "完成", hint: "已确认完成", tone: "ok" },
  { id: "cancelled", label: "已取消", hint: "不做了，记录保留", tone: "off" },
];

export const statusLabel = (status: TaskStatus) => TASK_STATUSES.find((entry) => entry.id === status)?.label ?? status;

/** 已经了结的状态：不计入状态栏的「未了结」。 */
export const CLOSED_STATUSES: ReadonlySet<TaskStatus> = new Set(["done", "cancelled"]);

/** 对外编号：`T-12`。 */
export const taskKey = (id: number) => `T-${id}`;

/** 项目只显示最后一级目录名。 */
export function projectName(project?: string | null): string {
  if (!project) return "";
  const parts = project.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? project;
}

/** 搜索：编号（T-12 / 12）、标题、正文、项目，忽略大小写，空格分开的词都要命中。 */
export function matchesQuery(task: Task, query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const haystack = [taskKey(task.id), String(task.id), task.title, task.body, task.project ?? ""].join("\n").toLowerCase();
  return words.every((word) => haystack.includes(word));
}

/** 按状态分列；每列里最近改过状态的在前（刚拖进来的在最上面）。 */
export function groupByStatus(tasks: Task[]): Record<TaskStatus, Task[]> {
  const groups = Object.fromEntries(TASK_STATUSES.map((entry) => [entry.id, [] as Task[]])) as Record<TaskStatus, Task[]>;
  for (const task of tasks) (groups[task.status] ?? groups.backlog).push(task);
  for (const list of Object.values(groups)) list.sort((a, b) => b.statusAt - a.statusAt || b.id - a.id);
  return groups;
}

/** 底部状态栏的计数：还没了结的总数，以及进行中 / 待验收 / 卡住各几个。 */
export function boardSummary(tasks: Task[]) {
  const count = (status: TaskStatus) => tasks.filter((task) => task.status === status).length;
  return {
    open: tasks.filter((task) => !CLOSED_STATUSES.has(task.status)).length,
    inProgress: count("in_progress"),
    inReview: count("in_review"),
    blocked: count("blocked"),
  };
}
