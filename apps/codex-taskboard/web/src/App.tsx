import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type Dispatch,
  type PointerEvent as ReactPointerEvent,
  type SetStateAction,
} from "react";
import { createPortal } from "react-dom";
import {
  isAutomationModel,
  isAutomationReasoningEffort,
  isSupportedModelEffort,
  type AutomationModel,
  type AutomationReasoningEffort,
} from "../../shared/taskboard-automation-options.mjs";
import {
  ApiError,
  addTaskRelation,
  archiveProject as archiveProjectRequest,
  archiveTask as archiveTaskRequest,
  createProject as createProjectRequest,
  createTask as createTaskRequest,
  deleteProject as deleteProjectRequest,
  deleteTask as deleteTaskRequest,
  getCodexThreadTitle,
  getTaskboardRevision,
  getWorkflowWorkspace,
  getTaskboardMetadata,
  listDevelopmentContexts,
  listDeviceWorkspaces,
  listProjects,
  listTasks,
  moveTask as moveTaskRequest,
  removeTaskRelation,
  restoreTask as restoreTaskRequest,
  setCurrentUserActor,
  syncCodexThreadTasks,
  uploadAttachment,
  updateTask as updateTaskRequest,
} from "./api";
import {
  actorForAssigneeTarget,
  assigneeTargetForActor,
} from "./actors";
import { BoardColumn, STATUS_DETAILS } from "./components/BoardColumn";
import { AiChat } from "./components/AiChat";
import { ArchivedIssues } from "./components/ArchivedIssues";
import { BoardSettingsMenu } from "./components/BoardSettingsMenu";
import { HiddenColumns } from "./components/HiddenColumns";
import { IssueHierarchyView } from "./components/IssueHierarchyView";
import {
  resolveInlineMediaMarkdown,
  type PendingInlineImage,
} from "./components/InlineMediaComposer";
import { LinearIcon } from "./components/LinearIcon";
import { ProjectAutomationMenu } from "./components/ProjectAutomationMenu";
import { TaskContextMenu } from "./components/TaskContextMenu";
import { TaskDetail } from "./components/TaskDetail";
import { TaskEditor } from "./components/TaskEditor";
import { TaskFilterMenu } from "./components/TaskFilterMenu";
import { buildIssueUrl, readIssueIdentifier } from "./issueRoute";
import { DEFAULT_LABELS } from "./labels";
import {
  EMPTY_TASK_FILTERS,
  matchesTaskFilters,
  matchesTaskSearch,
  readTaskFilters,
  taskFilterCount,
  writeTaskFilters,
} from "./taskFilters";
import {
  TASK_STATUSES,
  type ActorIdentity,
  type DevelopmentScan,
  type HostContext,
  type IssueRelationType,
  type Project,
  type Task,
  type TaskboardMetadata,
  type TaskDraft,
  type TaskStatus,
  type WorkflowOption,
} from "./types";
import {
  DEFAULT_WORKFLOW_OPTIONS,
  readLegacyWorkflowWorkspace,
  workflowOptionsFromWorkspace,
} from "./workflowStore";
// The poller stays in ESM JavaScript so its lifecycle can be tested directly with node:test.
// @ts-expect-error The module's option contract is enforced by its focused node tests.
import { createRevisionPoller, getRevisionPollingInterval } from "./revisionPolling.mjs";

type ConnectionState = "connecting" | "live" | "reconnecting";
type Theme = "light" | "dark";
type BoardView = "issues" | "hierarchy" | "archive" | "workflow";
const SHOW_WORKFLOW_BOARD_ENTRY = false;
const DEFAULT_PROJECT_ID = "local";
const NO_PROJECT_NAME = "无项目";

function projectDisplayName(project: { id: string; name: string } | null | undefined) {
  if (!project) return "";
  return project.id === DEFAULT_PROJECT_ID ? NO_PROJECT_NAME : project.name;
}

const WorkflowBoard = lazy(() => import("./components/WorkflowBoard").then((module) => ({
  default: module.WorkflowBoard,
})));

interface EditorState {
  task: Task | null;
  status: TaskStatus;
  projectId?: string;
}

interface ContextMenuState {
  taskId: string;
  x: number;
  y: number;
}

