import type { Task, TaskRelationSummary } from "./types";

export type HierarchyTrailItem = Pick<TaskRelationSummary, "id" | "identifier" | "title" | "status">;

export function createTaskLookup(tasks: readonly Task[]) {
  return new Map(tasks.map((task) => [task.id, task]));
}

export function getAncestorTrail(
  task: Task,
  taskById: ReadonlyMap<string, Task>,
): HierarchyTrailItem[] {
  const trail: HierarchyTrailItem[] = [];
  const seen = new Set([task.id]);
  let parent = task.relations.parent;

  while (parent && !seen.has(parent.id)) {
    seen.add(parent.id);
    const fullParent = taskById.get(parent.id);
    trail.push(fullParent ?? parent);
    if (!fullParent) break;
    parent = fullParent.relations.parent;
  }

  return trail.reverse();
}

export function countTaskDescendants(
  task: Task,
  taskById: ReadonlyMap<string, Task>,
) {
  const descendants = new Set<string>();
  const queue = [...task.relations.subIssues.map((issue) => issue.id)];

  while (queue.length > 0) {
    const id = queue.shift()!;
    if (descendants.has(id)) continue;
    descendants.add(id);
    const child = taskById.get(id);
    if (child) queue.push(...child.relations.subIssues.map((issue) => issue.id));
  }

  return descendants.size;
}
