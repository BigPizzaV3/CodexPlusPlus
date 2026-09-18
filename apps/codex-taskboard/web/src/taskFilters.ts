import {
  TASK_PRIORITIES,
  TASK_STATUSES,
  type Task,
  type TaskPriority,
  type TaskStatus,
} from "./types";

export type TaskLinkFilter = "all" | "linked" | "unlinked";
export type TaskFilterKey = "statuses" | "priorities" | "labels" | "link" | "content";

export interface TaskFilters {
  statuses: TaskStatus[];
  priorities: TaskPriority[];
  labels: string[];
  link: TaskLinkFilter;
  content: string;
}

export const EMPTY_TASK_FILTERS: TaskFilters = {
  statuses: [],
  priorities: [],
  labels: [],
  link: "all",
  content: "",
};

function isTaskStatus(value: string): value is TaskStatus {
  return TASK_STATUSES.includes(value as TaskStatus);
}

function isTaskPriority(value: string): value is TaskPriority {
  return TASK_PRIORITIES.includes(value as TaskPriority);
}

export function readTaskFilters(): TaskFilters {
  const params = new URLSearchParams(window.location.search);
  const statuses = (params.get("status") ?? "").split(",").filter(isTaskStatus);
  const priorities = (params.get("priority") ?? "").split(",").filter(isTaskPriority);
  const linkValue = params.get("linked");

  return {
    statuses,
    priorities,
    labels: params.getAll("label").filter(Boolean),
    link: linkValue === "yes" ? "linked" : linkValue === "no" ? "unlinked" : "all",
    content: params.get("content") ?? "",
  };
}

export function writeTaskFilters(filters: TaskFilters) {
  const url = new URL(window.location.href);

  if (filters.statuses.length) url.searchParams.set("status", filters.statuses.join(","));
  else url.searchParams.delete("status");

  if (filters.priorities.length) url.searchParams.set("priority", filters.priorities.join(","));
  else url.searchParams.delete("priority");

  url.searchParams.delete("label");
  filters.labels.forEach((label) => url.searchParams.append("label", label));

  if (filters.link === "linked") url.searchParams.set("linked", "yes");
  else if (filters.link === "unlinked") url.searchParams.set("linked", "no");
  else url.searchParams.delete("linked");

  if (filters.content.trim()) url.searchParams.set("content", filters.content.trim());
  else url.searchParams.delete("content");

  window.history.replaceState(null, "", url);
}

export function taskFilterCount(filters: TaskFilters): number {
  return Number(filters.statuses.length > 0)
    + Number(filters.priorities.length > 0)
    + Number(filters.labels.length > 0)
    + Number(filters.link !== "all")
    + Number(Boolean(filters.content.trim()));
}

export function matchesTaskSearch(task: Task, search: string): boolean {
  const needle = search.trim().toLowerCase();
  if (!needle) return true;
  return [task.identifier, task.title, task.description, ...task.labels]
    .join(" ")
    .toLowerCase()
    .includes(needle);
}

export function matchesTaskFilters(
  task: Task,
  filters: TaskFilters,
  omit?: TaskFilterKey,
): boolean {
  if (omit !== "statuses" && filters.statuses.length && !filters.statuses.includes(task.status)) {
    return false;
  }
  if (omit !== "priorities" && filters.priorities.length && !filters.priorities.includes(task.priority)) {
    return false;
  }
  if (
    omit !== "labels"
    && filters.labels.length
    && !filters.labels.some((label) => task.labels.includes(label))
  ) {
    return false;
  }
  if (omit !== "link" && filters.link === "linked" && !task.threadId) return false;
  if (omit !== "link" && filters.link === "unlinked" && task.threadId) return false;

  if (omit !== "content") {
    const content = filters.content.trim().toLowerCase();
    if (content && ![task.title, task.description].join(" ").toLowerCase().includes(content)) {
      return false;
    }
  }

  return true;
}