interface SelectionBox {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface SelectionDragState {
  startX: number;
  startY: number;
  pointerId: number;
  additive: boolean;
  active: boolean;
  baseIds: Set<string>;
}

interface ProjectChoice {
  id: string;
  name: string;
  workspacePath: string | null;
  issueCount: number;
  inCodex: boolean;
  persisted: boolean;
}

function canManageProjectChoice(project: ProjectChoice) {
  return project.persisted && project.id !== DEFAULT_PROJECT_ID;
}

interface UndoOperation {
  id: number;
  message: string;
  undo: () => Promise<void>;
}

interface UndoNotice {
  id: number;
  message: string;
}

type ColumnVisibilityByProject = Record<string, Partial<Record<TaskStatus, boolean>>>;
type ProjectAutomationStatus = "ACTIVE" | "PAUSED";
type AutomationQuotaState = "available" | "blocked" | "unknown" | "unavailable";
type AutomationIntervalMinutes = 5 | 10 | 15 | 30 | 60;

interface AutomationQuotaStatus {
  state: AutomationQuotaState;
  checkedAt: number;
  resetsAt?: number;
  reason?: "api-key";
}

interface ProjectAutomationRecord {
  automationId?: string;
  codexProjectId: string;
  status: ProjectAutomationStatus;
  enabledByUser: boolean;
  quotaAware: boolean;
  quota?: AutomationQuotaStatus;
  intervalMinutes: AutomationIntervalMinutes;
  model: AutomationModel;
  reasoningEffort: AutomationReasoningEffort;
}

type ProjectAutomations = Record<string, ProjectAutomationRecord>;

interface AutomationHostItem {
  id: string;
  status: ProjectAutomationStatus;
  model: AutomationModel;
  reasoningEffort: AutomationReasoningEffort;
  rrule: string;
}

interface AutomationHostResponse {
  requestId: string;
  ok: boolean;
  item?: AutomationHostItem;
  items?: AutomationHostItem[];
  quota?: AutomationQuotaStatus;
  policy?: {
    automationId?: string;
    enabledByUser: boolean;
    quotaAware: boolean;
    intervalMinutes: AutomationIntervalMinutes;
    model: AutomationModel;
    reasoningEffort: AutomationReasoningEffort;
  };
  error?: string;
}

interface PendingAutomationRequest {
  resolve: (response: AutomationHostResponse) => void;
  reject: (error: Error) => void;
  timeoutId: number;
}

const DEFAULT_USER_ACTOR: ActorIdentity = {
  type: "user",
  id: "local-user",
  name: "本地用户",
  avatarUrl: null,
};

const LAST_PROJECT_KEY = "taskboard.lastProjectId";
const FAVORITE_PROJECTS_KEY = "taskboard.favoriteProjectIds";
const HIDDEN_PROJECTS_KEY = "taskboard.hiddenProjectIds.v1";
const DEVICE_WORKSPACE_PATHS_KEY = "taskboard.deviceWorkspacePaths.v1";
const SHOW_EMPTY_COLUMNS_KEY = "taskboard.showEmptyColumns.v1";
const COLUMN_VISIBILITY_KEY = "taskboard.columnVisibility.v1";
const PROJECT_AUTOMATIONS_KEY = "taskboard.projectAutomations.v1";
const DEFAULT_AUTOMATION_OPTIONS = {
  enabledByUser: false,
  quotaAware: false,
  intervalMinutes: 5,
  model: "gpt-5.5",
  reasoningEffort: "high",
} as const;

const EVENT_NAMES = [
  "task.created",
  "task.updated",
  "task.moved",
  "task.archived",
  "task.restored",
  "task.deleted",
  "task.relation.updated",
  "comment.created",
  "comment.updated",
  "comment.deleted",
  "attachment.created",
  "attachment.deleted",
  "project.created",
  "project.archived",
  "project.deleted",
  "workflow.updated",
] as const;

function isTheme(value: unknown): value is Theme {
  return value === "light" || value === "dark";
}

function getInitialTheme(): Theme {
  const fromQuery = new URLSearchParams(window.location.search).get("theme");
  if (isTheme(fromQuery)) return fromQuery;
  const stored = window.localStorage.getItem("taskboard.theme");
  if (isTheme(stored)) return stored;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function readFavoriteProjectIds(): Set<string> {
  try {
    const value = JSON.parse(window.localStorage.getItem(FAVORITE_PROJECTS_KEY) ?? "[]");
    return new Set(Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : []);
  } catch {
    return new Set();
  }
}

function readHiddenProjectIds(): Set<string> {
  try {
    const value = JSON.parse(window.localStorage.getItem(HIDDEN_PROJECTS_KEY) ?? "[]");
    return new Set(Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : []);
  } catch {
    return new Set();
  }
}

function writeStringSet(key: string, value: Set<string>) {
  window.localStorage.setItem(key, JSON.stringify([...value]));
}

function sameStringSet(left: Set<string>, right: Set<string>) {
  if (left.size !== right.size) return false;
  for (const item of left) {
    if (!right.has(item)) return false;
  }
  return true;
}

function readDeviceWorkspacePaths(): Record<string, string> {
  try {
    const value = JSON.parse(window.localStorage.getItem(DEVICE_WORKSPACE_PATHS_KEY) ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter((entry): entry is [string, string] => (
      typeof entry[1] === "string" && entry[1].trim().length > 0
    )));
  } catch {
    return {};
  }
}

function readShowEmptyColumns(): boolean {
  return window.localStorage.getItem(SHOW_EMPTY_COLUMNS_KEY) === "true";
}

function readProjectAutomations(): ProjectAutomations {
  try {
    const value = JSON.parse(window.localStorage.getItem(PROJECT_AUTOMATIONS_KEY) ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    const result: ProjectAutomations = {};
    for (const [projectId, record] of Object.entries(value)) {
      if (!record || typeof record !== "object" || Array.isArray(record)) continue;
      const candidate = record as Partial<ProjectAutomationRecord>;
      const model = candidate.model ?? "gpt-5.5";
      const reasoningEffort = candidate.reasoningEffort ?? "high";
      const enabledByUser = candidate.enabledByUser ?? candidate.status === "ACTIVE";
      const quotaAware = candidate.quotaAware ?? false;
      if (
        (candidate.automationId !== undefined && typeof candidate.automationId !== "string")
        || typeof candidate.codexProjectId !== "string"
        || (candidate.status !== "ACTIVE" && candidate.status !== "PAUSED")
        || !isAutomationIntervalMinutes(candidate.intervalMinutes ?? 5)
        || !isAutomationModel(model)
        || !isAutomationReasoningEffort(reasoningEffort)
        || !isSupportedModelEffort(model, reasoningEffort)
        || (candidate.status === "ACTIVE" && !candidate.automationId)
        || typeof enabledByUser !== "boolean"
        || typeof quotaAware !== "boolean"
      ) continue;
      const quota = isAutomationQuotaStatus(candidate.quota) ? candidate.quota : undefined;
      result[projectId] = {
        automationId: candidate.automationId,
        codexProjectId: candidate.codexProjectId,
        status: candidate.status,
        enabledByUser,
        quotaAware,
        ...(quota ? { quota } : {}),
        intervalMinutes: candidate.intervalMinutes ?? 5,
        model,
        reasoningEffort,
      };
    }
    return result;
  } catch {
    return {};
  }
}

function isAutomationQuotaStatus(value: unknown): value is AutomationQuotaStatus {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const candidate = value as Partial<AutomationQuotaStatus>;
  return (
    (candidate.state === "available"
      || candidate.state === "blocked"
      || candidate.state === "unknown"
      || candidate.state === "unavailable")
    && Number.isFinite(candidate.checkedAt)
    && (candidate.resetsAt === undefined || Number.isFinite(candidate.resetsAt))
    && (candidate.reason === undefined || candidate.reason === "api-key")
  );
}

function isAutomationHostPolicy(
  value: AutomationHostResponse["policy"] | undefined,
): value is NonNullable<AutomationHostResponse["policy"]> {
  return Boolean(
    value
    && (value.automationId === undefined || typeof value.automationId === "string")
    && typeof value.enabledByUser === "boolean"
    && typeof value.quotaAware === "boolean"
    && isAutomationIntervalMinutes(value.intervalMinutes)
    && isAutomationModel(value.model)
    && isAutomationReasoningEffort(value.reasoningEffort)
    && isSupportedModelEffort(value.model, value.reasoningEffort),
  );
}

function isAutomationIntervalMinutes(value: unknown): value is AutomationIntervalMinutes {
  return value === 5 || value === 10 || value === 15 || value === 30 || value === 60;
}

function intervalMinutesFromRrule(value: string): AutomationIntervalMinutes | null {
  const match = /^RRULE:FREQ=MINUTELY;INTERVAL=(5|10|15|30|60)$/.exec(value);
  return match ? Number(match[1]) as AutomationIntervalMinutes : null;
}

function readColumnVisibilityByProject(): ColumnVisibilityByProject {
  try {
    const value = JSON.parse(window.localStorage.getItem(COLUMN_VISIBILITY_KEY) ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    const result: ColumnVisibilityByProject = {};
    for (const [projectId, visibilityValue] of Object.entries(value)) {
      if (!visibilityValue || typeof visibilityValue !== "object" || Array.isArray(visibilityValue)) continue;
      const visibility: Partial<Record<TaskStatus, boolean>> = {};
      for (const status of TASK_STATUSES) {
        const visible = (visibilityValue as Record<string, unknown>)[status];
        if (typeof visible === "boolean") visibility[status] = visible;
      }
      result[projectId] = visibility;
    }
    return result;
  } catch {
    return {};
  }
}

function workspaceName(path?: string): string | null {
  if (!path) return null;
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? path;
}

function errorMessage(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  if (error instanceof Error) return error.message;
  return "Something went wrong while loading your issues.";
}

function isAutomationHostItem(value: unknown): value is AutomationHostItem {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const item = value as Partial<AutomationHostItem>;
  return (
    typeof item.id === "string"
    && (item.status === "ACTIVE" || item.status === "PAUSED")
    && isAutomationModel(item.model)
    && isAutomationReasoningEffort(item.reasoningEffort)
    && isSupportedModelEffort(item.model, item.reasoningEffort)
    && typeof item.rrule === "string"
    && intervalMinutesFromRrule(item.rrule) !== null
  );
}

function isLocalTaskboardOrigin(origin: string): boolean {
  try {
    const { protocol, hostname } = new URL(origin);
    return (protocol === "http:" || protocol === "https:")
      && (hostname === "127.0.0.1" || hostname === "localhost");
  } catch {
    return false;
  }
}

function sortTasks(tasks: Task[]): Task[] {
  return [...tasks].sort(
    (left, right) => left.sortOrder - right.sortOrder || left.createdAt.localeCompare(right.createdAt),
  );
}

function compareTasksByBoardPosition(left: Task, right: Task): number {
  const statusDelta = TASK_STATUSES.indexOf(left.status) - TASK_STATUSES.indexOf(right.status);
  return statusDelta || left.sortOrder - right.sortOrder || left.createdAt.localeCompare(right.createdAt);
}

function calculateGroupSortOrders(previousTask: Task | null, nextTask: Task | null, count: number): number[] {
  if (count <= 0) return [];
  if (previousTask && nextTask) {
    const step = (nextTask.sortOrder - previousTask.sortOrder) / (count + 1);
    return Array.from({ length: count }, (_, index) => previousTask.sortOrder + step * (index + 1));
  }
  if (previousTask) {
    return Array.from({ length: count }, (_, index) => previousTask.sortOrder + 1024 * (index + 1));
  }
  if (nextTask) {
    return Array.from({ length: count }, (_, index) => nextTask.sortOrder - 1024 * (count - index));
  }
  return Array.from({ length: count }, (_, index) => 1024 * (index + 1));
}

function taskToDraft(task: Task): TaskDraft {
  return {
    title: task.title,
    description: task.description,
    status: task.status,
    priority: task.priority,
    labels: task.labels,
    workflowId: task.workflowId,
    developmentContext: task.developmentContext,
    dueDate: task.dueDate,
    recurrence: task.recurrence,
  };
}

interface LocalRealtimeSyncProps {
  selectedProjectId: string;
  detailTaskId: string | null;
  refreshProjectList: () => Promise<void>;
  refreshTasks: (
    projectId: string,
    options?: { quiet?: boolean; signal?: AbortSignal },
  ) => Promise<void>;
  refreshWorkflowOptions: (projectId: string, signal?: AbortSignal) => Promise<void>;
  setConnection: Dispatch<SetStateAction<ConnectionState>>;
  setCommentsRevision: Dispatch<SetStateAction<number>>;
  setAttachmentsRevision: Dispatch<SetStateAction<number>>;
}

function LocalRealtimeSync({
  selectedProjectId,
  detailTaskId,
  refreshProjectList,
  refreshTasks,
  refreshWorkflowOptions,
  setConnection,
  setCommentsRevision,
  setAttachmentsRevision,
}: LocalRealtimeSyncProps) {
  useEffect(() => {
    const source = new EventSource("/api/events");
    let refreshTimer: number | undefined;
    let refreshProjectsPending = false;
    let refreshTasksPending = false;

    const scheduleRefresh = (options: { projects?: boolean; tasks?: boolean }) => {
      refreshProjectsPending ||= options.projects === true;
      refreshTasksPending ||= options.tasks === true;
      window.clearTimeout(refreshTimer);
      refreshTimer = window.setTimeout(() => {
        if (refreshProjectsPending) void refreshProjectList();
        if (refreshTasksPending && selectedProjectId) {
          void refreshTasks(selectedProjectId, { quiet: true });
        }
        refreshProjectsPending = false;
        refreshTasksPending = false;
      }, 120);
    };

    const handleEvent = (event: Event) => {
      const message = event as MessageEvent<string>;
      let payload: { projectId?: string; previousProjectId?: string; taskId?: string } = {};
      try {
        payload = JSON.parse(message.data) as { projectId?: string; previousProjectId?: string; taskId?: string };
      } catch {
        // A malformed event should not interrupt later updates.
      }
      const affectsSelectedProject = Boolean(selectedProjectId)
        && (
          !payload.projectId
          || payload.projectId === selectedProjectId
          || payload.previousProjectId === selectedProjectId
        );
      if (event.type.startsWith("project.")) {
        scheduleRefresh({ projects: true });
        return;
      }
      if (event.type.startsWith("task.")) {
        scheduleRefresh({ projects: true, tasks: affectsSelectedProject });
        return;
      }
      if (!affectsSelectedProject) return;
      if (event.type === "workflow.updated") {
        if (selectedProjectId) void refreshWorkflowOptions(selectedProjectId);
        return;
      }
      if (event.type.startsWith("comment.")) {
        if (!detailTaskId || !payload.taskId || payload.taskId === detailTaskId) {
          setCommentsRevision((current) => current + 1);
        }
        scheduleRefresh({ tasks: true });
        return;
      }
      if (event.type.startsWith("attachment.")) {
        if (!detailTaskId || !payload.taskId || payload.taskId === detailTaskId) {
          setAttachmentsRevision((current) => current + 1);
          setCommentsRevision((current) => current + 1);
        }
      }
    };

    EVENT_NAMES.forEach((name) => source.addEventListener(name, handleEvent));
    source.onopen = () => {
      setConnection("live");
      scheduleRefresh({ projects: true, tasks: Boolean(selectedProjectId) });
      if (selectedProjectId) void refreshWorkflowOptions(selectedProjectId);
      if (detailTaskId) {
        setCommentsRevision((current) => current + 1);
        setAttachmentsRevision((current) => current + 1);
      }
    };
    source.onerror = () => setConnection("reconnecting");

    return () => {
      window.clearTimeout(refreshTimer);
      EVENT_NAMES.forEach((name) => source.removeEventListener(name, handleEvent));
      source.close();
    };
  }, [
    detailTaskId,
    refreshProjectList,
    refreshTasks,
    refreshWorkflowOptions,
    selectedProjectId,
    setAttachmentsRevision,
    setCommentsRevision,
    setConnection,
  ]);

  return null;
}

export function App() {
  const query = useMemo(() => new URLSearchParams(window.location.search), []);
  const embedded = query.get("host") === "codex";
  const undoShortcut = navigator.userAgent.includes("Macintosh") ? "⌘Z" : "Ctrl+Z";
  const [theme, setTheme] = useState<Theme>(getInitialTheme);
  const [hostContext, setHostContext] = useState<HostContext | null>(null);
  const [developmentScan, setDevelopmentScan] = useState<DevelopmentScan>({ workspacePath: null, contexts: [] });
  const [developmentScanLoading, setDevelopmentScanLoading] = useState(false);
  const [manageTaskboardSkillPath, setManageTaskboardSkillPath] = useState("");
  const [taskboardMetadata, setTaskboardMetadata] = useState<TaskboardMetadata | null>(null);
  const [localAiChatAvailable, setLocalAiChatAvailable] = useState(false);
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState("");
  const [tasks, setTasks] = useState<Task[]>([]);
  const [projectsLoading, setProjectsLoading] = useState(true);
  const [tasksLoading, setTasksLoading] = useState(false);
  const [hasLoadedTasks, setHasLoadedTasks] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [connection, setConnection] = useState<ConnectionState>("connecting");
  const [search, setSearch] = useState("");
  const [filters, setFilters] = useState(readTaskFilters);
  const [showEmptyColumns, setShowEmptyColumns] = useState(readShowEmptyColumns);
  const [columnVisibilityByProject, setColumnVisibilityByProject] = useState(readColumnVisibilityByProject);
  const [boardView, setBoardView] = useState<BoardView>("issues");
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [detailTaskIdentifier, setDetailTaskIdentifier] = useState<string | null>(
    () => readIssueIdentifier(window.location.search),
  );
  const [commentsRevision, setCommentsRevision] = useState(0);
  const [attachmentsRevision, setAttachmentsRevision] = useState(0);
  const [workflowRevision, setWorkflowRevision] = useState(0);
  const [workflowOptions, setWorkflowOptions] = useState<WorkflowOption[]>(DEFAULT_WORKFLOW_OPTIONS);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [selectedProjectIds, setSelectedProjectIds] = useState<Set<string>>(() => new Set());
  const [selectedTaskIds, setSelectedTaskIds] = useState<Set<string>>(() => new Set());
  const [projectSelectionBox, setProjectSelectionBox] = useState<SelectionBox | null>(null);
  const [selectionBox, setSelectionBox] = useState<SelectionBox | null>(null);
  const [draggedTaskId, setDraggedTaskId] = useState<string | null>(null);
  const [draggedTaskIds, setDraggedTaskIds] = useState<string[]>([]);
  const [draggedTaskHeight, setDraggedTaskHeight] = useState(0);
  const [dropTarget, setDropTarget] = useState<TaskStatus | null>(null);
  const [movingTaskId, setMovingTaskId] = useState<string | null>(null);
  const [settlingTaskId, setSettlingTaskId] = useState<string | null>(null);
  const [restoringArchivedTaskId, setRestoringArchivedTaskId] = useState<string | null>(null);
  const [pendingArchivedDeleteTask, setPendingArchivedDeleteTask] = useState<Task | null>(null);
  const [pendingBulkDeleteTasks, setPendingBulkDeleteTasks] = useState<Task[]>([]);
  const [bulkArchivingSelectedTasks, setBulkArchivingSelectedTasks] = useState(false);
  const [bulkDeletingSelectedTasks, setBulkDeletingSelectedTasks] = useState(false);
  const [deletingArchivedTask, setDeletingArchivedTask] = useState(false);
  const [projectActionPending, setProjectActionPending] = useState<"archive" | "delete" | null>(null);
  const [openingProjectId, setOpeningProjectId] = useState<string | null>(null);
  const [openingThreadTaskId, setOpeningThreadTaskId] = useState<string | null>(null);
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const [bulkProjectMoveMenuOpen, setBulkProjectMoveMenuOpen] = useState(false);
  const [bulkProjectMoveMenuPosition, setBulkProjectMoveMenuPosition] = useState({ left: 0, top: 0, ready: false });
  const [favoriteProjectIds, setFavoriteProjectIds] = useState(readFavoriteProjectIds);
  const [hiddenProjectIds, setHiddenProjectIds] = useState(readHiddenProjectIds);
  const [deviceWorkspacePaths, setDeviceWorkspacePaths] = useState(readDeviceWorkspacePaths);
  const [projectAutomations, setProjectAutomations] = useState(readProjectAutomations);
  const [automationPending, setAutomationPending] = useState(false);
  const [automationError, setAutomationError] = useState<string | null>(null);
  const [announcement, setAnnouncementValue] = useState("");
  const [undoNotice, setUndoNotice] = useState<UndoNotice | null>(null);
  const tasksRequestRef = useRef(0);
  const projectsRef = useRef<Project[]>([]);
  const tasksRef = useRef<Task[]>([]);
  const undoSequenceRef = useRef(0);
  const undoStackRef = useRef<UndoOperation[]>([]);
  const undoInFlightRef = useRef(false);
  const codexThreadTitleCacheRef = useRef(new Map<string, string>());
  const lastCodexThreadSyncRef = useRef("");
  const dragRegionRef = useRef<HTMLDivElement>(null);
  const projectSelectionDragRef = useRef<SelectionDragState | null>(null);
  const selectionDragRef = useRef<SelectionDragState | null>(null);
  const bulkProjectMoveTriggerRef = useRef<HTMLButtonElement>(null);
  const bulkProjectMoveMenuRef = useRef<HTMLDivElement>(null);
  const selectedProjectIdRef = useRef(selectedProjectId);
  selectedProjectIdRef.current = selectedProjectId;
  projectsRef.current = projects;

  const revisionPollingInterval = getRevisionPollingInterval(taskboardMetadata);
  const pendingAutomationRequestsRef = useRef(new Map<string, PendingAutomationRequest>());
  const automationRequestInFlightRef = useRef(false);
  const projectAutomationsRef = useRef(projectAutomations);

  const setAnnouncement = useCallback((message: string) => {
    setUndoNotice(null);
    setAnnouncementValue(message);
  }, []);

  const updateHiddenProjectIds = useCallback((updater: (current: Set<string>) => Set<string>) => {
    setHiddenProjectIds((current) => {
      const next = updater(new Set(current));
      if (sameStringSet(next, current)) return current;
      writeStringSet(HIDDEN_PROJECTS_KEY, next);
      return next;
    });
  }, []);

  const reconcileProjectListState = useCallback((nextProjects: Project[]) => {
    const nextProjectIds = new Set(nextProjects.map((project) => project.id));
    const previousProjectIds = new Set(projectsRef.current.map((project) => project.id));
    updateHiddenProjectIds((current) => {
      for (const projectId of previousProjectIds) {
        if (!nextProjectIds.has(projectId) && projectId !== DEFAULT_PROJECT_ID) current.add(projectId);
      }
      for (const projectId of nextProjectIds) current.delete(projectId);
      return current;
    });
    setSelectedProjectIds((current) => {
      if (current.size === 0) return current;
      const next = new Set([...current].filter((projectId) => nextProjectIds.has(projectId)));
      return sameStringSet(next, current) ? current : next;
    });
  }, [updateHiddenProjectIds]);

  const forgetProjects = useCallback((projectIds: string[]) => {
    const ids = new Set(projectIds);
    if (ids.size === 0) return;
    setFavoriteProjectIds((current) => {
      const next = new Set([...current].filter((projectId) => !ids.has(projectId)));
      if (sameStringSet(next, current)) return current;
      writeStringSet(FAVORITE_PROJECTS_KEY, next);
      return next;
    });
    setDeviceWorkspacePaths((current) => {
      let changed = false;
      const next = { ...current };
      for (const projectId of ids) {
        if (projectId in next) {
          delete next[projectId];
          changed = true;
        }
      }
      if (!changed) return current;
      window.localStorage.setItem(DEVICE_WORKSPACE_PATHS_KEY, JSON.stringify(next));
      return next;
    });
    setProjectAutomations((current) => {
      let changed = false;
      const next = { ...current };
      for (const projectId of ids) {
        if (projectId in next) {
          delete next[projectId];
          changed = true;
        }
      }
      if (!changed) return current;
      window.localStorage.setItem(PROJECT_AUTOMATIONS_KEY, JSON.stringify(next));
      return next;
    });
    updateHiddenProjectIds((current) => {
      for (const projectId of ids) current.add(projectId);
      return current;
    });
    const rememberedProjectId = window.localStorage.getItem(LAST_PROJECT_KEY);
    if (rememberedProjectId && ids.has(rememberedProjectId)) {
      window.localStorage.removeItem(LAST_PROJECT_KEY);
    }
  }, [updateHiddenProjectIds]);

  const rememberDeviceWorkspacePath = useCallback((projectId: string, workspacePath: string) => {
    const normalizedPath = workspacePath.trim();
    setDeviceWorkspacePaths((current) => {
      if (current[projectId] === normalizedPath || (!normalizedPath && !(projectId in current))) {
        return current;
      }
      const next = { ...current };
      if (normalizedPath) next[projectId] = normalizedPath;
      else delete next[projectId];
      window.localStorage.setItem(DEVICE_WORKSPACE_PATHS_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const mergeDeviceWorkspacePaths = useCallback((workspaces: Record<string, string>) => {
    setDeviceWorkspacePaths((current) => {
      const next = { ...current, ...workspaces };
      if (JSON.stringify(next) === JSON.stringify(current)) return current;
      window.localStorage.setItem(DEVICE_WORKSPACE_PATHS_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const selectedProject = projects.find((project) => project.id === selectedProjectId) ?? null;
  const selectedHostProject = hostContext?.projects?.find((project) => project.id === selectedProjectId);
  const selectedProjectDisplayName = selectedProject
    ? projectDisplayName({ ...selectedProject, name: selectedHostProject?.name ?? selectedProject.name })
    : "";
  const currentUser = hostContext?.user ?? DEFAULT_USER_ACTOR;
  const selectedDeviceWorkspacePath = deviceWorkspacePaths[selectedProjectId];
  const selectedProjectAutomation = projectAutomations[selectedProjectId];
  const automationProjectContext = useMemo(() => {
    if (!embedded || window.parent === window) {
      return { unavailableReason: "仅可在 Codex App 中使用" };
    }
    if (!isLocalTaskboardOrigin(window.location.origin)) {
      return { unavailableReason: "仅本地任务面板可用" };
    }
    if (!selectedProject) return { unavailableReason: "请先选择项目" };

    const directCodexProject = hostContext?.projects?.some(
      (project) => project.id === selectedProject.id,
    );
    const workspacePath = deviceWorkspacePaths[selectedProject.id]
      ?? selectedProject.workspacePath
      ?? (
        directCodexProject && hostContext?.projectId === selectedProject.id
          ? hostContext.workspacePath
          : undefined
      );
    const codexProjectId = directCodexProject
      ? selectedProject.id
      : hostContext?.projects?.find(
        (project) => deviceWorkspacePaths[project.id] === workspacePath,
      )?.id;

    if (!workspacePath || !codexProjectId) {
      return { unavailableReason: "请先在 Codex 中添加并映射该项目目录" };
    }
    if (!manageTaskboardSkillPath) {
      return { unavailableReason: "任务面板还没有读取到 Skill 路径" };
    }
    return { workspacePath, codexProjectId, unavailableReason: null };
  }, [
    deviceWorkspacePaths,
    embedded,
    hostContext,
    manageTaskboardSkillPath,
    selectedProject,
  ]);
  const syncCodexProjectId = automationProjectContext.codexProjectId ?? "";
  const detailTask = detailTaskIdentifier
    ? tasks.find((task) => task.identifier === detailTaskIdentifier) ?? null
    : null;
  const detailTaskId = detailTask?.id ?? null;
  const contextMenuTask = contextMenu
    ? tasks.find((task) => task.id === contextMenu.taskId) ?? null
    : null;
  const availableLabels = useMemo(
    () => [...new Set([
      ...DEFAULT_LABELS.map((label) => label.name),
      ...tasks.flatMap((task) => task.labels),
    ])],
    [tasks],
  );
  const projectChoices = useMemo<ProjectChoice[]>(() => {
    const persistedById = new Map(projects.map((project) => [project.id, project]));
    const seen = new Set<string>();
    const choices: ProjectChoice[] = [];
    for (const project of hostContext?.projects ?? []) {
      if (!project.id || !project.name || seen.has(project.id) || hiddenProjectIds.has(project.id)) continue;
      const persisted = persistedById.get(project.id);
      seen.add(project.id);
      choices.push({
        id: project.id,
        name: projectDisplayName({ id: project.id, name: project.name }),
        workspacePath: deviceWorkspacePaths[project.id]
          ?? persisted?.workspacePath
          ?? (hostContext?.projectId === project.id ? hostContext.workspacePath ?? null : null),
        issueCount: persisted?.issueCount ?? 0,
        inCodex: true,
        persisted: Boolean(persisted),
      });
    }
    for (const project of projects) {
      if (seen.has(project.id)) continue;
      choices.push({
        id: project.id,
        name: projectDisplayName(project),
        workspacePath: deviceWorkspacePaths[project.id] ?? project.workspacePath ?? null,
        issueCount: project.issueCount,
        inCodex: false,
        persisted: true,
      });
    }
    return choices.sort((left, right) => (
      Number(favoriteProjectIds.has(right.id)) - Number(favoriteProjectIds.has(left.id))
    ));
  }, [
    deviceWorkspacePaths,
    favoriteProjectIds,
    hiddenProjectIds,
    hostContext?.projectId,
    hostContext?.projects,
    hostContext?.workspacePath,
    projects,
  ]);
  const projectsWithIssues = useMemo(
    () => projectChoices.filter((project) => project.issueCount > 0),
    [projectChoices],
  );
  const projectsWithoutIssues = useMemo(
    () => projectChoices.filter((project) => project.issueCount === 0),
    [projectChoices],
  );
  const closeContextMenu = useCallback(() => setContextMenu(null), []);

  const writeProjectAutomation = useCallback((
    projectId: string,
    record: ProjectAutomationRecord | null | undefined,
  ) => {
    setProjectAutomations((current) => {
      if (
        record
        && current[projectId]?.automationId === record.automationId
        && current[projectId]?.codexProjectId === record.codexProjectId
        && current[projectId]?.status === record.status
        && current[projectId]?.enabledByUser === record.enabledByUser
        && current[projectId]?.quotaAware === record.quotaAware
        && JSON.stringify(current[projectId]?.quota) === JSON.stringify(record.quota)
        && current[projectId]?.intervalMinutes === record.intervalMinutes
        && current[projectId]?.model === record.model
        && current[projectId]?.reasoningEffort === record.reasoningEffort
      ) {
        return current;
      }
      const next = { ...current };
      if (record) next[projectId] = record;
      else delete next[projectId];
      projectAutomationsRef.current = next;
      window.localStorage.setItem(PROJECT_AUTOMATIONS_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const sendAutomationRequest = useCallback((
    operation: "ensure-active" | "pause" | "list" | "apply-policy",
    options: Pick<
      ProjectAutomationRecord,
      "enabledByUser" | "quotaAware" | "intervalMinutes" | "model" | "reasoningEffort"
    >,
    automationId?: string,
  ) => {
    if (
      !selectedProject
      || !automationProjectContext.codexProjectId
      || !automationProjectContext.workspacePath
    ) {
      return Promise.reject(new Error(
        automationProjectContext.unavailableReason ?? "无法读取项目自动化信息",
      ));
    }
    const requestId = window.crypto.randomUUID();
    const response = new Promise<AutomationHostResponse>((resolve, reject) => {
      const timeoutId = window.setTimeout(() => {
        pendingAutomationRequestsRef.current.delete(requestId);
        reject(new Error("Codex 自动化没有响应，请稍后重试"));
      }, 10_000);
      pendingAutomationRequestsRef.current.set(requestId, { resolve, reject, timeoutId });
    });
    window.parent.postMessage({
      type: "taskboard:automation-request",
      payload: {
        requestId,
        operation,
        taskboardProjectId: selectedProjectId,
        codexProjectId: automationProjectContext.codexProjectId,
        projectName: selectedProjectDisplayName,
        workspacePath: automationProjectContext.workspacePath,
        skillPath: manageTaskboardSkillPath,
        ...(automationId ? { automationId } : {}),
        enabledByUser: options.enabledByUser,
        quotaAware: options.quotaAware,
        intervalMinutes: options.intervalMinutes,
        model: options.model,
        reasoningEffort: options.reasoningEffort,
      },
    }, "*");
    return response;
  }, [
    automationProjectContext,
    manageTaskboardSkillPath,
    selectedProject,
    selectedProjectDisplayName,
    selectedProjectId,
  ]);

  const reconcileProjectAutomation = useCallback(async () => {
    if (automationProjectContext.unavailableReason) {
      setAutomationError(null);
      return;
    }
    if (!selectedProjectId || !automationProjectContext.codexProjectId || automationRequestInFlightRef.current) return;
    const stored = projectAutomationsRef.current[selectedProjectId];
    automationRequestInFlightRef.current = true;
    setAutomationPending(true);
    setAutomationError(null);
    try {
      const options = stored ?? {
        status: "PAUSED" as const,
        ...DEFAULT_AUTOMATION_OPTIONS,
      };
      const response = await sendAutomationRequest(
        stored ? "apply-policy" : "list",
        options,
        stored?.automationId,
      );
      const items = Array.isArray(response.items)
        ? response.items.filter(isAutomationHostItem)
        : [];
      if (!stored) {
        const policy = isAutomationHostPolicy(response.policy) ? response.policy : null;
        if (!policy) return;
        const item = items.find((candidate) => candidate.id === policy.automationId)
          ?? (items.length === 1 ? items[0] : undefined);
        writeProjectAutomation(selectedProjectId, {
          automationId: item?.id ?? policy.automationId,
          codexProjectId: automationProjectContext.codexProjectId,
          status: item?.status ?? "PAUSED",
          enabledByUser: policy.enabledByUser,
          quotaAware: policy.quotaAware,
          intervalMinutes: policy.intervalMinutes,
          model: policy.model,
          reasoningEffort: policy.reasoningEffort,
        });
        return;
      }
      const item = (isAutomationHostItem(response.item) ? response.item : undefined)
        ?? items.find((item) => item.id === stored?.automationId)
        ?? (items.length === 1 ? items[0] : undefined);
      if (!item) {
        if (stored) {
          writeProjectAutomation(selectedProjectId, {
            ...stored,
            automationId: undefined,
            status: "PAUSED",
            ...(response.quota ? { quota: response.quota } : {}),
          });
        }
        return;
      }
      const intervalMinutes = intervalMinutesFromRrule(item.rrule);
      if (!intervalMinutes) return;
      writeProjectAutomation(selectedProjectId, {
        automationId: item.id,
        codexProjectId: automationProjectContext.codexProjectId,
        status: item.status,
        enabledByUser: stored.enabledByUser,
        quotaAware: stored.quotaAware,
        ...(response.quota ? { quota: response.quota } : {}),
        intervalMinutes,
        model: item.model,
        reasoningEffort: item.reasoningEffort,
      });
    } catch (error) {
      setAutomationError(error instanceof Error ? error.message : "无法读取自动化状态");
    } finally {
      automationRequestInFlightRef.current = false;
      setAutomationPending(false);
    }
  }, [
    automationProjectContext,
    selectedProjectId,
    sendAutomationRequest,
    writeProjectAutomation,
  ]);

  const saveProjectAutomation = useCallback(async (options: {
    enabledByUser: boolean;
    quotaAware: boolean;
    intervalMinutes: AutomationIntervalMinutes;
    model: AutomationModel;
    reasoningEffort: AutomationReasoningEffort;
  }) => {
    const stored = projectAutomations[selectedProjectId];
    if (
      !selectedProjectId
      || automationProjectContext.unavailableReason
      || !automationProjectContext.codexProjectId
      || automationRequestInFlightRef.current
    ) return;
    const previousRecord = stored;
    automationRequestInFlightRef.current = true;
    setAutomationPending(true);
    setAutomationError(null);
    try {
      const response = await sendAutomationRequest("apply-policy", options, stored?.automationId);
      const item = isAutomationHostItem(response.item) ? response.item : undefined;
      writeProjectAutomation(selectedProjectId, {
        automationId: item?.id,
        codexProjectId: automationProjectContext.codexProjectId,
        status: item?.status ?? "PAUSED",
        enabledByUser: options.enabledByUser,
        quotaAware: options.quotaAware,
        ...(response.quota ? { quota: response.quota } : {}),
        intervalMinutes: options.intervalMinutes,
        model: options.model,
        reasoningEffort: options.reasoningEffort,
      });
    } catch (error) {
      writeProjectAutomation(selectedProjectId, previousRecord);
      setAutomationError(error instanceof Error ? error.message : "无法更新自动化");
    } finally {
      automationRequestInFlightRef.current = false;
      setAutomationPending(false);
    }
  }, [
    automationProjectContext,
    projectAutomations,
    selectedProjectId,
    sendAutomationRequest,
    writeProjectAutomation,
  ]);

  function openTaskDetail(task: Pick<Task, "identifier" | "projectId">) {
    closeContextMenu();
    setProjectMenuOpen(false);
    setBulkProjectMoveMenuOpen(false);
    setPendingBulkDeleteTasks([]);
    setDetailTaskIdentifier(task.identifier);
    const currentIssue = readIssueIdentifier(window.location.search);
    const boardUrl = buildIssueUrl(window.location.href, task.projectId, null);
    if (!currentIssue) {
      window.history.replaceState(window.history.state, "", boardUrl);
    }
    const detailUrl = buildIssueUrl(
      currentIssue ? window.location.href : boardUrl.href,
      task.projectId,
      task.identifier,
    );
    window.history.pushState(window.history.state, "", detailUrl);
  }

  function closeTaskDetail() {
    setPendingBulkDeleteTasks([]);
    setDetailTaskIdentifier(null);
    const url = buildIssueUrl(window.location.href, selectedProjectId || null, null);
    window.history.replaceState(window.history.state, "", url);
  }

  useEffect(() => {
    function syncRouteFromLocation() {
      const url = new URL(window.location.href);
      const routeProjectId = url.searchParams.get("project") ?? "";
      setDetailTaskIdentifier(readIssueIdentifier(url.search));
      if (routeProjectId === selectedProjectId) return;
      setBoardView("issues");
      setPendingBulkDeleteTasks([]);
      setSelectedProjectId(routeProjectId);
      if (routeProjectId) window.localStorage.setItem(LAST_PROJECT_KEY, routeProjectId);
      else window.localStorage.removeItem(LAST_PROJECT_KEY);
    }

    window.addEventListener("popstate", syncRouteFromLocation);
    return () => window.removeEventListener("popstate", syncRouteFromLocation);
  }, [selectedProjectId]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.embedded = String(embedded);
    document.documentElement.style.colorScheme = theme;
    if (!embedded) window.localStorage.setItem("taskboard.theme", theme);
  }, [embedded, theme]);

  useEffect(() => {
    writeTaskFilters(filters);
  }, [filters]);

  useEffect(() => {
    tasksRef.current = tasks;
  }, [tasks]);

  useEffect(() => {
    setSelectedTaskIds((current) => {
      if (current.size === 0) return current;
      const availableTaskIds = new Set(tasks.map((task) => task.id));
      const next = new Set([...current].filter((taskId) => availableTaskIds.has(taskId)));
      return next.size === current.size ? current : next;
    });
  }, [tasks]);

  useEffect(() => {
    if (!projectMenuOpen) return;
    function closeProjectMenu(event: PointerEvent) {
      const target = event.target as HTMLElement;
      if (!target.closest("[data-project-switcher]")) setProjectMenuOpen(false);
    }
    function closeProjectMenuWithEscape(event: KeyboardEvent) {
      if (event.key === "Escape") setProjectMenuOpen(false);
    }
    document.addEventListener("pointerdown", closeProjectMenu);
    window.addEventListener("keydown", closeProjectMenuWithEscape);
    return () => {
      document.removeEventListener("pointerdown", closeProjectMenu);
      window.removeEventListener("keydown", closeProjectMenuWithEscape);
    };
  }, [projectMenuOpen]);

  useEffect(() => {
    setAutomationError(null);
    void reconcileProjectAutomation();
  }, [selectedProjectId, reconcileProjectAutomation]);

  useEffect(() => {
    if (!embedded || window.parent === window) return;

    function receiveHostMessage(event: MessageEvent) {
      if (event.source !== window.parent || !event.data || typeof event.data !== "object") return;
      const message = event.data as { type?: string; payload?: unknown; theme?: unknown };

      if (message.type === "taskboard:automation-response" && message.payload) {
        const payload = message.payload as Partial<AutomationHostResponse>;
        if (typeof payload.requestId !== "string") return;
        const pending = pendingAutomationRequestsRef.current.get(payload.requestId);
        if (!pending) return;
        window.clearTimeout(pending.timeoutId);
        pendingAutomationRequestsRef.current.delete(payload.requestId);
        if (payload.ok) pending.resolve(payload as AutomationHostResponse);
        else pending.reject(new Error(
          typeof payload.error === "string" ? payload.error : "Codex 无法更新自动化",
        ));
        return;
      }

      if (message.type === "taskboard:theme" && isTheme(message.theme)) {
        setTheme(message.theme);
        return;
      }

      if (message.type === "taskboard:thread-prepared") {
        setOpeningThreadTaskId(null);
        return;
      }

      if (message.type === "taskboard:thread-create-error" && message.payload) {
        const payload = message.payload as { taskId?: unknown; error?: unknown };
        setOpeningThreadTaskId(null);
        setActionError(typeof payload.error === "string" ? payload.error : "无法在 Codex 中创建对话。");
        return;
      }

      if (message.type !== "taskboard:host-context" || !message.payload) return;
      const payload = message.payload as HostContext;
      setHostContext(payload);
      setCurrentUserActor(payload.user);
      if (isTheme(payload.theme)) setTheme(payload.theme);
    }

    window.addEventListener("message", receiveHostMessage);
    window.parent.postMessage({ type: "taskboard:ready" }, "*");
    return () => {
      window.removeEventListener("message", receiveHostMessage);
      for (const pending of pendingAutomationRequestsRef.current.values()) {
        window.clearTimeout(pending.timeoutId);
      }
      pendingAutomationRequestsRef.current.clear();
    };
  }, [embedded]);

  useLayoutEffect(() => {
    if (!embedded || window.parent === window || !dragRegionRef.current) return;
    const region = dragRegionRef.current;
    const publish = () => {
      const rect = region.getBoundingClientRect();
      window.parent.postMessage({
        type: "taskboard:drag-region",
        payload: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      }, "*");
    };
    const observer = new ResizeObserver(publish);
    observer.observe(region);
    window.addEventListener("resize", publish);
    publish();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", publish);
      window.parent.postMessage({ type: "taskboard:drag-region", payload: null }, "*");
    };
  }, [detailTaskId, embedded, selectedProjectId]);

  const loadProjectList = useCallback(async (signal?: AbortSignal) => {
    setProjectsLoading(true);
    setLoadError(null);
    try {
      const [nextProjects, metadata, workspaces] = await Promise.all([
        listProjects(signal),
        getTaskboardMetadata(signal),
        listDeviceWorkspaces(signal),
      ]);
      setTaskboardMetadata((current) => (
        current
        && current.mode === metadata.mode
        && current.realtime?.transport === metadata.realtime?.transport
        && current.realtime?.intervalMs === metadata.realtime?.intervalMs
        && current.manageTaskboardSkillPath === metadata.manageTaskboardSkillPath
        && current.localCapabilities?.available === metadata.localCapabilities?.available
          ? current
          : metadata
      ));
      setManageTaskboardSkillPath(metadata.manageTaskboardSkillPath ?? "");
      setLocalAiChatAvailable(metadata.capabilities?.localAiChat === true);
      mergeDeviceWorkspacePaths(workspaces);
      reconcileProjectListState(nextProjects);
      setProjects(nextProjects);
      setSelectedProjectId((current) => {
        const fromQuery = new URLSearchParams(window.location.search).get("project");
        const remembered = window.localStorage.getItem(LAST_PROJECT_KEY);
        if (fromQuery && nextProjects.some((project) => project.id === fromQuery)) return fromQuery;
        if (current && nextProjects.some((project) => project.id === current)) return current;
        if (remembered && nextProjects.some((project) => project.id === remembered)) return remembered;
        return "";
      });
    } catch (error) {
      if ((error as Error).name !== "AbortError") setLoadError(errorMessage(error));
    } finally {
      setProjectsLoading(false);
    }
  }, [mergeDeviceWorkspacePaths, reconcileProjectListState]);

  useEffect(() => {
    const controller = new AbortController();
    void loadProjectList(controller.signal);
    return () => controller.abort();
  }, [loadProjectList]);

  const hostProjectIds = useMemo(() => (
    (hostContext?.projects ?? [])
      .map((project) => project.id)
      .filter(Boolean)
      .sort()
      .join("\n")
  ), [hostContext?.projects]);

  const hostVisibleThreadIds = useMemo(() => (
    (hostContext?.visibleThreadIds ?? [])
      .map((threadId) => threadId.trim())
      .filter(Boolean)
      .sort()
      .join("\n")
  ), [hostContext?.visibleThreadIds]);

  useEffect(() => {
    if (!embedded || !hostProjectIds) return;
    const controller = new AbortController();
    void listDeviceWorkspaces(controller.signal)
      .then(mergeDeviceWorkspacePaths)
      .catch((error) => {
        if ((error as Error).name !== "AbortError") {
          console.warn("Unable to refresh Codex project workspaces", error);
      }
    });
    return () => controller.abort();
  }, [embedded, hostProjectIds, mergeDeviceWorkspacePaths]);

  useEffect(() => {
    if (!embedded || !syncCodexProjectId) return;
    const signature = `${syncCodexProjectId}\n${hostVisibleThreadIds}`;
    if (lastCodexThreadSyncRef.current === signature) return;
    lastCodexThreadSyncRef.current = signature;
    void syncCodexThreadTasks({
      projectId: syncCodexProjectId,
      visibleThreadIds: hostContext?.visibleThreadIds ?? [],
    }).catch((error) => {
      console.warn("Unable to sync Codex thread tasks", error);
    });
  }, [embedded, hostContext?.visibleThreadIds, hostVisibleThreadIds, syncCodexProjectId]);

  useEffect(() => {
    if (!hostContext?.projectId || !hostContext.workspacePath) return;
    mergeDeviceWorkspacePaths({ [hostContext.projectId]: hostContext.workspacePath });
  }, [hostContext?.projectId, hostContext?.workspacePath, mergeDeviceWorkspacePaths]);

  const refreshProjectList = useCallback(async () => {
    try {
      const nextProjects = await listProjects();
      reconcileProjectListState(nextProjects);
      setProjects(nextProjects);
      const selectedProjectMissing = Boolean(selectedProjectIdRef.current)
        && !nextProjects.some((project) => project.id === selectedProjectIdRef.current);
      if (selectedProjectMissing) {
        setDetailTaskIdentifier(null);
        setSelectedTaskIds(new Set());
        window.localStorage.removeItem(LAST_PROJECT_KEY);
        window.history.replaceState(null, "", buildIssueUrl(window.location.href, null, null));
      }
      setSelectedProjectId((current) => {
        if (!current || nextProjects.some((project) => project.id === current)) return current;
        return "";
      });
    } catch (error) {
      setLoadError(errorMessage(error));
    }
  }, [reconcileProjectListState]);

  const refreshTasks = useCallback(async (
    projectId: string,
    options: { quiet?: boolean; signal?: AbortSignal } = {},
  ) => {
    const requestId = ++tasksRequestRef.current;
    if (!options.quiet) setTasksLoading(true);
    setLoadError(null);
    try {
      const nextTasks = await listTasks(projectId, {
        signal: options.signal,
        archived: boardView === "archive" ? "true" : "false",
      });
      if (requestId !== tasksRequestRef.current) return;
      setTasks(sortTasks(nextTasks));
      const nextTaskIds = new Set(nextTasks.map((task) => task.id));
      setSelectedTaskIds((current) => {
        if (current.size === 0) return current;
        const next = new Set([...current].filter((taskId) => nextTaskIds.has(taskId)));
        return next.size === current.size ? current : next;
      });
      setHasLoadedTasks(true);
    } catch (error) {
      if ((error as Error).name !== "AbortError" && requestId === tasksRequestRef.current) {
        setLoadError(errorMessage(error));
      }
    } finally {
      if (!options.quiet && requestId === tasksRequestRef.current) setTasksLoading(false);
    }
  }, [boardView]);

  useEffect(() => {
    if (!selectedProjectId) {
      setTasks([]);
      setHasLoadedTasks(false);
      return;
    }
    setHasLoadedTasks(false);
    const controller = new AbortController();
    void refreshTasks(selectedProjectId, { signal: controller.signal });
    return () => controller.abort();
  }, [refreshTasks, selectedProjectId]);

  const refreshWorkflowOptions = useCallback(async (projectId: string, signal?: AbortSignal) => {
    const record = await getWorkflowWorkspace<unknown>(projectId, signal);
    if (!signal?.aborted) setWorkflowOptions(workflowOptionsFromWorkspace(record.workspace));
  }, []);

  useEffect(() => {
    if (!selectedProjectId) {
      setWorkflowOptions(DEFAULT_WORKFLOW_OPTIONS);
      return;
    }
    setWorkflowOptions(workflowOptionsFromWorkspace(readLegacyWorkflowWorkspace(selectedProjectId)));
    const controller = new AbortController();
    void refreshWorkflowOptions(selectedProjectId, controller.signal).catch((error) => {
      if ((error as Error).name !== "AbortError") {
        setWorkflowOptions(workflowOptionsFromWorkspace(readLegacyWorkflowWorkspace(selectedProjectId)));
      }
    });
    return () => controller.abort();
  }, [refreshWorkflowOptions, selectedProjectId]);

  useEffect(() => {
    if (!selectedProjectId) {
      setDevelopmentScan({ workspacePath: null, contexts: [] });
      return;
    }
    const controller = new AbortController();
    const codexProjectId = selectedProjectId === DEFAULT_PROJECT_ID ? hostContext?.projectId : selectedProjectId;
    const codexThreadId = hostContext?.threadId ?? detailTask?.threadId ?? undefined;
    setDevelopmentScan({ workspacePath: selectedDeviceWorkspacePath ?? null, contexts: [] });
    setDevelopmentScanLoading(true);
    void listDevelopmentContexts(
      selectedProjectId,
      codexProjectId,
      codexThreadId,
      controller.signal,
      selectedDeviceWorkspacePath,
    )
      .then((scan) => {
        setDevelopmentScan(scan);
        if (scan.workspacePath) rememberDeviceWorkspacePath(selectedProjectId, scan.workspacePath);
      })
      .catch((error) => {
        if ((error as Error).name !== "AbortError") {
          setDevelopmentScan({ workspacePath: selectedDeviceWorkspacePath ?? null, contexts: [] });
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setDevelopmentScanLoading(false);
      });
    return () => controller.abort();
  }, [
    detailTask?.threadId,
    hostContext?.projectId,
    hostContext?.threadId,
    rememberDeviceWorkspacePath,
    selectedProjectId,
    selectedDeviceWorkspacePath,
  ]);

  useEffect(() => {
    if (revisionPollingInterval === null) return;
    const controller = new AbortController();
    setConnection("connecting");
    const poller = createRevisionPoller({
      intervalMs: revisionPollingInterval,
      fetchRevision: async (since: number) => {
        try {
          const result = await getTaskboardRevision(since, controller.signal);
          setConnection("live");
          return result;
        } catch (error) {
          if (!controller.signal.aborted) setConnection("reconnecting");
          throw error;
        }
      },
      onInvalidate: () => {
        void refreshProjectList();
        const projectId = selectedProjectIdRef.current;
        if (projectId) {
          void refreshTasks(projectId, { quiet: true });
          void refreshWorkflowOptions(projectId).catch(() => {});
        }
        setWorkflowRevision((current) => current + 1);
        setCommentsRevision((current) => current + 1);
        setAttachmentsRevision((current) => current + 1);
      },
    });
    poller.start();
    return () => {
      controller.abort();
      poller.stop();
    };
  }, [
    revisionPollingInterval,
    refreshProjectList,
    refreshTasks,
    refreshWorkflowOptions,
  ]);

  function pushUndo(message: string, undo: () => Promise<void>, showNotice = true) {
    const operation = { id: ++undoSequenceRef.current, message, undo };
    undoStackRef.current = [...undoStackRef.current.slice(-19), operation];
    setAnnouncementValue("");
    setUndoNotice(showNotice ? { id: operation.id, message } : null);
  }

  async function performUndo() {
    if (undoInFlightRef.current) return;
    const operation = undoStackRef.current.at(-1);
    if (!operation) return;
    undoStackRef.current = undoStackRef.current.slice(0, -1);
    undoInFlightRef.current = true;
    setUndoNotice(null);
    setProjectMenuOpen(false);
    setBulkProjectMoveMenuOpen(false);
    setPendingBulkDeleteTasks([]);
    closeContextMenu();
    setActionError(null);
    try {
      await operation.undo();
    } catch (error) {
      setActionError(`无法撤回这次操作：${errorMessage(error)}`);
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    } finally {
      undoInFlightRef.current = false;
    }
  }

  async function restoreTaskDetails(
    snapshot: Task,
    changed: Task,
    assigneeTarget = assigneeTargetForActor(snapshot.assignee, currentUser),
  ) {
    const candidate = tasksRef.current.find((task) => task.id === changed.id);
    const current = candidate && candidate.version >= changed.version ? candidate : changed;
    const restored = await updateTaskRequest(current, {
      ...taskToDraft(snapshot),
      ...(assigneeTarget ? { assigneeTarget } : {}),
    });
    setTasks((tasks) => sortTasks(tasks.map((task) => task.id === restored.id ? restored : task)));
  }

  useEffect(() => {
    function handleShortcut(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const isTyping = target?.matches("input, textarea, select, [contenteditable='true']");
      if (
        event.key.toLowerCase() === "z"
        && (event.metaKey || event.ctrlKey)
        && !event.shiftKey
        && !isTyping
        && !editor
      ) {
        event.preventDefault();
        void performUndo();
        return;
      }
      if (isTyping || contextMenu || projectMenuOpen || bulkProjectMoveMenuOpen) return;
      if (
        event.key.toLowerCase() === "c"
        && !event.metaKey
        && !event.ctrlKey
        && selectedProjectId
        && boardView === "issues"
      ) {
        event.preventDefault();
        setEditor({ task: null, status: "backlog" });
      }
      if (event.key === "/" && !detailTaskId && selectedProjectId && boardView !== "workflow") {
        event.preventDefault();
        document.getElementById("task-search")?.focus();
      }
      if (event.key === "Escape" && detailTaskId) {
        closeTaskDetail();
      }
    }

    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [boardView, bulkProjectMoveMenuOpen, contextMenu, detailTaskId, editor, projectMenuOpen, selectedProjectId]);

  const filteredTasks = useMemo(() => {
    return tasks.filter(
      (task) => matchesTaskSearch(task, search) && matchesTaskFilters(task, filters),
    );
  }, [filters, search, tasks]);

  const activeFilterCount = taskFilterCount(filters);
  const selectedProjectIdArray = useMemo(() => Array.from(selectedProjectIds), [selectedProjectIds]);
  const selectedManagedProjectChoices = useMemo(
    () => selectedProjectIdArray
      .map((projectId) => projectChoices.find((project) => project.id === projectId))
      .filter((project): project is ProjectChoice => project !== undefined && canManageProjectChoice(project)),
    [projectChoices, selectedProjectIdArray],
  );
  const selectedTaskIdArray = useMemo(() => Array.from(selectedTaskIds), [selectedTaskIds]);
  const selectedIssueTasks = useMemo(
    () => selectedTaskIdArray
      .map((taskId) => tasks.find((task) => task.id === taskId))
      .filter((task): task is Task => Boolean(task)),
    [selectedTaskIdArray, tasks],
  );
  const selectedActiveTasks = useMemo(
    () => selectedIssueTasks.filter((task) => task.projectId === selectedProjectId && task.archivedAt === null),
    [selectedIssueTasks, selectedProjectId],
  );
  const projectMoveChoices = useMemo(
    () => projectChoices.filter((project) => project.id !== selectedProjectId),
    [projectChoices, selectedProjectId],
  );
  useEffect(() => {
    const selectableProjectIds = new Set(
      projectChoices.filter(canManageProjectChoice).map((project) => project.id),
    );
    setSelectedProjectIds((current) => {
      if (current.size === 0) return current;
      const next = new Set([...current].filter((projectId) => selectableProjectIds.has(projectId)));
      return sameStringSet(next, current) ? current : next;
    });
  }, [projectChoices]);
  const bulkTaskActionBusy = bulkArchivingSelectedTasks
    || bulkDeletingSelectedTasks
    || pendingBulkDeleteTasks.length > 0
    || movingTaskId !== null;
  const bulkTaskActionDisabled = selectedActiveTasks.length === 0 || bulkTaskActionBusy;
  const bulkProjectMoveDisabled = selectedActiveTasks.length === 0 || projectMoveChoices.length === 0 || bulkTaskActionBusy;

  useLayoutEffect(() => {
    if (!bulkProjectMoveMenuOpen || !bulkProjectMoveTriggerRef.current || !bulkProjectMoveMenuRef.current) return;
    const trigger = bulkProjectMoveTriggerRef.current.getBoundingClientRect();
    const menu = bulkProjectMoveMenuRef.current.getBoundingClientRect();
    const left = Math.max(8, Math.min(trigger.right - menu.width, window.innerWidth - menu.width - 8));
    const top = trigger.bottom + 6 + menu.height <= window.innerHeight - 8
      ? trigger.bottom + 6
      : Math.max(8, trigger.top - menu.height - 6);
    setBulkProjectMoveMenuPosition({ left, top, ready: true });
  }, [bulkProjectMoveMenuOpen, projectMoveChoices.length, selectedActiveTasks.length]);

  useEffect(() => {
    if (!bulkProjectMoveMenuOpen) return;
    if (bulkProjectMoveDisabled || boardView !== "issues") {
      setBulkProjectMoveMenuOpen(false);
      return;
    }

    function closeFromOutside(event: PointerEvent) {
      const target = event.target as Node;
      if (
        !bulkProjectMoveMenuRef.current?.contains(target)
        && !bulkProjectMoveTriggerRef.current?.contains(target)
      ) {
        setBulkProjectMoveMenuOpen(false);
      }
    }

    function closeFromViewportChange() {
      setBulkProjectMoveMenuOpen(false);
    }

    function closeFromEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setBulkProjectMoveMenuOpen(false);
        bulkProjectMoveTriggerRef.current?.focus();
      }
    }

    document.addEventListener("pointerdown", closeFromOutside);
    document.addEventListener("keydown", closeFromEscape);
    window.addEventListener("blur", closeFromViewportChange);
    window.addEventListener("resize", closeFromViewportChange);
    window.addEventListener("scroll", closeFromViewportChange, true);
    return () => {
      document.removeEventListener("pointerdown", closeFromOutside);
      document.removeEventListener("keydown", closeFromEscape);
      window.removeEventListener("blur", closeFromViewportChange);
      window.removeEventListener("resize", closeFromViewportChange);
      window.removeEventListener("scroll", closeFromViewportChange, true);
    };
  }, [boardView, bulkProjectMoveDisabled, bulkProjectMoveMenuOpen]);

  const tasksByStatus = useMemo(() => {
    return Object.fromEntries(
      TASK_STATUSES.map((status) => [status, filteredTasks.filter((task) => task.status === status)]),
    ) as Record<TaskStatus, Task[]>;
  }, [filteredTasks]);

  const columnVisibility = columnVisibilityByProject[selectedProjectId];

  const visibleStatuses = useMemo(
    () => TASK_STATUSES.filter((status) => (
      tasksByStatus[status].length === 0
        ? showEmptyColumns
        : (columnVisibility?.[status] ?? true)
    )),
    [columnVisibility, showEmptyColumns, tasksByStatus],
  );

  const hiddenStatuses = useMemo(
    () => TASK_STATUSES.filter((status) => (
      tasksByStatus[status].length === 0
        ? !showEmptyColumns
        : !(columnVisibility?.[status] ?? true)
    )),
    [columnVisibility, showEmptyColumns, tasksByStatus],
  );

  function updateShowEmptyColumns(show: boolean) {
    window.localStorage.setItem(SHOW_EMPTY_COLUMNS_KEY, String(show));
    setShowEmptyColumns(show);
  }

  function updateColumnVisibility(status: TaskStatus, visible: boolean) {
    if (!selectedProjectId || tasksByStatus[status].length === 0) return;
    setColumnVisibilityByProject((current) => {
      const next = {
        ...current,
        [selectedProjectId]: {
          ...current[selectedProjectId],
          [status]: visible,
        },
      };
      window.localStorage.setItem(COLUMN_VISIBILITY_KEY, JSON.stringify(next));
      return next;
    });
  }

  function selectBoardView(view: BoardView) {
    closeContextMenu();
    setBulkProjectMoveMenuOpen(false);
    setPendingBulkDeleteTasks([]);
    setSelectedTaskIds(new Set());
    setBoardView(view);
  }

  function setTaskSelection(task: Task, selected: boolean) {
    setSelectedTaskIds((current) => {
      const next = new Set(current);
      if (selected) next.add(task.id);
      else next.delete(task.id);
      return next;
    });
  }

  function normalizeSelectionBox(startX: number, startY: number, endX: number, endY: number): SelectionBox {
    const left = Math.min(startX, endX);
    const top = Math.min(startY, endY);
    return {
      left,
      top,
      width: Math.max(startX, endX) - left,
      height: Math.max(startY, endY) - top,
    };
  }

  function projectCardIntersectsSelection(card: HTMLElement, box: SelectionBox): boolean {
    const rect = card.getBoundingClientRect();
    return rect.left < box.left + box.width
      && rect.right > box.left
      && rect.top < box.top + box.height
      && rect.bottom > box.top;
  }

  function selectedProjectIdsFromBox(container: HTMLElement, box: SelectionBox): string[] {
    return Array.from(container.querySelectorAll<HTMLElement>("[data-project-id][data-project-selectable='true']"))
      .filter((card) => {
        const projectId = card.dataset.projectId;
        return Boolean(projectId) && projectCardIntersectsSelection(card, box);
      })
      .map((card) => card.dataset.projectId!)
      .filter((projectId, index, projectIds) => projectIds.indexOf(projectId) === index);
  }

  function canStartProjectBoxSelection(target: EventTarget | null): target is HTMLElement {
    if (openingProjectId !== null || projectActionPending !== null) return false;
    if (!(target instanceof HTMLElement)) return false;
    if (
      target.closest(
        "[data-project-id], .project-group-heading, .project-home-bulk-actions, button, input, textarea, select, a, [role='menu'], [contenteditable='true']",
      )
    ) {
      return false;
    }
    return Boolean(target.closest(".project-home-groups"));
  }

  function startProjectBoxSelection(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    if (!canStartProjectBoxSelection(event.target)) return;

    const additive = event.metaKey || event.ctrlKey || event.shiftKey;
    projectSelectionDragRef.current = {
      startX: event.clientX,
      startY: event.clientY,
      pointerId: event.pointerId,
      additive,
      active: false,
      baseIds: new Set(additive ? selectedProjectIds : []),
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function updateProjectBoxSelection(event: ReactPointerEvent<HTMLDivElement>) {
    const dragState = projectSelectionDragRef.current;
    if (!dragState || dragState.pointerId !== event.pointerId) return;

    const box = normalizeSelectionBox(dragState.startX, dragState.startY, event.clientX, event.clientY);
    if (!dragState.active && Math.max(box.width, box.height) < 4) return;

    dragState.active = true;
    event.preventDefault();
    setProjectSelectionBox(box);
    const next = new Set(dragState.additive ? dragState.baseIds : []);
    for (const projectId of selectedProjectIdsFromBox(event.currentTarget, box)) next.add(projectId);
    setSelectedProjectIds(next);
  }

  function finishProjectBoxSelection(event: ReactPointerEvent<HTMLDivElement>) {
    const dragState = projectSelectionDragRef.current;
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    const shouldClearSelection = event.type === "pointerup" && !dragState.active && !dragState.additive;
    projectSelectionDragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setProjectSelectionBox(null);
    if (shouldClearSelection) {
      setSelectedProjectIds((current) => current.size === 0 ? current : new Set());
    }
  }

  function taskCardIntersectsSelection(card: HTMLElement, box: SelectionBox): boolean {
    const rect = card.getBoundingClientRect();
    return rect.left < box.left + box.width
      && rect.right > box.left
      && rect.top < box.top + box.height
      && rect.bottom > box.top;
  }

  function selectedTaskIdsFromBox(container: HTMLElement, box: SelectionBox): string[] {
    return Array.from(container.querySelectorAll<HTMLElement>("[data-task-id]"))
      .filter((card) => {
        const taskId = card.dataset.taskId;
        return Boolean(taskId) && taskCardIntersectsSelection(card, box);
      })
      .map((card) => card.dataset.taskId!)
      .filter((taskId, index, taskIds) => taskIds.indexOf(taskId) === index);
  }

  function canStartBoxSelection(target: EventTarget | null): target is HTMLElement {
    if (!(target instanceof HTMLElement)) return false;
    if (target.closest("[data-task-id], button, input, textarea, select, a, [role='menu'], [contenteditable='true']")) {
      return false;
    }
    if (target.closest(".column-header, .hidden-columns")) return false;
    return Boolean(target.closest(".board-scroll, .board, .column-list"));
  }

  function startBoxSelection(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0 || draggedTaskId) return;
    if (!canStartBoxSelection(event.target)) return;

    const additive = event.metaKey || event.ctrlKey || event.shiftKey;
    selectionDragRef.current = {
      startX: event.clientX,
      startY: event.clientY,
      pointerId: event.pointerId,
      additive,
      active: false,
      baseIds: new Set(additive ? selectedTaskIds : []),
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function updateBoxSelection(event: ReactPointerEvent<HTMLDivElement>) {
    const dragState = selectionDragRef.current;
    if (!dragState || dragState.pointerId !== event.pointerId) return;

    const box = normalizeSelectionBox(dragState.startX, dragState.startY, event.clientX, event.clientY);
    if (!dragState.active && Math.max(box.width, box.height) < 4) return;

    dragState.active = true;
    event.preventDefault();
    setSelectionBox(box);
    const next = new Set(dragState.additive ? dragState.baseIds : []);
    for (const taskId of selectedTaskIdsFromBox(event.currentTarget, box)) next.add(taskId);
    setSelectedTaskIds(next);
  }

  function finishBoxSelection(event: ReactPointerEvent<HTMLDivElement>) {
    const dragState = selectionDragRef.current;
    if (!dragState || dragState.pointerId !== event.pointerId) return;
    const shouldClearSelection = event.type === "pointerup" && !dragState.active && !dragState.additive;
    selectionDragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setSelectionBox(null);
    if (shouldClearSelection) {
      setSelectedTaskIds((current) => current.size === 0 ? current : new Set());
    }
  }

  async function saveEditor(
    draft: TaskDraft,
    attachments: File[],
    inlineImages: PendingInlineImage[],
  ) {
    if (!editor) return;
    const targetProjectId = editor.task?.projectId ?? editor.projectId ?? selectedProjectId;
    if (!targetProjectId) return;
    setActionError(null);
    try {
      const creating = editor.task === null;
      let saved = editor.task
        ? await updateTaskRequest(editor.task, draft)
        : await createTaskRequest(targetProjectId, draft);
      if (creating) {
        setProjects((current) => current.map((project) => (
          project.id === targetProjectId
            ? { ...project, issueCount: project.issueCount + 1 }
            : project
        )));
      }
      let uploadedAttachments = 0;
      let failedAttachments = 0;
      if (creating && (attachments.length > 0 || inlineImages.length > 0)) {
        const [results, inlineAttachments] = await Promise.all([
          Promise.allSettled(
            attachments.map((file) => uploadAttachment(saved.id, file)),
          ),
          Promise.all(
            inlineImages.map((image) => uploadAttachment(saved.id, image.file)),
          ),
        ]);
        uploadedAttachments = results.filter((result) => result.status === "fulfilled").length;
        failedAttachments = results.length - uploadedAttachments;
        if (inlineImages.length > 0) {
          const description = resolveInlineMediaMarkdown(
            draft.description,
            inlineImages,
            inlineAttachments,
          );
          saved = await updateTaskRequest(saved, { ...draft, description });
        }
      }
      setTasks((current) => sortTasks([
        ...current.filter((task) => task.id !== saved.id),
        saved,
      ]));
      setEditor(null);
      if (failedAttachments > 0) {
        setActionError(`${saved.identifier} 已创建，但有 ${failedAttachments} 个附件上传失败，可在详情页重试。`);
      }
      if (creating) {
        const totalUploaded = uploadedAttachments + inlineImages.length;
        const message = `${saved.identifier} 已创建${totalUploaded > 0 ? `，已上传 ${totalUploaded} 个附件` : ""}。`;
        pushUndo(message, async () => {
          const candidate = tasksRef.current.find((task) => task.id === saved.id);
          const current = candidate && candidate.version >= saved.version ? candidate : saved;
          await archiveTaskRequest(current);
          setTasks((tasks) => tasks.filter((task) => task.id !== saved.id));
        });
      } else if (editor.task) {
        const previous = editor.task;
        const previousAssigneeTarget = assigneeTargetForActor(previous.assignee, currentUser);
        if (!draft.assigneeTarget || previousAssigneeTarget) {
          pushUndo(
            `${saved.identifier} 已更新。`,
            () => restoreTaskDetails(previous, saved, previousAssigneeTarget),
          );
        }
      }
    } catch (error) {
      if (error instanceof ApiError && error.code === "VERSION_CONFLICT") {
        void refreshTasks(targetProjectId, { quiet: true });
      }
      throw error;
    }
  }

  async function moveTask(
    task: Task,
    status: TaskStatus,
    beforeTaskId: string | null = null,
    silent = false,
  ) {
    if (movingTaskId) {
      setDropTarget(null);
      setDraggedTaskId(null);
      setDraggedTaskIds([]);
      setDraggedTaskHeight(0);
      return;
    }

    const destination = tasks.filter((candidate) => candidate.status === status && candidate.id !== task.id);
    const insertionIndex = beforeTaskId
      ? destination.findIndex((candidate) => candidate.id === beforeTaskId)
      : destination.length;
    const targetIndex = insertionIndex < 0 ? destination.length : insertionIndex;
    const desiredOrder = [...destination];
    desiredOrder.splice(targetIndex, 0, task);
    const currentOrder = tasks.filter((candidate) => candidate.status === status);
    if (
      task.status === status
      && currentOrder.length === desiredOrder.length
      && currentOrder.every((candidate, index) => candidate.id === desiredOrder[index].id)
    ) {
      setDropTarget(null);
      setDraggedTaskId(null);
      setDraggedTaskIds([]);
      setDraggedTaskHeight(0);
      return;
    }
    const previousTask = destination[targetIndex - 1] ?? null;
    const nextTask = destination[targetIndex] ?? null;
    const sortOrder = previousTask && nextTask
      ? (previousTask.sortOrder + nextTask.sortOrder) / 2
      : previousTask
        ? previousTask.sortOrder + 1024
        : nextTask
          ? nextTask.sortOrder - 1024
          : 1024;
    const previous = task;
    setActionError(null);
    setMovingTaskId(task.id);
    setTasks((current) => sortTasks(current.map((candidate) =>
      candidate.id === task.id ? { ...candidate, status, sortOrder } : candidate,
    )));

    try {
      const moved = await moveTaskRequest(task, status, sortOrder);
      setTasks((current) => sortTasks(current.map((candidate) =>
        candidate.id === moved.id ? moved : candidate,
      )));
      const message = task.status === status
        ? `${task.identifier} 排序已调整。`
        : `${task.identifier} 已移至${STATUS_DETAILS[status].label}。`;
      pushUndo(message, async () => {
        const candidate = tasksRef.current.find((current) => current.id === moved.id);
        const current = candidate && candidate.version >= moved.version ? candidate : moved;
        const restored = await moveTaskRequest(current, previous.status, previous.sortOrder);
        setTasks((tasks) => sortTasks(tasks.map((item) => item.id === restored.id ? restored : item)));
      }, !silent);
    } catch (error) {
      setTasks((current) => sortTasks(current.map((candidate) =>
        candidate.id === previous.id ? previous : candidate,
      )));
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "That task changed elsewhere. The board has been refreshed."
        : errorMessage(error));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    } finally {
      setMovingTaskId(null);
      setDropTarget(null);
      setDraggedTaskId(null);
      setDraggedTaskIds([]);
      setDraggedTaskHeight(0);
    }
  }

  async function moveTaskGroup(
    taskGroup: Task[],
    status: TaskStatus,
    beforeTaskId: string | null = null,
    silent = false,
  ) {
    if (movingTaskId) {
      setDropTarget(null);
      setDraggedTaskId(null);
      setDraggedTaskIds([]);
      setDraggedTaskHeight(0);
      return;
    }

    const uniqueTasks = [...new Map(taskGroup.map((task) => [task.id, task])).values()]
      .sort(compareTasksByBoardPosition);
    if (uniqueTasks.length <= 1) {
      if (uniqueTasks[0]) await moveTask(uniqueTasks[0], status, beforeTaskId, silent);
      return;
    }

    const movingIds = new Set(uniqueTasks.map((task) => task.id));
    const destination = tasks.filter((candidate) => candidate.status === status && !movingIds.has(candidate.id));
    const insertionIndex = beforeTaskId && !movingIds.has(beforeTaskId)
      ? destination.findIndex((candidate) => candidate.id === beforeTaskId)
      : destination.length;
    const targetIndex = insertionIndex < 0 ? destination.length : insertionIndex;
    const desiredOrder = [...destination];
    desiredOrder.splice(targetIndex, 0, ...uniqueTasks);
    const currentOrder = tasks.filter((candidate) => candidate.status === status);
    if (
      uniqueTasks.every((task) => task.status === status)
      && currentOrder.length === desiredOrder.length
      && currentOrder.every((candidate, index) => candidate.id === desiredOrder[index].id)
    ) {
      setDropTarget(null);
      setDraggedTaskId(null);
      setDraggedTaskIds([]);
      setDraggedTaskHeight(0);
      return;
    }

    const sortOrders = calculateGroupSortOrders(
      destination[targetIndex - 1] ?? null,
      destination[targetIndex] ?? null,
      uniqueTasks.length,
    );
    const previousTasks = new Map(uniqueTasks.map((task) => [task.id, task]));
    const optimisticTasks = new Map(uniqueTasks.map((task, index) => [
      task.id,
      { ...task, status, sortOrder: sortOrders[index] },
    ]));
    setActionError(null);
    setMovingTaskId(uniqueTasks[0].id);
    setTasks((current) => sortTasks(current.map((candidate) =>
      optimisticTasks.get(candidate.id) ?? candidate,
    )));

    try {
      const movedTasks = await Promise.all(
        uniqueTasks.map((task, index) => moveTaskRequest(task, status, sortOrders[index])),
      );
      const movedById = new Map(movedTasks.map((task) => [task.id, task]));
      setTasks((current) => sortTasks(current.map((candidate) =>
        movedById.get(candidate.id) ?? candidate,
      )));
      const movedWithinStatus = uniqueTasks.every((task) => task.status === status);
      const message = movedWithinStatus
        ? `${uniqueTasks.length} 个任务排序已调整。`
        : `${uniqueTasks.length} 个任务已移至${STATUS_DETAILS[status].label}。`;
      pushUndo(message, async () => {
        const restoredTasks = await Promise.all(movedTasks.map((moved) => {
          const previous = previousTasks.get(moved.id);
          const candidate = tasksRef.current.find((current) => current.id === moved.id);
          const current = candidate && candidate.version >= moved.version ? candidate : moved;
          return previous ? moveTaskRequest(current, previous.status, previous.sortOrder) : Promise.resolve(current);
        }));
        const restoredById = new Map(restoredTasks.map((task) => [task.id, task]));
        setTasks((tasks) => sortTasks(tasks.map((item) => restoredById.get(item.id) ?? item)));
      }, !silent);
    } catch (error) {
      setTasks((current) => sortTasks(current.map((candidate) =>
        previousTasks.get(candidate.id) ?? candidate,
      )));
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "That task changed elsewhere. The board has been refreshed."
        : errorMessage(error));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    } finally {
      setMovingTaskId(null);
      setDropTarget(null);
      setDraggedTaskId(null);
      setDraggedTaskIds([]);
      setDraggedTaskHeight(0);
    }
  }

  async function moveSelectedTasksToProject(projectId: string) {
    if (!selectedProjectId || bulkTaskActionBusy || projectId === selectedProjectId) return;
    setBulkProjectMoveMenuOpen(false);
    const targetChoice = projectMoveChoices.find((project) => project.id === projectId);
    if (!targetChoice) return;
    const taskGroup = [...selectedActiveTasks]
      .sort(compareTasksByBoardPosition);
    if (taskGroup.length === 0) return;

    const previousTasks = new Map(taskGroup.map((task) => [task.id, task]));
    setActionError(null);
    setMovingTaskId(taskGroup[0].id);
    setTasks((current) => current.filter((candidate) => !previousTasks.has(candidate.id)));

    try {
      const targetProject = await ensureProject(targetChoice);
      const movedTasks = await Promise.all(
        taskGroup.map((task) => moveTaskRequest(task, task.status, undefined, { projectId: targetProject.id })),
      );
      setSelectedTaskIds(new Set());
      const targetName = projectDisplayName(targetProject);
      pushUndo(`${taskGroup.length} 个任务已移至${targetName}。`, async () => {
        await Promise.all(movedTasks.map((moved) => {
          const previous = previousTasks.get(moved.id);
          return previous
            ? moveTaskRequest(moved, previous.status, previous.sortOrder, { projectId: previous.projectId })
            : Promise.resolve(moved);
        }));
        await refreshProjectList();
        const currentProjectId = selectedProjectIdRef.current;
        if (currentProjectId) await refreshTasks(currentProjectId, { quiet: true });
      });
      await refreshProjectList();
      await refreshTasks(selectedProjectId, { quiet: true });
    } catch (error) {
      setTasks((current) => sortTasks([
        ...current.filter((candidate) => !previousTasks.has(candidate.id)),
        ...taskGroup,
      ]));
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "That task changed elsewhere. The board has been refreshed."
        : errorMessage(error));
      void refreshProjectList();
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    } finally {
      setMovingTaskId(null);
      setDropTarget(null);
      setDraggedTaskId(null);
      setDraggedTaskIds([]);
      setDraggedTaskHeight(0);
    }
  }

  async function archiveSelectedTasks() {
    if (!selectedProjectId || bulkTaskActionBusy || selectedActiveTasks.length === 0) return;
    setBulkProjectMoveMenuOpen(false);
    const taskGroup = [...selectedActiveTasks].sort(compareTasksByBoardPosition);
    const previousTasks = new Map(taskGroup.map((task) => [task.id, task]));
    setActionError(null);
    setBulkArchivingSelectedTasks(true);
    setTasks((current) => current.filter((candidate) => !previousTasks.has(candidate.id)));

    try {
      const archivedTasks = await Promise.all(taskGroup.map((task) => archiveTaskRequest(task)));
      setSelectedTaskIds(new Set());
      pushUndo(`${taskGroup.length} 个任务已归档。`, async () => {
        await Promise.all(archivedTasks.map((archived) => restoreTaskRequest(archived)));
        await refreshProjectList();
        const currentProjectId = selectedProjectIdRef.current;
        if (currentProjectId) await refreshTasks(currentProjectId, { quiet: true });
      });
      await refreshProjectList();
      await refreshTasks(selectedProjectId, { quiet: true });
    } catch (error) {
      setTasks((current) => sortTasks([
        ...current.filter((candidate) => !previousTasks.has(candidate.id)),
        ...taskGroup,
      ]));
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "有任务已在其他位置更新，任务面板已重新同步。"
        : errorMessage(error));
      void refreshProjectList();
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    } finally {
      setBulkArchivingSelectedTasks(false);
    }
  }

  function requestBulkTaskDelete() {
    if (bulkTaskActionBusy || selectedActiveTasks.length === 0) return;
    setBulkProjectMoveMenuOpen(false);
    setPendingBulkDeleteTasks([...selectedActiveTasks].sort(compareTasksByBoardPosition));
  }

  async function confirmBulkTaskDelete() {
    if (pendingBulkDeleteTasks.length === 0 || bulkDeletingSelectedTasks) return;
    const taskGroup = pendingBulkDeleteTasks;
    const previousTasks = new Map(taskGroup.map((task) => [task.id, task]));
    setActionError(null);
    setBulkDeletingSelectedTasks(true);
    setTasks((current) => current.filter((candidate) => !previousTasks.has(candidate.id)));

    try {
      await Promise.all(taskGroup.map((task) => deleteTaskRequest(task)));
      setSelectedTaskIds(new Set());
      setPendingBulkDeleteTasks([]);
      if (taskGroup.some((task) => task.identifier === detailTaskIdentifier)) closeTaskDetail();
      setAnnouncement(`${taskGroup.length} 个任务已永久删除。`);
      await refreshProjectList();
      if (selectedProjectId) await refreshTasks(selectedProjectId, { quiet: true });
    } catch (error) {
      setTasks((current) => sortTasks([
        ...current.filter((candidate) => !previousTasks.has(candidate.id)),
        ...taskGroup,
      ]));
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "有任务已在其他位置更新，任务面板已重新同步。"
        : errorMessage(error));
      void refreshProjectList();
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    } finally {
      setBulkDeletingSelectedTasks(false);
    }
  }

  function finishTaskDrop(destination: TaskStatus, taskId: string, beforeTaskId: string | null = null) {
    const task = tasks.find((candidate) => candidate.id === taskId);
    setDraggedTaskId(null);
    setDraggedTaskIds([]);
    setDraggedTaskHeight(0);
    setDropTarget(null);
    if (!task) return;
    const selectedTasks = selectedTaskIds.has(taskId)
      ? selectedTaskIdArray
        .map((selectedTaskId) => tasks.find((candidate) => candidate.id === selectedTaskId))
        .filter((candidate): candidate is Task => Boolean(candidate))
      : [];
    const movingTasks = selectedTasks.some((candidate) => candidate.id === task.id)
      ? selectedTasks
      : [task, ...selectedTasks];
    setSettlingTaskId(task.id);
    window.setTimeout(() => {
      setSettlingTaskId((current) => current === task.id ? null : current);
    }, 220);
    if (movingTasks.length > 1) {
      void moveTaskGroup(movingTasks, destination, beforeTaskId, true);
      return;
    }
    void moveTask(task, destination, beforeTaskId, true);
  }

  async function updateTaskProperties(task: Task, changes: Partial<TaskDraft>, message?: string): Promise<Task> {
    const previous = task;
    const { assigneeTarget, ...taskChanges } = changes;
    const optimisticAssignee = assigneeTarget
      ? actorForAssigneeTarget(assigneeTarget, currentUser)
      : task.assignee;
    setActionError(null);
    setTasks((current) => current.map((candidate) =>
      candidate.id === task.id
        ? { ...candidate, ...taskChanges, assignee: optimisticAssignee }
        : candidate,
    ));

    try {
      const updated = await updateTaskRequest(task, { ...taskToDraft(task), ...changes });
      setTasks((current) => sortTasks(current.map((candidate) =>
        candidate.id === updated.id ? updated : candidate,
      )));
      const previousAssigneeTarget = assigneeTargetForActor(previous.assignee, currentUser);
      if (!assigneeTarget || previousAssigneeTarget) {
        pushUndo(
          message ?? `${task.identifier} 已更新。`,
          () => restoreTaskDetails(previous, updated, previousAssigneeTarget),
        );
      }
      return updated;
    } catch (error) {
      setTasks((current) => sortTasks(current.map((candidate) =>
        candidate.id === previous.id ? previous : candidate,
      )));
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "该任务已在其他位置更新，看板已重新同步。"
        : errorMessage(error));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
      throw error;
    }
  }

  async function mutateTaskRelation(
    action: "add" | "remove",
    task: Task,
    type: IssueRelationType,
    relatedTaskId: string,
  ) {
    setActionError(null);
    try {
      const result = action === "add"
        ? await addTaskRelation(task, type, relatedTaskId)
        : await removeTaskRelation(task, type, relatedTaskId);
      setTasks((current) => sortTasks(current.map((candidate) => {
        if (candidate.id === result.task.id) return result.task;
        if (candidate.id === result.relatedTask.id) return result.relatedTask;
        return candidate;
      })));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
      return result;
    } catch (error) {
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "该任务已在其他位置更新，看板已重新同步。"
        : errorMessage(error));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
      throw error;
    }
  }

  async function duplicateTask(task: Task) {
    setActionError(null);
    try {
      const duplicated = await createTaskRequest(task.projectId, {
        ...taskToDraft(task),
        assigneeTarget: assigneeTargetForActor(task.assignee, currentUser),
        developmentContext: null,
      });
      setTasks((current) => sortTasks([...current, duplicated]));
      pushUndo(`${duplicated.identifier} 副本已创建。`, async () => {
        const candidate = tasksRef.current.find((current) => current.id === duplicated.id);
        const current = candidate && candidate.version >= duplicated.version ? candidate : duplicated;
        await archiveTaskRequest(current);
        setTasks((tasks) => tasks.filter((item) => item.id !== duplicated.id));
      });
    } catch (error) {
      setActionError(errorMessage(error));
    }
  }

  async function archiveTask(task: Task) {
    setActionError(null);
    try {
      const archived = await archiveTaskRequest(task);
      setTasks((current) => current.filter((candidate) => candidate.id !== task.id));
      pushUndo(`${task.identifier} 已归档。`, async () => {
        const restored = await restoreTaskRequest(archived);
        setTasks((current) => sortTasks([
          ...current.filter((candidate) => candidate.id !== restored.id),
          restored,
        ]));
      });
    } catch (error) {
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "该任务已在其他位置更新，看板已重新同步。"
        : errorMessage(error));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    }
  }

  async function deleteTask(task: Task) {
    setActionError(null);
    try {
      await deleteTaskRequest(task);
      setTasks((current) => current.filter((candidate) => candidate.id !== task.id));
      if (detailTaskIdentifier === task.identifier) closeTaskDetail();
      setAnnouncement(`${task.identifier} 已永久删除。`);
    } catch (error) {
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "该任务已在其他位置更新，看板已重新同步。"
        : errorMessage(error));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    }
  }

  async function restoreArchivedTask(task: Task) {
    if (restoringArchivedTaskId) return;
    setRestoringArchivedTaskId(task.id);
    setActionError(null);
    try {
      const restored = await restoreTaskRequest(task);
      setTasks((current) => current.filter((candidate) => candidate.id !== restored.id));
      if (detailTaskIdentifier === task.identifier) closeTaskDetail();
      setAnnouncement(`${task.identifier} 已恢复到任务看板。`);
    } catch (error) {
      setActionError(error instanceof ApiError && error.code === "VERSION_CONFLICT"
        ? "该任务已在其他位置更新，看板已重新同步。"
        : errorMessage(error));
      if (selectedProjectId) void refreshTasks(selectedProjectId, { quiet: true });
    } finally {
      setRestoringArchivedTaskId(null);
    }
  }

  async function confirmArchivedTaskDelete() {
    if (!pendingArchivedDeleteTask || deletingArchivedTask) return;
    setDeletingArchivedTask(true);
    try {
      await deleteTask(pendingArchivedDeleteTask);
      setPendingArchivedDeleteTask(null);
    } finally {
      setDeletingArchivedTask(false);
    }
  }

  async function copyText(text: string, message: string) {
    try {
      await navigator.clipboard.writeText(text);
      setAnnouncement(message);
    } catch {
      setActionError("无法写入剪贴板。");
    }
  }

  function resolveThreadWorkspacePath(task?: Task | null) {
    if (task?.developmentContext?.type === "worktree") return task.developmentContext.path;
    return developmentScan.workspacePath ?? selectedDeviceWorkspacePath ?? hostContext?.workspacePath ?? null;
  }

  async function resolveCodexThreadTitle(threadId: string, fallbackTitle = "") {
    const codexThreadId = threadId.trim();
    const fallback = fallbackTitle.trim();
    if (!codexThreadId) return fallback;
    const cached = codexThreadTitleCacheRef.current.get(codexThreadId);
    if (cached !== undefined) return cached || fallback;
    try {
      const title = (await getCodexThreadTitle(codexThreadId))?.trim() ?? "";
      codexThreadTitleCacheRef.current.set(codexThreadId, title);
      return title || fallback;
    } catch {
      return fallback;
    }
  }

  async function openThread(threadId: string, workspacePath?: string, threadTitle?: string) {
    const codexThreadId = threadId.trim();
    if (!codexThreadId) return;
    const resolvedWorkspacePath = workspacePath?.trim() || resolveThreadWorkspacePath(detailTask);
    const resolvedThreadTitle = await resolveCodexThreadTitle(codexThreadId, threadTitle);
    if (embedded && window.parent !== window) {
      window.parent.postMessage({
        type: "taskboard:open-thread",
        payload: {
          threadId: codexThreadId,
          ...(resolvedWorkspacePath ? { workspacePath: resolvedWorkspacePath } : {}),
          ...(resolvedThreadTitle ? { threadTitle: resolvedThreadTitle } : {}),
        },
      }, "*");
      return;
    }

    window.location.assign(`codex://threads/${encodeURIComponent(codexThreadId)}`);
  }

  async function continueThreadInPanel(task: Task, threadId: string) {
    const codexThreadId = threadId.trim();
    if (!codexThreadId) return;
    const workspacePath = resolveThreadWorkspacePath(task);
    const threadTitle = await resolveCodexThreadTitle(codexThreadId, task.title);
    if (embedded && window.parent !== window) {
      window.parent.postMessage({
        type: "taskboard:show-thread-panel",
        payload: {
          threadId: codexThreadId,
          title: `${task.identifier} ${task.title}`.slice(0, 160),
          threadTitle,
          ...(workspacePath ? { workspacePath } : {}),
        },
      }, "*");
      setAnnouncement(`${task.identifier} 已在当前页面打开对话。`);
      return;
    }

    void openThread(codexThreadId, workspacePath ?? undefined, threadTitle);
  }

  function expandCodexSidebar() {
    if (!embedded || window.parent === window) return;
    window.parent.postMessage({ type: "taskboard:expand-sidebar" }, "*");
  }

  function openTaskInThread(task: Task) {
    if (!manageTaskboardSkillPath) {
      setActionError("任务面板还没有读取到 manage-taskboard Skill 路径，请刷新后重试。");
      return;
    }
    const worktreePath = task.developmentContext?.type === "worktree"
      ? task.developmentContext.path
      : null;
    const workspacePath = worktreePath
      ?? selectedDeviceWorkspacePath
      ?? developmentScan.workspacePath
      ?? hostContext?.workspacePath;
    const instruction = `e-taskboard Addressing the tasks mentioned in ${task.identifier}`;
    const prompt = `[$manage-taskboard](${manageTaskboardSkillPath}) ${instruction}`;

    if (!embedded || window.parent === window) {
      const query = new URLSearchParams();
      if (workspacePath) query.set("path", workspacePath);
      query.set("prompt", prompt);
      window.location.assign(`codex://new?${query.toString().replace(/\+/g, "%20")}`);
      return;
    }
    if (openingThreadTaskId) return;
    const codexProject = hostContext?.projects?.find((project) => project.id === selectedProject?.id);
    setOpeningThreadTaskId(task.id);
    setActionError(null);
    window.parent.postMessage({
      type: "taskboard:create-thread",
      payload: {
        taskId: task.id,
        identifier: task.identifier,
        instruction,
        skillName: "manage-taskboard",
        skillDisplayName: "Manage Taskboard",
        skillPath: manageTaskboardSkillPath,
        codexProjectId: codexProject?.id ?? (
          selectedProject?.id === DEFAULT_PROJECT_ID ? hostContext?.projectId : selectedProject?.id
        ),
        projectName: selectedProject ? selectedProjectDisplayName : undefined,
        workspacePath,
        workspaceLabel: worktreePath ? workspaceName(worktreePath) : undefined,
      },
    }, "*");
  }

  function changeProject(projectId: string) {
    closeContextMenu();
    setProjectMenuOpen(false);
    setPendingBulkDeleteTasks([]);
    setDetailTaskIdentifier(null);
    setSelectedProjectIds(new Set());
    setProjectSelectionBox(null);
    setSelectedTaskIds(new Set());
    setBoardView("issues");
    setSelectedProjectId(projectId);
    window.localStorage.setItem(LAST_PROJECT_KEY, projectId);
    setSearch("");
    setFilters(EMPTY_TASK_FILTERS);
    setActionError(null);
    undoStackRef.current = [];
    setUndoNotice(null);
    const url = buildIssueUrl(window.location.href, projectId, null);
    window.history.replaceState(null, "", url);
  }

  function returnToProjectHome() {
    closeContextMenu();
    setProjectMenuOpen(false);
    setPendingBulkDeleteTasks([]);
    setDetailTaskIdentifier(null);
    setSelectedProjectIds(new Set());
    setProjectSelectionBox(null);
    setSelectedTaskIds(new Set());
    setSelectedProjectId("");
    window.localStorage.removeItem(LAST_PROJECT_KEY);
    setSearch("");
    setFilters(EMPTY_TASK_FILTERS);
    setActionError(null);
    undoStackRef.current = [];
    setUndoNotice(null);
    const url = buildIssueUrl(window.location.href, null, null);
    window.history.replaceState(null, "", url);
    void loadProjectList();
  }

  function toggleFavoriteProject() {
    if (!selectedProjectId) return;
    const shouldFavorite = !favoriteProjectIds.has(selectedProjectId);
    setFavoriteProjectIds((current) => {
      const next = new Set(current);
      if (shouldFavorite) next.add(selectedProjectId);
      else next.delete(selectedProjectId);
      window.localStorage.setItem(FAVORITE_PROJECTS_KEY, JSON.stringify([...next]));
      return next;
    });
    setAnnouncement(`${selectedProjectDisplayName || "项目"}${shouldFavorite ? "已收藏。" : "已取消收藏。"}`);
  }

  async function ensureProject(choice: ProjectChoice): Promise<Project> {
    const existing = projects.find((candidate) => candidate.id === choice.id);
    if (existing) return existing;
    let workspacePath = choice.workspacePath;
    if (!workspacePath) {
      try {
        const workspaces = await listDeviceWorkspaces();
        mergeDeviceWorkspacePaths(workspaces);
        workspacePath = workspaces[choice.id] ?? null;
      } catch {}
    }
    try {
      const project = await createProjectRequest({
        id: choice.id,
        name: choice.name,
        workspacePath,
      });
      setProjects((current) => (
        current.some((candidate) => candidate.id === project.id) ? current : [...current, project]
      ));
      return project;
    } catch (error) {
      if (!(error instanceof ApiError) || error.code !== "PROJECT_EXISTS") throw error;
      const nextProjects = await listProjects();
      setProjects(nextProjects);
      const project = nextProjects.find((candidate) => candidate.id === choice.id) ?? null;
      if (!project) throw error;
      return project;
    }
  }

  async function selectProject(choice: ProjectChoice, options: { createIssue?: boolean } = {}) {
    if (openingProjectId) return;
    setOpeningProjectId(choice.id);
    setActionError(null);
    try {
      const project = await ensureProject(choice);
      changeProject(project.id);
      if (options.createIssue) {
        setEditor({ task: null, status: "backlog", projectId: project.id });
      }
    } catch (error) {
      setActionError(errorMessage(error));
    } finally {
      setOpeningProjectId(null);
    }
  }

  async function mutateProjects(projectIds: string[], action: "archive" | "delete") {
    if (projectActionPending !== null) return;
    const requestedIds = new Set(projectIds);
    const targets = projectChoices.filter((project) => (
      requestedIds.has(project.id) && canManageProjectChoice(project)
    ));
    if (targets.length === 0) return;

    setActionError(null);
    setProjectActionPending(action);
    const completedIds: string[] = [];
    let completedAll = false;
    try {
      for (const project of targets) {
        if (action === "archive") await archiveProjectRequest(project.id);
        else await deleteProjectRequest(project.id);
        completedIds.push(project.id);
      }
      completedAll = true;
    } catch (error) {
      setActionError(errorMessage(error));
    } finally {
      if (completedIds.length > 0) {
        forgetProjects(completedIds);
        setSelectedProjectIds((current) => {
          const completed = new Set(completedIds);
          const next = new Set([...current].filter((projectId) => !completed.has(projectId)));
          return sameStringSet(next, current) ? current : next;
        });
      }
      setProjectSelectionBox(null);
      await refreshProjectList();
      setProjectActionPending(null);
    }

    if (completedAll) {
      setAnnouncement(`${targets.length} 个项目已${action === "archive" ? "归档" : "删除"}。`);
    }
  }

  const contextName = workspaceName(hostContext?.workspacePath);
  const headerProjectName = selectedProjectDisplayName || "任务面板";
  const appShellStyle = embedded
    ? { "--codex-titlebar-left-inset": `${hostContext?.titlebarLeftInset ?? 0}px` } as CSSProperties
    : undefined;
  const bulkProjectMoveLabel = selectedActiveTasks.length > 0
    ? `移动 ${selectedActiveTasks.length} 个任务`
    : "移动到项目";
  const bulkArchiveLabel = bulkArchivingSelectedTasks
    ? "归档中..."
    : selectedActiveTasks.length > 0
      ? `归档 ${selectedActiveTasks.length} 个`
      : "归档";
  const bulkDeleteLabel = bulkDeletingSelectedTasks
    ? "删除中..."
    : selectedActiveTasks.length > 0
      ? `删除 ${selectedActiveTasks.length} 个`
      : "删除";
  const selectedProjectActionIds = selectedManagedProjectChoices.map((project) => project.id);
  const projectActionBusy = projectActionPending !== null;
  const projectArchiveLabel = projectActionPending === "archive" ? "归档中..." : "归档";
  const projectDeleteLabel = projectActionPending === "delete" ? "删除中..." : "删除";
  const bulkProjectMoveMenu = bulkProjectMoveMenuOpen ? createPortal(
    <div
      ref={bulkProjectMoveMenuRef}
      className="bulk-project-move-menu"
      role="menu"
      aria-label="移动到项目"
      style={{
        left: bulkProjectMoveMenuPosition.left,
        top: bulkProjectMoveMenuPosition.top,
        visibility: bulkProjectMoveMenuPosition.ready ? "visible" : "hidden",
      }}
    >
      <span>移动到项目</span>
      {projectMoveChoices.map((project) => (
        <button
          key={project.id}
          type="button"
          role="menuitem"
          disabled={movingTaskId !== null}
          onClick={() => void moveSelectedTasksToProject(project.id)}
        >
          <span className="project-avatar" aria-hidden="true">{project.name.slice(0, 1).toUpperCase()}</span>
          <span>{project.name}</span>
        </button>
      ))}
    </div>,
    document.body,
  ) : null;

  return (
    <div className={`app-shell${embedded ? " embedded" : ""}`} style={appShellStyle}>
      {taskboardMetadata && taskboardMetadata.mode !== "cloud" && (
        <LocalRealtimeSync
          selectedProjectId={selectedProjectId}
          detailTaskId={detailTaskId}
          refreshProjectList={refreshProjectList}
          refreshTasks={refreshTasks}
          refreshWorkflowOptions={refreshWorkflowOptions}
          setConnection={setConnection}
          setCommentsRevision={setCommentsRevision}
          setAttachmentsRevision={setAttachmentsRevision}
        />
      )}
      {!embedded && (
        <aside className="app-nav" aria-label="Taskboard navigation">
          <div className="brand-row">
            <span className="brand-mark" aria-hidden="true"><LinearIcon name="project" /></span>
            <span>任务面板</span>
          </div>

          <nav className="primary-nav" aria-label="Views">
            <span className="nav-label">工作区</span>
            <button className="nav-item active" type="button" aria-current="page">
              <span className="nav-glyph" aria-hidden="true">
                <LinearIcon name="myIssues" />
              </span>
              任务
              <span className="nav-count">{tasks.length}</span>
            </button>
          </nav>

          <div className="project-nav">
            <span className="nav-label">项目</span>
            {projects.map((project) => {
              const projectName = projectDisplayName(project);
              return (
                <button
                  key={project.id}
                  type="button"
                  className={`project-nav-item${selectedProjectId === project.id ? " active" : ""}`}
                  onClick={() => changeProject(project.id)}
                >
                  <span className="project-dot" aria-hidden="true" />
                  <span>{projectName}</span>
                </button>
              );
            })}
          </div>

          <div className="nav-spacer" />
          <div className="nav-footer">
            <div className={`connection connection-${connection}`}>
              <span aria-hidden="true" />
              {connection === "live" ? "实时同步" : "正在重新连接…"}
            </div>
            <button
              type="button"
              className="theme-toggle"
              onClick={() => setTheme((current) => current === "dark" ? "light" : "dark")}
              aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
            >
              <span aria-hidden="true"><LinearIcon name={theme === "dark" ? "sun" : "moon"} /></span>
              {theme === "dark" ? "浅色模式" : "深色模式"}
            </button>
          </div>
        </aside>
      )}

      <main className="workspace">
        {selectedProjectId ? (
          <header className="workspace-header">
          <div className="workspace-title">
            <div className="workspace-kicker">
              {detailTask && (
                <button
                  className="detail-back-button"
                  type="button"
                  aria-label="返回任务看板"
                  title="返回任务看板 (Esc)"
                  onClick={closeTaskDetail}
                >
                  <LinearIcon name="chevronLeft" />
                </button>
              )}
              {embedded && hostContext?.sidebarCollapsed && (
                <button
                  className="detail-back-button codex-sidebar-expand-button"
                  type="button"
                  aria-label="展开 Codex 侧边栏"
                  title="展开侧边栏"
                  onClick={expandCodexSidebar}
                >
                  <LinearIcon name="codexSidebarExpand" />
                </button>
              )}
              {selectedProjectId && (
                <button
                  className="detail-back-button project-home-button"
                  type="button"
                  aria-label="返回项目首页"
                  title="返回项目首页"
                  onClick={returnToProjectHome}
                >
                  <LinearIcon name="home" />
                  <span>首页</span>
                </button>
              )}
              {selectedProjectId && <span className="breadcrumb-chevron" aria-hidden="true"><LinearIcon name="chevronRight" /></span>}
              {selectedProjectId ? (
                <div className="header-project-switcher" data-project-switcher>
                  <button
                    className="header-project-button"
                    type="button"
                    aria-label="切换项目"
                    aria-haspopup="menu"
                    aria-expanded={projectMenuOpen}
                    onClick={() => setProjectMenuOpen((current) => !current)}
                  >
                    <span className="project-avatar" aria-hidden="true">
                      {headerProjectName.slice(0, 1).toUpperCase()}
                    </span>
                    <span className="project-name">{headerProjectName}</span>
                    <LinearIcon className="project-switcher-chevron" name="chevronDown" />
                  </button>
                  {projectMenuOpen && (
                    <div className="header-project-menu" role="menu" aria-label="项目">
                      <span>切换项目</span>
                      {projectChoices.map((project) => (
                        <button
                          type="button"
                          role="menuitemradio"
                          aria-checked={project.id === selectedProjectId}
                          disabled={openingProjectId !== null}
                          key={project.id}
                          onClick={() => {
                            if (project.id === selectedProjectId) setProjectMenuOpen(false);
                            else void selectProject(project);
                          }}
                        >
                          <span className="project-avatar" aria-hidden="true">{project.name.slice(0, 1).toUpperCase()}</span>
                          <span>{project.name}</span>
                          {favoriteProjectIds.has(project.id) && <span className="project-menu-favorite" aria-label="已收藏"><LinearIcon name="favorite" /></span>}
                          {project.id === selectedProjectId && <span className="project-menu-check" aria-hidden="true"><LinearIcon name="check" /></span>}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              ) : (
                <>
                  <span className="project-avatar" aria-hidden="true">
                    {headerProjectName.slice(0, 1).toUpperCase()}
                  </span>
                  <span className="project-name">{headerProjectName}</span>
                </>
              )}
              {!selectedProjectId && (
                <>
                  <span className="breadcrumb-chevron" aria-hidden="true"><LinearIcon name="chevronRight" /></span>
                  <strong>项目</strong>
                </>
              )}
              {!detailTask && selectedProjectId && (
                <button
                  className={`favorite-button${favoriteProjectIds.has(selectedProjectId) ? " active" : ""}`}
                  type="button"
                  aria-label={favoriteProjectIds.has(selectedProjectId) ? "取消收藏项目" : "收藏项目"}
                  aria-pressed={favoriteProjectIds.has(selectedProjectId)}
                  title={favoriteProjectIds.has(selectedProjectId) ? "取消收藏" : "收藏项目"}
                  onClick={toggleFavoriteProject}
                >
                  <LinearIcon className="favorite-icon" name="favorite" />
                </button>
              )}
              {!detailTask && selectedProjectId && embedded && contextName && <span className="codex-context">{contextName}</span>}
            </div>
          </div>

          <div ref={dragRegionRef} className="workspace-drag-region" aria-hidden="true" />

          <div className="header-actions">
            {selectedProjectId && (
              <ProjectAutomationMenu
                automation={selectedProjectAutomation}
                pending={automationPending}
                error={automationError}
                unavailableReason={automationProjectContext.unavailableReason}
                onOpen={() => void reconcileProjectAutomation()}
                onChange={(options) => void saveProjectAutomation(options)}
              />
            )}
            {selectedProjectId && (boardView === "issues" || boardView === "hierarchy") && (
              <button
                className="icon-button header-create-button"
                type="button"
                onClick={() => setEditor({ task: null, status: "backlog" })}
                aria-label="新建任务"
                title="新建任务 (C)"
              >
                <LinearIcon name="plus" />
              </button>
            )}
          </div>
          </header>
        ) : (
          <div ref={dragRegionRef} className="home-window-drag-region" aria-hidden="true" />
        )}

        {selectedProjectId && !detailTask && <div className="board-toolbar">
          <div className="view-tabs" aria-label="看板视图">
            <button
              className={`view-tab${boardView === "issues" ? " active" : ""}`}
              type="button"
              aria-pressed={boardView === "issues"}
              onClick={() => selectBoardView("issues")}
            >
              任务看板
            </button>
            <button
              className={`view-tab${boardView === "hierarchy" ? " active" : ""}`}
              type="button"
              aria-pressed={boardView === "hierarchy"}
              onClick={() => selectBoardView("hierarchy")}
            >
              层级视图
            </button>
            <button
              className={`view-tab${boardView === "archive" ? " active" : ""}`}
              type="button"
              aria-pressed={boardView === "archive"}
              onClick={() => selectBoardView("archive")}
            >
              已归档
            </button>
            {SHOW_WORKFLOW_BOARD_ENTRY && (
              <button
                className={`view-tab${boardView === "workflow" ? " active" : ""}`}
                type="button"
                aria-pressed={boardView === "workflow"}
                onClick={() => selectBoardView("workflow")}
              >
                节点模式
              </button>
            )}
          </div>
          {(boardView === "issues" || boardView === "hierarchy" || boardView === "archive") && <div className="toolbar-tools">
            {boardView === "issues" && (
              <>
              <div className="bulk-task-actions" aria-label="批量任务操作">
                <button
                  className="bulk-task-action"
                  type="button"
                  disabled={bulkTaskActionDisabled}
                  aria-label={selectedActiveTasks.length > 0 ? `归档 ${selectedActiveTasks.length} 个任务` : "批量归档任务"}
                  title={selectedActiveTasks.length > 0 ? `归档 ${selectedActiveTasks.length} 个任务` : "选择任务后批量归档"}
                  onClick={() => void archiveSelectedTasks()}
                >
                  <LinearIcon name="folder" />
                  <span>{bulkArchiveLabel}</span>
                </button>
                <button
                  className="bulk-task-action danger"
                  type="button"
                  disabled={bulkTaskActionDisabled}
                  aria-label={selectedActiveTasks.length > 0 ? `删除 ${selectedActiveTasks.length} 个任务` : "批量删除任务"}
                  title={selectedActiveTasks.length > 0 ? `删除 ${selectedActiveTasks.length} 个任务` : "选择任务后批量删除"}
                  onClick={requestBulkTaskDelete}
                >
                  <LinearIcon name="trash" />
                  <span>{bulkDeleteLabel}</span>
                </button>
              </div>
              <button
                ref={bulkProjectMoveTriggerRef}
                className={`bulk-project-move${bulkProjectMoveMenuOpen ? " is-open" : ""}${selectedActiveTasks.length > 0 ? " has-selection" : ""}`}
                type="button"
                disabled={bulkProjectMoveDisabled}
                aria-haspopup="menu"
                aria-expanded={bulkProjectMoveMenuOpen}
                aria-label={selectedActiveTasks.length > 0 ? `移动 ${selectedActiveTasks.length} 个任务到项目` : "移动到项目"}
                title={
                  projectMoveChoices.length === 0
                    ? "没有可移动的目标项目"
                    : selectedActiveTasks.length > 0
                      ? `移动 ${selectedActiveTasks.length} 个任务到项目`
                      : "选择任务后移动到项目"
                }
                onClick={() => {
                  setBulkProjectMoveMenuPosition((current) => ({ ...current, ready: false }));
                  setBulkProjectMoveMenuOpen((current) => !current);
                }}
              >
                <LinearIcon className="bulk-project-move-icon" name="project" />
                <span>{bulkProjectMoveLabel}</span>
                <LinearIcon className="bulk-project-move-chevron" name="chevronDown" />
              </button>
              {bulkProjectMoveMenu}
              </>
            )}
            <label className={`search-field${search ? " has-value" : ""}`} title="搜索任务 (/)" >
              <LinearIcon className="search-icon" name="search" />
              <span className="sr-only">搜索任务</span>
              <input
                id="task-search"
                type="search"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="搜索任务…"
              />
              {!search && <kbd>/</kbd>}
            </label>
            <TaskFilterMenu
              tasks={tasks}
              search={search}
              labels={availableLabels}
              filters={filters}
              onChange={setFilters}
            />
            {boardView === "issues" && (
              <BoardSettingsMenu
                showEmptyColumns={showEmptyColumns}
                onShowEmptyColumnsChange={updateShowEmptyColumns}
              />
            )}
            {(search || activeFilterCount > 0) && (
              <button
                className="clear-filter"
                type="button"
                aria-label="清除筛选"
                title="清除筛选"
                onClick={() => { setSearch(""); setFilters(EMPTY_TASK_FILTERS); }}
              >
                <LinearIcon name="close" />
              </button>
            )}
          </div>}
        </div>}

        {(loadError || actionError) && (
          <div className="error-banner" role="alert">
            <span className="error-mark" aria-hidden="true"><LinearIcon name="alert" /></span>
            <div><strong>Taskboard needs attention</strong><p>{actionError ?? loadError}</p></div>
            <button
              type="button"
              onClick={() => {
                setActionError(null);
                if (selectedProjectId) void refreshTasks(selectedProjectId);
                else void loadProjectList();
              }}
            >
              Try again
            </button>
          </div>
        )}

        {!selectedProjectId ? (
          <section className="project-home">
            <div className="project-home-heading">
              <span>任务面板</span>
              <h1>选择项目</h1>
              <p>从 Codex 项目开始，或继续使用之前保存的项目。</p>
            </div>
            {projectsLoading ? (
              <div className="project-grid project-grid-loading" aria-label="正在加载项目" aria-busy="true">
                <span /><span /><span />
              </div>
            ) : projectChoices.length > 0 ? (
              <>
                <div
                  className={`project-home-bulk-actions${selectedManagedProjectChoices.length > 0 ? " is-visible" : ""}`}
                  aria-hidden={selectedManagedProjectChoices.length === 0}
                >
                    <span>已选 {selectedManagedProjectChoices.length} 个项目</span>
                    <button
                      className="project-bulk-action"
                      type="button"
                      disabled={projectActionBusy || selectedProjectActionIds.length === 0}
                      onClick={() => void mutateProjects(selectedProjectActionIds, "archive")}
                    >
                      <LinearIcon name="archive" />
                      <span>{projectArchiveLabel}</span>
                    </button>
                    <button
                      className="project-bulk-action danger"
                      type="button"
                      disabled={projectActionBusy || selectedProjectActionIds.length === 0}
                      onClick={() => void mutateProjects(selectedProjectActionIds, "delete")}
                    >
                      <LinearIcon name="trash" />
                      <span>{projectDeleteLabel}</span>
                    </button>
                </div>
                <div
                  className={`project-home-groups${projectSelectionBox ? " is-selecting" : ""}`}
                  onPointerDown={startProjectBoxSelection}
                  onPointerMove={updateProjectBoxSelection}
                  onPointerUp={finishProjectBoxSelection}
                  onPointerCancel={finishProjectBoxSelection}
                  onLostPointerCapture={finishProjectBoxSelection}
                >
                  {[
                    { id: "with-issues", title: "已有任务", projects: projectsWithIssues },
                    { id: "without-issues", title: "尚未添加任务", projects: projectsWithoutIssues },
                  ].map((group) => (
                    <section className="project-home-group" key={group.id} aria-labelledby={`project-group-${group.id}`}>
                      <div className="project-group-heading">
                        <h2 id={`project-group-${group.id}`}>{group.title}</h2>
                        <span>{group.projects.length}</span>
                      </div>
                      {group.projects.length > 0 ? (
                        <div className="project-grid">
                          {group.projects.map((project) => {
                            const canManageProject = canManageProjectChoice(project);
                            const isSelected = selectedProjectIds.has(project.id);
                            const workspacePath = project.workspacePath ?? deviceWorkspacePaths[project.id] ?? "";
                            return (
                              <div
                                className={`project-card${isSelected ? " is-selected" : ""}`}
                                key={project.id}
                                data-project-id={project.id}
                                data-project-selectable={canManageProject ? "true" : undefined}
                              >
                                <div className="project-card-main">
                                  <button
                                    className="project-card-open"
                                    type="button"
                                    disabled={openingProjectId !== null || projectActionBusy}
                                    onClick={() => void selectProject(project)}
                                  >
                                    <span className="project-card-avatar" aria-hidden="true">
                                      {project.name.slice(0, 1).toUpperCase()}
                                    </span>
                                    <span className="project-card-copy">
                                      <strong>{project.name}</strong>
                                      <span>
                                        {project.inCodex ? "Codex 项目" : "已保存的项目"}
                                        {project.issueCount > 0 ? ` · ${project.issueCount} 个任务` : ""}
                                      </span>
                                    </span>
                                    {favoriteProjectIds.has(project.id) && <span className="project-card-favorite" aria-label="已收藏"><LinearIcon name="favorite" /></span>}
                                    <span className="project-card-action" aria-hidden="true">
                                      {openingProjectId === project.id ? "正在打开…" : <LinearIcon name="chevronRight" />}
                                    </span>
                                  </button>
                                  <div className="project-card-controls">
                                    <button
                                      className="icon-button project-card-create"
                                      type="button"
                                      disabled={openingProjectId !== null || projectActionBusy}
                                      onClick={() => void selectProject(project, { createIssue: true })}
                                      aria-label={`在 ${project.name} 中新建任务`}
                                      title={`在 ${project.name} 中新建任务`}
                                    >
                                      <LinearIcon name="plus" />
                                    </button>
                                    {canManageProject && (
                                      <>
                                        <button
                                          className="icon-button project-card-archive"
                                          type="button"
                                          disabled={openingProjectId !== null || projectActionBusy}
                                          onClick={() => void mutateProjects([project.id], "archive")}
                                          aria-label={`归档 ${project.name}`}
                                          title={`归档 ${project.name}`}
                                        >
                                          <LinearIcon name="archive" />
                                        </button>
                                        <button
                                          className="icon-button project-card-delete"
                                          type="button"
                                          disabled={openingProjectId !== null || projectActionBusy}
                                          onClick={() => void mutateProjects([project.id], "delete")}
                                          aria-label={`删除 ${project.name}`}
                                          title={`删除 ${project.name}`}
                                        >
                                          <LinearIcon name="trash" />
                                        </button>
                                      </>
                                    )}
                                  </div>
                                </div>
                                <label className="project-card-directory">
                                  <LinearIcon name="folder" />
                                  <input
                                    key={workspacePath}
                                    type="text"
                                    defaultValue={workspacePath}
                                    placeholder="设置此设备的项目目录"
                                    aria-label={`${project.name} 在此设备上的项目目录`}
                                    onBlur={(event) => rememberDeviceWorkspacePath(project.id, event.currentTarget.value)}
                                    onKeyDown={(event) => {
                                      if (event.key === "Enter") event.currentTarget.blur();
                                    }}
                                  />
                                </label>
                              </div>
                            );
                          })}
                        </div>
                      ) : (
                        <p className="project-group-empty">暂无项目</p>
                      )}
                    </section>
                  ))}
                  {projectSelectionBox && (
                    <div
                      className="project-selection-box"
                      style={projectSelectionBox}
                      aria-hidden="true"
                    />
                  )}
                </div>
              </>
            ) : (
              <div className="project-home-empty">
                <span className="empty-orbit" aria-hidden="true"><i /><i /></span>
                <h2>还没有项目</h2>
                <p>在 Codex 中创建项目后，再打开任务面板。</p>
              </div>
            )}
          </section>
        ) : detailTask && selectedProject ? (
          <TaskDetail
            key={detailTask.id}
            task={detailTask}
            tasks={tasks}
            currentUser={currentUser}
            availableLabels={availableLabels}
            workflows={workflowOptions}
            developmentScan={developmentScan}
            developmentScanLoading={developmentScanLoading}
            commentsRevision={commentsRevision}
            attachmentsRevision={attachmentsRevision}
            onUpdate={(current, changes) => updateTaskProperties(current, changes)}
            onArchive={archiveTask}
            onRestore={restoreArchivedTask}
            onDelete={deleteTask}
            onOpenTask={openTaskDetail}
            onAddRelation={(current, type, relatedTaskId) => (
              mutateTaskRelation("add", current, type, relatedTaskId)
            )}
            onRemoveRelation={(current, type, relatedTaskId) => (
              mutateTaskRelation("remove", current, type, relatedTaskId)
            )}
            onContinueThread={continueThreadInPanel}
            onOpenThread={openThread}
            onOpenInThread={openTaskInThread}
            openingThread={openingThreadTaskId === detailTask.id}
            onError={setActionError}
            onAnnounce={setAnnouncement}
          />
        ) : boardView === "archive" ? (
          tasksLoading && !hasLoadedTasks ? (
            <div className="loading-board archive-loading" aria-label="正在加载归档任务" aria-busy="true">
              {TASK_STATUSES.slice(0, 3).map((status) => (
                <div className="loading-column" key={status}>
                  <span /><div /><div />
                </div>
              ))}
            </div>
          ) : (
            <ArchivedIssues
              tasks={filteredTasks}
              totalTasks={tasks.length}
              hasQuery={Boolean(search || activeFilterCount > 0)}
              restoringTaskId={restoringArchivedTaskId}
              deletingTaskId={deletingArchivedTask ? pendingArchivedDeleteTask?.id ?? null : null}
              onClearQuery={() => { setSearch(""); setFilters(EMPTY_TASK_FILTERS); }}
              onOpenTask={openTaskDetail}
              onRestore={(task) => void restoreArchivedTask(task)}
              onRequestDelete={setPendingArchivedDeleteTask}
            />
          )
        ) : boardView === "hierarchy" ? (
          tasksLoading && !hasLoadedTasks ? (
            <div className="loading-board archive-loading" aria-label="正在加载层级视图" aria-busy="true">
              {TASK_STATUSES.slice(0, 3).map((status) => (
                <div className="loading-column" key={status}>
                  <span /><div /><div />
                </div>
              ))}
            </div>
          ) : (
            <IssueHierarchyView
              tasks={filteredTasks}
              allTasks={tasks}
              totalTasks={tasks.length}
              hasQuery={Boolean(search || activeFilterCount > 0)}
              onClearQuery={() => { setSearch(""); setFilters(EMPTY_TASK_FILTERS); }}
              onOpenTask={openTaskDetail}
              onOpenThread={openThread}
            />
          )
        ) : boardView === "workflow" ? (
          <Suspense fallback={<div className="workflow-board-loading">正在打开节点模式…</div>}>
            <WorkflowBoard
              key={selectedProject?.id ?? DEFAULT_PROJECT_ID}
              projectId={selectedProject?.id ?? DEFAULT_PROJECT_ID}
              projectName={selectedProjectDisplayName || "当前项目"}
              workspacePath={
                selectedDeviceWorkspacePath
                ?? developmentScan.workspacePath
                ?? hostContext?.workspacePath
              }
              revision={workflowRevision}
              onWorkflowsChange={setWorkflowOptions}
            />
          </Suspense>
        ) : tasksLoading && !hasLoadedTasks ? (
          <div className="loading-board" aria-label="Loading tasks" aria-busy="true">
            {TASK_STATUSES.map((status) => (
              <div className="loading-column" key={status}>
                <span /><div /><div />
              </div>
            ))}
          </div>
        ) : (
          <div
            className={`board-scroll${selectionBox ? " is-selecting" : ""}`}
            aria-label="Task board"
            onPointerDown={startBoxSelection}
            onPointerMove={updateBoxSelection}
            onPointerUp={finishBoxSelection}
            onPointerCancel={finishBoxSelection}
            onLostPointerCapture={finishBoxSelection}
          >
            <div className="board">
              {filteredTasks.length === 0 && tasks.length > 0 && !showEmptyColumns && (
                <section className="page-empty filter-empty board-filter-empty">
                  <span className="empty-search" aria-hidden="true"><LinearIcon name="search" /></span>
                  <h2>没有匹配的任务</h2>
                  <p>请更换搜索词，或移除一个筛选条件。</p>
                  <button
                    className="button secondary"
                    type="button"
                    onClick={() => { setSearch(""); setFilters(EMPTY_TASK_FILTERS); }}
                  >
                    清除筛选
                  </button>
                </section>
              )}
              {visibleStatuses.map((status) => (
                <BoardColumn
                  key={status}
                  status={status}
                  statusIndex={TASK_STATUSES.indexOf(status)}
                  tasks={tasksByStatus[status]}
                  allTasks={tasks}
                  isDropTarget={dropTarget === status}
                  draggedTaskId={draggedTaskId}
                  draggedTaskIds={draggedTaskIds}
                  selectedTaskIds={selectedTaskIdArray}
                  draggedTaskHeight={draggedTaskHeight}
                  movingTaskId={movingTaskId}
                  settlingTaskId={settlingTaskId}
                  contextMenuTaskId={contextMenu?.taskId ?? null}
                  onCreate={(initialStatus) => setEditor({ task: null, status: initialStatus })}
                  onEdit={openTaskDetail}
                  onSelectionChange={setTaskSelection}
                  onContextMenu={(task, position) => setContextMenu({ taskId: task.id, ...position })}
                  onMove={(task, destination) => void moveTask(task, destination)}
                  onDragStart={(task, height) => {
                    const draggingIds = selectedTaskIds.has(task.id) ? selectedTaskIdArray : [task.id];
                    setDraggedTaskId(task.id);
                    setDraggedTaskIds(draggingIds);
                    setDraggedTaskHeight(height);
                    setDropTarget(task.status);
                  }}
                  onDragEnd={() => {
                    setDraggedTaskId(null);
                    setDraggedTaskIds([]);
                    setDraggedTaskHeight(0);
                    setDropTarget(null);
                  }}
                  onDragEnter={setDropTarget}
                  onDrop={finishTaskDrop}
                  onOpenThread={openThread}
                  onHide={(hiddenStatus) => updateColumnVisibility(hiddenStatus, false)}
                />
              ))}
              {hiddenStatuses.length > 0 && (
                <HiddenColumns
                  statuses={hiddenStatuses}
                  counts={Object.fromEntries(
                    TASK_STATUSES.map((status) => [status, tasksByStatus[status].length]),
                  ) as Record<TaskStatus, number>}
                  dropTarget={dropTarget}
                  onDragTargetChange={setDropTarget}
                  onDrop={(destination, taskId) => finishTaskDrop(destination, taskId)}
                  onShow={(shownStatus) => updateColumnVisibility(shownStatus, true)}
                />
              )}
            </div>
            {selectionBox && (
              <div
                className="board-selection-box"
                style={selectionBox}
                aria-hidden="true"
              />
            )}
          </div>
        )}
      </main>

      {editor && (
        <TaskEditor
          key={editor.task?.id ?? `new-${editor.projectId ?? selectedProjectId}-${editor.status}`}
          task={editor.task}
          initialStatus={editor.status}
          labels={availableLabels}
          workflows={workflowOptions}
          currentUser={currentUser}
          developmentScan={developmentScan}
          developmentScanLoading={developmentScanLoading}
          onCancel={() => setEditor(null)}
          onSave={saveEditor}
        />
      )}

      {contextMenu && contextMenuTask && (
        <TaskContextMenu
          task={contextMenuTask}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          labels={availableLabels}
          onClose={closeContextMenu}
          onEdit={openTaskDetail}
          onStatusChange={(task, status) => void moveTask(task, status)}
          onPriorityChange={(task, nextPriority) => void updateTaskProperties(
            task,
            { priority: nextPriority },
            `${task.identifier} 优先级已更新。`,
          ).catch(() => {})}
          onLabelsChange={(task, labels) => void updateTaskProperties(
            task,
            { labels },
            `${task.identifier} 标签已更新。`,
          ).catch(() => {})}
          onDuplicate={(task) => void duplicateTask(task)}
          onCopy={(text, message) => void copyText(text, message)}
          onOpenInThread={openTaskInThread}
          onArchive={(task) => void archiveTask(task)}
        />
      )}

      {pendingArchivedDeleteTask && (
        <div className="delete-backdrop" role="presentation" onMouseDown={(event) => {
          if (event.target === event.currentTarget && !deletingArchivedTask) setPendingArchivedDeleteTask(null);
        }}>
          <div className="delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="delete-archived-task-title">
            <h2 id="delete-archived-task-title">永久删除这个归档任务？</h2>
            <p>“{pendingArchivedDeleteTask.identifier} {pendingArchivedDeleteTask.title}” 会被永久删除，评论、附件和关联也会删除，无法恢复。</p>
            <div>
              <button className="button secondary" type="button" disabled={deletingArchivedTask} onClick={() => setPendingArchivedDeleteTask(null)}>取消</button>
              <button className="button danger" type="button" disabled={deletingArchivedTask} onClick={() => void confirmArchivedTaskDelete()}>{deletingArchivedTask ? "删除中..." : "永久删除"}</button>
            </div>
          </div>
        </div>
      )}

      {pendingBulkDeleteTasks.length > 0 && (
        <div className="delete-backdrop" role="presentation" onMouseDown={(event) => {
          if (event.target === event.currentTarget && !bulkDeletingSelectedTasks) setPendingBulkDeleteTasks([]);
        }}>
          <div className="delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="bulk-delete-task-title">
            <h2 id="bulk-delete-task-title">永久删除 {pendingBulkDeleteTasks.length} 个任务？</h2>
            <p>这些任务会被永久删除，评论、附件和关联也会删除，无法恢复。</p>
            <div>
              <button className="button secondary" type="button" disabled={bulkDeletingSelectedTasks} onClick={() => setPendingBulkDeleteTasks([])}>取消</button>
              <button className="button danger" type="button" disabled={bulkDeletingSelectedTasks} onClick={() => void confirmBulkTaskDelete()}>{bulkDeletingSelectedTasks ? "删除中..." : "永久删除"}</button>
            </div>
          </div>
        </div>
      )}

      {!embedded && (
        <AiChat
          available={localAiChatAvailable}
          projectId={selectedProjectId || null}
          issueId={detailTaskId}
        />
      )}

      <div className="sr-only" role="status" aria-live="polite">{announcement}</div>
      {undoNotice && (
        <div
          className="toast undo-toast"
          role="status"
          onAnimationEnd={() => setUndoNotice((current) => current?.id === undoNotice.id ? null : current)}
        >
          <span className="toast-check" aria-hidden="true"><LinearIcon name="check" /></span>
          <span className="undo-toast-message">{undoNotice.message}</span>
          <button type="button" onClick={() => void performUndo()}>
            撤回 <kbd>{undoShortcut}</kbd>
          </button>
        </div>
      )}
      {announcement && (
        <div className="toast" role="status" onAnimationEnd={() => setAnnouncementValue("")}>
          <span aria-hidden="true"><LinearIcon name="check" /></span>{announcement}
        </div>
      )}
      {draggedTaskId && (
        <div className="drag-hint" aria-hidden="true">
          {draggedTaskIds.length > 1 ? `拖动 ${draggedTaskIds.length} 个任务` : "拖到目标位置后松开"}
        </div>
      )}
    </div>
  );
}
