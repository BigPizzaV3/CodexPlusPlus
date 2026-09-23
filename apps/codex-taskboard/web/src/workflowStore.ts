import type { WorkflowOption } from "./types";

export const WORKFLOW_STATE_KEY_PREFIX = "taskboard.workflow.workspace.";
export const INITIAL_WORKFLOW_ID = "issue-delivery";
export const INITIAL_WORKFLOW_NAME = "任务处理与交付";
export const DEFAULT_WORKFLOW_OPTIONS: WorkflowOption[] = [
  { id: INITIAL_WORKFLOW_ID, name: INITIAL_WORKFLOW_NAME },
];

export function workflowStorageKey(projectId: string): string {
  return `${WORKFLOW_STATE_KEY_PREFIX}${projectId}`;
}

export function readLegacyWorkflowWorkspace(projectId: string): unknown | null {
  try {
    const raw = window.localStorage.getItem(workflowStorageKey(projectId));
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

export function clearLegacyWorkflowWorkspace(projectId: string): void {
  window.localStorage.removeItem(workflowStorageKey(projectId));
}

export function workflowOptionsFromWorkspace(workspace: unknown): WorkflowOption[] {
  if (!workspace || typeof workspace !== "object" || Array.isArray(workspace)) {
    return DEFAULT_WORKFLOW_OPTIONS;
  }
  const tabs = (workspace as { tabs?: unknown }).tabs;
  if (!Array.isArray(tabs)) return DEFAULT_WORKFLOW_OPTIONS;
  const workflows = tabs.filter((workflow): workflow is WorkflowOption => (
    workflow !== null
      && typeof workflow === "object"
      && typeof (workflow as Partial<WorkflowOption>).id === "string"
      && (workflow as Partial<WorkflowOption>).id!.length > 0
      && typeof (workflow as Partial<WorkflowOption>).name === "string"
      && (workflow as Partial<WorkflowOption>).name!.length > 0
  ));
  return workflows.length > 0 ? workflows : DEFAULT_WORKFLOW_OPTIONS;
}
