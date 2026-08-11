import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  applyNodeChanges,
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  type Edge,
  type EdgeTypes,
  type NodeChange,
  type NodeTypes,
  type OnNodeDrag,
  type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import "./workflow.css";
import {
  createWorkflowFlow,
  deleteWorkflowNode,
  deriveWorkflowLayout,
  findWorkflowItem,
  insertWorkflowNode,
  normalizeWorkflowSnapshot,
  serializeWorkflowSnapshot,
  workflowNodeIds,
} from "../../../shared/workflow-control-flow.mjs";
import type {
  WorkflowFlow,
  WorkflowSequenceRef,
} from "../../../shared/workflow-control-flow.mjs";
import {
  ApiError,
  getWorkflowWorkspace,
  listWorkflowCapabilities,
  saveWorkflowWorkspace,
} from "../api";
import type { WorkflowCapabilities, WorkflowOption } from "../types";
import {
  clearLegacyWorkflowWorkspace,
  INITIAL_WORKFLOW_ID,
  INITIAL_WORKFLOW_NAME,
  readLegacyWorkflowWorkspace,
} from "../workflowStore";
import { LinearIcon } from "./LinearIcon";
import { WorkflowInsertEdge } from "./WorkflowInsertEdge";
import { WorkflowInspector } from "./WorkflowInspector";
import {
  WorkflowNode,
  type WorkflowCanvasNode,
  type WorkflowNodeData,
} from "./WorkflowNode";
import { WorkflowStepPicker } from "./WorkflowStepPicker";
import {
  PALETTE_ITEMS,
  capabilityNodeMeta,
  isWorkflowTriggerKind,
  paletteData,
  type PaletteItem,
  workflowNodeConfigured,
  workflowNodeDisplayTitle,
} from "./workflowCatalog";

interface WorkflowBoardProps {
  projectId: string;
  projectName: string;
  workspacePath?: string;
  revision: number;
  onWorkflowsChange: (workflows: WorkflowOption[]) => void;
}

interface WorkflowTab {
  id: string;
  name: string;
}

interface WorkflowSnapshot {
  nodes: WorkflowCanvasNode[];
  flow: WorkflowFlow;
  selectedNodeId: string | null;
}

interface LegacyWorkflowSnapshot {
  nodes: WorkflowCanvasNode[];
  edges?: Edge[];
  flow?: WorkflowFlow;
  selectedNodeId: string | null;
}

interface WorkflowWorkspace {
  version: 1;
  tabs: WorkflowTab[];
  activeWorkflowId: string;
  snapshots: Record<string, LegacyWorkflowSnapshot>;
}

type StepPickerTarget =
  | {
      kind: "sequence";
      sequenceRef: WorkflowSequenceRef;
      index: number;
    }
  | {
      kind: "plan";
      parentId: string;
    };

interface WorkflowTabMenu {
  workflowId: string;
  x: number;
  y: number;
}

interface PlanDragPreview {
  nodeId: string;
  parentId: string;
  sourceOrderIds: string[];
  sourceIndex: number;
  targetIndex: number;
}

const WORKFLOW_STEP_WIDTH = 250;
const WORKFLOW_STEP_HEIGHT = 138;
const PLAN_ITEM_WIDTH = 230;
const PLAN_ITEM_HEIGHT = 34;
const PLAN_ITEM_GAP = 4;
const PLAN_LIST_TOP = 86;
const PLAN_CONTAINER_BOTTOM = 38;
const END_STEP_HEIGHT = 1;
const TOP_CENTER_ORIGIN: [number, number] = [0.5, 0];
const TOP_LEFT_ORIGIN: [number, number] = [0, 0];
const CANVAS_PAN_PADDING = 72;
const PRO_OPTIONS = { hideAttribution: true };
const NODE_TYPES = { workflow: WorkflowNode } satisfies NodeTypes;
const EDGE_TYPES = { workflowInsert: WorkflowInsertEdge } satisfies EdgeTypes;
const NESTABLE_TONES = new Set(["capability", "api", "integration", "development"]);

function isVirtualWorkflowNodeId(nodeId: string) {
  return nodeId.startsWith("__flow-");
}

function workflowContentBounds(nodes: WorkflowCanvasNode[]) {
  const rootNodes = nodes.filter((node) => !node.parentId && !isVirtualWorkflowNodeId(node.id));
  if (rootNodes.length === 0) return null;
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  for (const node of rootNodes) {
    const width = Number(node.style?.width ?? node.measured?.width ?? node.initialWidth ?? WORKFLOW_STEP_WIDTH);
    const height = Number(node.style?.height ?? node.measured?.height ?? node.initialHeight ?? WORKFLOW_STEP_HEIGHT);
    const origin = node.origin ?? TOP_CENTER_ORIGIN;
    const x = node.position.x - width * origin[0];
    const y = node.position.y - height * origin[1];
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x + width);
    maxY = Math.max(maxY, y + height);
  }
  return { minX, minY, maxX, maxY };
}

function planItemPosition(index: number) {
  return {
    x: (WORKFLOW_STEP_WIDTH - PLAN_ITEM_WIDTH) / 2,
    y: PLAN_LIST_TOP + index * (PLAN_ITEM_HEIGHT + PLAN_ITEM_GAP),
  };
}

function planContainerHeight(childCount: number) {
  const listHeight = childCount === 0
    ? 0
    : childCount * PLAN_ITEM_HEIGHT + (childCount - 1) * PLAN_ITEM_GAP;
  return Math.max(
    WORKFLOW_STEP_HEIGHT,
    PLAN_LIST_TOP + listHeight + PLAN_CONTAINER_BOTTOM,
  );
}

function layoutPlanChildren(
  nodes: WorkflowCanvasNode[],
  parentId: string,
  orderedChildIds: string[],
): WorkflowCanvasNode[] {
  const positions = new Map(orderedChildIds.map((id, index) => [id, planItemPosition(index)]));
  return nodes.map((node) => (
    node.parentId === parentId && positions.has(node.id)
      ? {
          ...node,
          origin: TOP_LEFT_ORIGIN,
          position: positions.get(node.id)!,
          style: { width: PLAN_ITEM_WIDTH, height: PLAN_ITEM_HEIGHT },
          initialWidth: PLAN_ITEM_WIDTH,
          initialHeight: PLAN_ITEM_HEIGHT,
          measured: { width: PLAN_ITEM_WIDTH, height: PLAN_ITEM_HEIGHT },
          zIndex: 2,
        }
      : node
  ));
}

function prepareWorkflowNodes(nodes: WorkflowCanvasNode[]): WorkflowCanvasNode[] {
  let next = nodes;
  for (const node of nodes) {
    if (node.parentId) continue;
    if (node.data.acceptsChildren) {
      const childIds = next
        .filter((candidate) => candidate.parentId === node.id)
        .sort((left, right) => left.position.y - right.position.y)
        .map((candidate) => candidate.id);
      next = layoutPlanChildren(next, node.id, childIds);
      const height = planContainerHeight(childIds.length);
      next = next.map((candidate) => (
        candidate.id === node.id
          ? {
              ...candidate,
              style: { width: WORKFLOW_STEP_WIDTH, height },
              initialWidth: WORKFLOW_STEP_WIDTH,
              initialHeight: height,
              measured: { width: WORKFLOW_STEP_WIDTH, height },
            }
          : candidate
      ));
    }
  }
  return next;
}

function layoutWorkflowFlow(
  nodes: WorkflowCanvasNode[],
  flow: WorkflowFlow,
): WorkflowCanvasNode[] {
  const prepared = prepareWorkflowNodes(nodes);
  const layout = deriveWorkflowLayout(flow, prepared);
  return prepared.map((node) => {
    if (node.parentId || !layout.positions[node.id]) return node;
    const height = Number(node.style?.height ?? WORKFLOW_STEP_HEIGHT);
    return {
      ...node,
      origin: TOP_CENTER_ORIGIN,
      position: layout.positions[node.id],
      style: { width: WORKFLOW_STEP_WIDTH, height },
      initialWidth: WORKFLOW_STEP_WIDTH,
      initialHeight: height,
      measured: { width: WORKFLOW_STEP_WIDTH, height },
    };
  });
}

function normalizeSnapshot(snapshot: LegacyWorkflowSnapshot): WorkflowSnapshot {
  const normalized = normalizeWorkflowSnapshot(snapshot);
  return {
    nodes: layoutWorkflowFlow(normalized.nodes, normalized.flow),
    flow: normalized.flow,
    selectedNodeId: null,
  };
}

function initialNodes(): WorkflowCanvasNode[] {
  const nodes: WorkflowCanvasNode[] = [
    {
      id: "issue-trigger",
      type: "workflow",
      position: { x: 0, y: 48 },
      data: {
        ...paletteData("issue-trigger"),
        title: "任务触发器",
        description: "状态变为「待办事项」时触发",
        meta: "任意优先级 · 任意标签",
      },
    },
    {
      id: "basic-planning",
      type: "workflow",
      position: { x: 0, y: 184 },
      data: {
        ...paletteData("basic-planning"),
        title: "拆解任务执行计划",
        description: "生成步骤、依赖和验收条件",
        meta: "基础规划 · 当前项目",
      },
    },
    {
      id: "skill",
      type: "workflow",
      parentId: "basic-planning",
      position: planItemPosition(0),
      data: {
        ...paletteData("skill"),
        title: "调用 Skill",
        description: "运行一个已安装的 Skill",
        meta: "尚未选择 Skill",
      },
    },
    {
      id: "mcp",
      type: "workflow",
      parentId: "basic-planning",
      position: planItemPosition(1),
      data: {
        ...paletteData("mcp"),
        title: "调用 MCP",
        description: "连接一个已配置的 MCP Server",
        meta: "尚未选择 MCP Server",
      },
    },
    {
      id: "nano-banana",
      type: "workflow",
      parentId: "basic-planning",
      position: planItemPosition(2),
      data: {
        ...paletteData("nano-banana"),
        title: "生成预览素材",
        description: "根据任务内容生成预览图",
        meta: "Nano Banana · 16:9",
      },
    },
    {
      id: "cloudflare-deploy",
      type: "workflow",
      parentId: "basic-planning",
      position: planItemPosition(3),
      data: {
        ...paletteData("cloudflare-deploy"),
        title: "部署预览版本",
        description: "构建并发布项目预览",
        meta: "Cloudflare Pages · Preview",
      },
    },
    {
      id: "codex-review",
      type: "workflow",
      position: { x: 0, y: 520 },
      data: {
        ...paletteData("codex-review"),
        title: "审核交付结果",
        description: "检查产物、测试与验收条件",
        meta: "Codex · 自动审核",
      },
    },
    {
      id: "issue-update",
      type: "workflow",
      position: { x: 0, y: 656 },
      data: {
        ...paletteData("issue-update"),
        title: "提交审核",
        description: "追加结果评论并更新状态",
        meta: "状态 → 审核中",
      },
    },
  ];
  return nodes;
}

function createInitialWorkflowWorkspace() {
  const stepIds = ["issue-trigger", "basic-planning", "codex-review", "issue-update"];
  const flow = createWorkflowFlow(stepIds);
  return {
    tabs: [{ id: INITIAL_WORKFLOW_ID, name: INITIAL_WORKFLOW_NAME }],
    activeWorkflowId: INITIAL_WORKFLOW_ID,
    snapshots: new Map<string, WorkflowSnapshot>([
      [
        INITIAL_WORKFLOW_ID,
        {
          nodes: layoutWorkflowFlow(initialNodes(), flow),
          flow,
          selectedNodeId: null,
        },
      ],
    ]),
  };
}

function parseWorkflowWorkspace(value: unknown) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const stored = value as Partial<WorkflowWorkspace>;
  if (stored.version !== 1 || !Array.isArray(stored.tabs) || !stored.snapshots) return null;
  const tabs = stored.tabs.filter((tab) => (
    typeof tab?.id === "string"
    && typeof tab.name === "string"
    && tab.id.length > 0
    && tab.name.length > 0
  ));
  if (tabs.length === 0) return null;

  const snapshots = new Map<string, WorkflowSnapshot>();
  for (const tab of tabs) {
    const snapshot = stored.snapshots[tab.id];
    if (
      !snapshot
      || !Array.isArray(snapshot.nodes)
      || (!Array.isArray(snapshot.edges) && snapshot.flow?.version !== 2)
      || (snapshot.selectedNodeId != null && typeof snapshot.selectedNodeId !== "string")
    ) {
      return null;
    }
    snapshots.set(tab.id, normalizeSnapshot(snapshot));
  }
  const activeWorkflowId = tabs.some((tab) => tab.id === stored.activeWorkflowId)
    ? stored.activeWorkflowId!
    : tabs[0].id;
  return { tabs, activeWorkflowId, snapshots };
}

function serializeWorkflowWorkspace(
  tabs: WorkflowTab[],
  activeWorkflowId: string,
  snapshots: Map<string, WorkflowSnapshot>,
): WorkflowWorkspace {
  return {
    version: 1,
    tabs,
    activeWorkflowId,
    snapshots: Object.fromEntries(tabs.map((tab) => {
      const snapshot = snapshots.get(tab.id)!;
      return [
        tab.id,
        {
          ...serializeWorkflowSnapshot(snapshot.nodes, snapshot.flow, null),
        },
      ];
    })),
  };
}

function workflowSignature(tab: WorkflowTab, snapshot: WorkflowSnapshot): string {
  return JSON.stringify({
    name: tab.name,
    nodes: snapshot.nodes,
    flow: snapshot.flow,
  });
}

function mergeLegacyWorkspace(
  remote: ReturnType<typeof createInitialWorkflowWorkspace>,
  legacy: ReturnType<typeof createInitialWorkflowWorkspace>,
) {
  const tabs = [...remote.tabs];
  const snapshots = new Map(remote.snapshots);
  for (const legacyTab of legacy.tabs) {
    const legacySnapshot = legacy.snapshots.get(legacyTab.id)!;
    const remoteTab = tabs.find((tab) => tab.id === legacyTab.id);
    if (!remoteTab) {
      tabs.push(legacyTab);
      snapshots.set(legacyTab.id, normalizeSnapshot(legacySnapshot));
      continue;
    }
    const remoteSnapshot = snapshots.get(remoteTab.id)!;
    if (workflowSignature(remoteTab, remoteSnapshot) === workflowSignature(legacyTab, legacySnapshot)) {
      continue;
    }
    const importedId = `workflow-imported-${crypto.randomUUID()}`;
    tabs.push({ id: importedId, name: `${legacyTab.name}（从另一入口导入）` });
    snapshots.set(importedId, normalizeSnapshot(legacySnapshot));
  }
  return {
    tabs,
    activeWorkflowId: remote.activeWorkflowId,
    snapshots,
  };
}

function planDragShift(
  nodeId: string,
  preview: PlanDragPreview | null,
): number {
  if (!preview || nodeId === preview.nodeId) return 0;
  const nodeIndex = preview.sourceOrderIds.indexOf(nodeId);
  const distance = PLAN_ITEM_HEIGHT + PLAN_ITEM_GAP;
  if (preview.targetIndex > preview.sourceIndex) {
    return nodeIndex > preview.sourceIndex && nodeIndex <= preview.targetIndex ? -distance : 0;
  }
  if (preview.targetIndex < preview.sourceIndex) {
    return nodeIndex >= preview.targetIndex && nodeIndex < preview.sourceIndex ? distance : 0;
  }
  return 0;
}

export function WorkflowBoard({
  projectId,
  projectName,
  workspacePath,
  revision,
  onWorkflowsChange,
}: WorkflowBoardProps) {
  const [initialWorkspace] = useState(
    () => parseWorkflowWorkspace(readLegacyWorkflowWorkspace(projectId)) ?? createInitialWorkflowWorkspace(),
  );
  const initialSnapshot = initialWorkspace.snapshots.get(initialWorkspace.activeWorkflowId)!;
  const [nodes, setNodes] = useState<WorkflowCanvasNode[]>(initialSnapshot.nodes);
  const [flow, setFlow] = useState<WorkflowFlow>(initialSnapshot.flow);
  const [workflowTabs, setWorkflowTabs] = useState<WorkflowTab[]>(initialWorkspace.tabs);
  const [activeWorkflowId, setActiveWorkflowId] = useState(initialWorkspace.activeWorkflowId);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [renamingWorkflowId, setRenamingWorkflowId] = useState<string | null>(null);
  const [workflowNameDraft, setWorkflowNameDraft] = useState("");
  const [workflowTabMenu, setWorkflowTabMenu] = useState<WorkflowTabMenu | null>(null);
  const [pickerTarget, setPickerTarget] = useState<StepPickerTarget | null>(null);
  const [planDragPreview, setPlanDragPreview] = useState<PlanDragPreview | null>(null);
  const [settlingNodeId, setSettlingNodeId] = useState<string | null>(null);
  const [workflowCapabilities, setWorkflowCapabilities] = useState<WorkflowCapabilities | null>(null);
  const [workflowCapabilitiesFailed, setWorkflowCapabilitiesFailed] = useState(false);
  const [hydrated, setHydrated] = useState(false);
  const [persistenceError, setPersistenceError] = useState("");
  const [canvasSize, setCanvasSize] = useState({ width: 0, height: 0 });
  const [canvasZoom, setCanvasZoom] = useState(1);
  const [contentBounds, setContentBounds] = useState(
    () => workflowContentBounds(initialSnapshot.nodes),
  );
  const flowRef = useRef<ReactFlowInstance<WorkflowCanvasNode, Edge> | null>(null);
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const workflowNameInputRef = useRef<HTMLInputElement | null>(null);
  const workflowTabMenuRef = useRef<HTMLDivElement | null>(null);
  const cancelWorkflowRenameRef = useRef(false);
  const workflowSnapshotsRef = useRef(initialWorkspace.snapshots);
  const remoteVersionRef = useRef(0);
  const lastRemoteWorkspaceRef = useRef("");
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const mountedRef = useRef(true);
  const settleTimerRef = useRef<number | null>(null);
  const planDragSessionRef = useRef<PlanDragPreview | null>(null);

  const layout = useMemo(() => deriveWorkflowLayout(flow, nodes), [flow, nodes]);
  const rootStepIds = flow.root.items.map((item) => item.nodeId);
  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;
  const translateExtent = useMemo<[[number, number], [number, number]] | undefined>(() => {
    if (!contentBounds || canvasSize.width === 0 || canvasSize.height === 0) return undefined;
    const padding = CANVAS_PAN_PADDING / canvasZoom;
    const viewportWidth = canvasSize.width / canvasZoom;
    const viewportHeight = canvasSize.height / canvasZoom;
    const contentWidth = contentBounds.maxX - contentBounds.minX;
    const contentHeight = contentBounds.maxY - contentBounds.minY;
    const extentWidth = Math.max(contentWidth + padding * 2, viewportWidth + padding * 2);
    const extentHeight = Math.max(contentHeight + padding * 2, viewportHeight + padding * 2);
    const centerX = (contentBounds.minX + contentBounds.maxX) / 2;
    const centerY = (contentBounds.minY + contentBounds.maxY) / 2;
    return [
      [centerX - extentWidth / 2, centerY - extentHeight / 2],
      [centerX + extentWidth / 2, centerY + extentHeight / 2],
    ];
  }, [canvasSize.height, canvasSize.width, canvasZoom, contentBounds]);

  const applyWorkspace = useCallback((workspace: ReturnType<typeof createInitialWorkflowWorkspace>) => {
    const snapshot = normalizeSnapshot(workspace.snapshots.get(workspace.activeWorkflowId)!);
    workflowSnapshotsRef.current = new Map(workspace.snapshots);
    workflowSnapshotsRef.current.set(workspace.activeWorkflowId, snapshot);
    setWorkflowTabs(workspace.tabs);
    setActiveWorkflowId(workspace.activeWorkflowId);
    setNodes(snapshot.nodes);
    setFlow(snapshot.flow);
    setSelectedNodeId(null);
    setRenamingWorkflowId(null);
    setWorkflowTabMenu(null);
    onWorkflowsChange(workspace.tabs);
  }, [onWorkflowsChange]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (settleTimerRef.current !== null) window.clearTimeout(settleTimerRef.current);
    };
  }, []);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    function updateCanvasSize() {
      const rect = canvas!.getBoundingClientRect();
      setCanvasSize((current) => (
        current.width === rect.width && current.height === rect.height
          ? current
          : { width: rect.width, height: rect.height }
      ));
    }
    updateCanvasSize();
    const observer = new ResizeObserver(updateCanvasSize);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      const rootNodeIds = nodes
        .filter((node) => !node.parentId && !isVirtualWorkflowNodeId(node.id))
        .map((node) => node.id);
      if (rootNodeIds.length === 0) {
        setContentBounds(null);
        return;
      }
      const measured = flowRef.current?.getNodesBounds(rootNodeIds);
      const next = measured && measured.width > 0 && measured.height > 0
        ? {
            minX: measured.x,
            minY: measured.y,
            maxX: measured.x + measured.width,
            maxY: measured.y + measured.height,
          }
        : workflowContentBounds(nodes);
      setContentBounds((current) => (
        current?.minX === next?.minX
        && current?.minY === next?.minY
        && current?.maxX === next?.maxX
        && current?.maxY === next?.maxY
          ? current
          : next
      ));
    });
    return () => window.cancelAnimationFrame(frame);
  }, [activeWorkflowId, nodes]);

  useEffect(() => {
    if (!translateExtent) return;
    const frame = window.requestAnimationFrame(() => {
      const instance = flowRef.current;
      if (instance) void instance.zoomTo(instance.getZoom(), { duration: 0 });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [translateExtent]);

  useEffect(() => {
    let cancelled = false;
    const legacy = parseWorkflowWorkspace(readLegacyWorkflowWorkspace(projectId));

    async function hydrateWorkspace() {
      try {
        let record = await getWorkflowWorkspace<WorkflowWorkspace>(projectId);
        let workspace = parseWorkflowWorkspace(record.workspace);
        if (!workspace) {
          workspace = legacy ?? initialWorkspace;
          try {
            record = await saveWorkflowWorkspace(
              projectId,
              serializeWorkflowWorkspace(workspace.tabs, workspace.activeWorkflowId, workspace.snapshots),
              record.version,
            );
          } catch (error) {
            if (!(error instanceof ApiError) || error.code !== "VERSION_CONFLICT") throw error;
            record = await getWorkflowWorkspace<WorkflowWorkspace>(projectId);
            const latest = parseWorkflowWorkspace(record.workspace);
            if (!latest) throw error;
            workspace = legacy ? mergeLegacyWorkspace(latest, legacy) : latest;
          }
        } else if (legacy) {
          const merged = mergeLegacyWorkspace(workspace, legacy);
          const serializedMerged = serializeWorkflowWorkspace(
            merged.tabs,
            merged.activeWorkflowId,
            merged.snapshots,
          );
          if (JSON.stringify(serializedMerged) !== JSON.stringify(record.workspace)) {
            record = await saveWorkflowWorkspace(projectId, serializedMerged, record.version);
            workspace = merged;
          }
        }
        if (cancelled) return;
        const serialized = serializeWorkflowWorkspace(
          workspace.tabs,
          workspace.activeWorkflowId,
          workspace.snapshots,
        );
        remoteVersionRef.current = record.version;
        lastRemoteWorkspaceRef.current = JSON.stringify(serialized);
        clearLegacyWorkflowWorkspace(projectId);
        applyWorkspace(workspace);
        setPersistenceError("");
      } catch {
        if (cancelled) return;
        setPersistenceError("流程暂时无法同步到任务面板服务");
        onWorkflowsChange(initialWorkspace.tabs);
      } finally {
        if (!cancelled) setHydrated(true);
      }
    }

    void hydrateWorkspace();
    return () => {
      cancelled = true;
    };
  }, [applyWorkspace, initialWorkspace, onWorkflowsChange, projectId]);

  useEffect(() => {
    const controller = new AbortController();
    setWorkflowCapabilities(null);
    setWorkflowCapabilitiesFailed(false);
    void listWorkflowCapabilities(workspacePath, controller.signal)
      .then(setWorkflowCapabilities)
      .catch(() => {
        if (controller.signal.aborted) return;
        setWorkflowCapabilities({ skills: [], mcpServers: [] });
        setWorkflowCapabilitiesFailed(true);
      });
    return () => controller.abort();
  }, [workspacePath]);

  useEffect(() => {
    workflowSnapshotsRef.current.set(activeWorkflowId, {
      nodes,
      flow,
      selectedNodeId: null,
    });
    const workspace = serializeWorkflowWorkspace(
      workflowTabs,
      activeWorkflowId,
      workflowSnapshotsRef.current,
    );
    onWorkflowsChange(workflowTabs);
    if (!hydrated) return;
    const serialized = JSON.stringify(workspace);
    if (serialized === lastRemoteWorkspaceRef.current) return;
    const timer = window.setTimeout(() => {
      saveQueueRef.current = saveQueueRef.current.then(async () => {
        if (!mountedRef.current || serialized === lastRemoteWorkspaceRef.current) return;
        try {
          const saved = await saveWorkflowWorkspace(
            projectId,
            workspace,
            remoteVersionRef.current,
          );
          if (!mountedRef.current) return;
          remoteVersionRef.current = saved.version;
          lastRemoteWorkspaceRef.current = serialized;
          clearLegacyWorkflowWorkspace(projectId);
          setPersistenceError("");
        } catch (error) {
          if (!mountedRef.current) return;
          if (error instanceof ApiError && error.code === "VERSION_CONFLICT") {
            try {
              const latest = await getWorkflowWorkspace<WorkflowWorkspace>(projectId);
              const latestWorkspace = parseWorkflowWorkspace(latest.workspace);
              if (latestWorkspace && mountedRef.current) {
                remoteVersionRef.current = latest.version;
                lastRemoteWorkspaceRef.current = JSON.stringify(latest.workspace);
                applyWorkspace(latestWorkspace);
                setPersistenceError("");
                return;
              }
            } catch {
              // The next edit retries after the service is reachable.
            }
          }
          setPersistenceError("流程保存失败，请稍后重试");
        }
      });
    }, 180);
    return () => window.clearTimeout(timer);
  }, [
    activeWorkflowId,
    applyWorkspace,
    flow,
    hydrated,
    nodes,
    onWorkflowsChange,
    projectId,
    workflowTabs,
  ]);

  useEffect(() => {
    if (!hydrated || revision === 0) return;
    const controller = new AbortController();
    void getWorkflowWorkspace<WorkflowWorkspace>(projectId, controller.signal)
      .then((record) => {
        if (record.version <= remoteVersionRef.current) return;
        const workspace = parseWorkflowWorkspace(record.workspace);
        if (!workspace) return;
        remoteVersionRef.current = record.version;
        lastRemoteWorkspaceRef.current = JSON.stringify(record.workspace);
        applyWorkspace(workspace);
        setPersistenceError("");
      })
      .catch((error) => {
        if ((error as Error).name !== "AbortError") {
          setPersistenceError("流程同步失败，请稍后重试");
        }
      });
    return () => controller.abort();
  }, [applyWorkspace, hydrated, projectId, revision]);

  useEffect(() => {
    if (!renamingWorkflowId) return;
    workflowNameInputRef.current?.focus();
    workflowNameInputRef.current?.select();
  }, [renamingWorkflowId]);

  useLayoutEffect(() => {
    if (!workflowTabMenu) return;
    const menu = workflowTabMenuRef.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    const x = Math.max(8, Math.min(workflowTabMenu.x, window.innerWidth - rect.width - 8));
    const y = Math.max(8, Math.min(workflowTabMenu.y, window.innerHeight - rect.height - 8));
    setWorkflowTabMenu((current) => (
      current && (current.x !== x || current.y !== y)
        ? { ...current, x, y }
        : current
    ));
  }, [workflowTabMenu]);

  useEffect(() => {
    if (!workflowTabMenu) return;
    const previousFocus = document.activeElement as HTMLElement | null;
    requestAnimationFrame(() => {
      workflowTabMenuRef.current?.querySelector<HTMLButtonElement>(".context-menu-item:not(:disabled)")?.focus();
    });

    function closeWorkflowTabMenuFromOutside(event: PointerEvent) {
      if (!workflowTabMenuRef.current?.contains(event.target as globalThis.Node)) {
        setWorkflowTabMenu(null);
      }
    }
    document.addEventListener("pointerdown", closeWorkflowTabMenuFromOutside);

    function closeWorkflowTabMenuFromEscape(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        setWorkflowTabMenu(null);
      }
    }
    document.addEventListener("keydown", closeWorkflowTabMenuFromEscape);

    function closeWorkflowTabMenuFromViewportChange() {
      setWorkflowTabMenu(null);
    }
    window.addEventListener("blur", closeWorkflowTabMenuFromViewportChange);
    window.addEventListener("resize", closeWorkflowTabMenuFromViewportChange);
    window.addEventListener("scroll", closeWorkflowTabMenuFromViewportChange, true);
    return () => {
      document.removeEventListener("pointerdown", closeWorkflowTabMenuFromOutside);
      document.removeEventListener("keydown", closeWorkflowTabMenuFromEscape);
      window.removeEventListener("blur", closeWorkflowTabMenuFromViewportChange);
      window.removeEventListener("resize", closeWorkflowTabMenuFromViewportChange);
      window.removeEventListener("scroll", closeWorkflowTabMenuFromViewportChange, true);
      previousFocus?.focus({ preventScroll: true });
    };
  }, [workflowTabMenu?.workflowId]);

  const commitFlow = useCallback((
    nextNodes: WorkflowCanvasNode[],
    nextFlow: WorkflowFlow,
  ) => {
    setNodes(layoutWorkflowFlow(nextNodes, nextFlow));
    setFlow(nextFlow);
  }, []);

  const commitNodes = useCallback((nextNodes: WorkflowCanvasNode[]) => {
    commitFlow(nextNodes, flow);
  }, [commitFlow, flow]);

  const openStepPicker = useCallback((
    sequenceRef: WorkflowSequenceRef,
    index: number,
  ) => {
    setPickerTarget({ kind: "sequence", sequenceRef, index });
  }, []);

  const openPlanStepPicker = useCallback((parentId: string) => {
    setPickerTarget({ kind: "plan", parentId });
  }, []);

  const updateSelectedNode = useCallback((changes: Partial<WorkflowNodeData>) => {
    if (!selectedNodeId) return;
    setNodes((current) => current.map((node) => (
      node.id === selectedNodeId
        ? { ...node, data: { ...node.data, ...changes } }
        : node
    )));
  }, [selectedNodeId]);

  const deleteNode = useCallback((nodeId: string) => {
    const node = nodes.find((candidate) => candidate.id === nodeId);
    if (!node || isWorkflowTriggerKind(node.data.kind)) return;
    if (node.parentId) {
      const nextNodes = nodes.filter((candidate) => candidate.id !== nodeId);
      commitNodes(nextNodes);
      setSelectedNodeId((current) => current === nodeId ? null : current);
      return;
    }
    const deleted = deleteWorkflowNode(flow, nodeId);
    const removedIds = new Set(deleted.removedNodeIds);
    for (const candidate of nodes) {
      if (candidate.parentId && removedIds.has(candidate.parentId)) {
        removedIds.add(candidate.id);
      }
    }
    const nextNodes = nodes.filter((candidate) => !removedIds.has(candidate.id));
    commitFlow(nextNodes, deleted.flow);
    setSelectedNodeId((current) => removedIds.has(current ?? "") ? null : current);
  }, [commitFlow, commitNodes, flow, nodes]);

  const duplicateNode = useCallback((nodeId: string) => {
    const source = nodes.find((candidate) => candidate.id === nodeId);
    if (!source || isWorkflowTriggerKind(source.data.kind) || source.data.kind === "condition") return;
    const duplicateId = `node-${crypto.randomUUID()}`;
    if (source.parentId) {
      const siblingIds = nodes
        .filter((candidate) => candidate.parentId === source.parentId)
        .sort((left, right) => left.position.y - right.position.y)
        .map((candidate) => candidate.id);
      const sourceIndex = siblingIds.indexOf(source.id);
      siblingIds.splice(sourceIndex + 1, 0, duplicateId);
      const nextNodes = [
        ...nodes,
        {
          ...source,
          id: duplicateId,
          selected: false,
          data: { ...source.data, title: `${source.data.title} 副本` },
        },
      ];
      commitNodes(layoutPlanChildren(nextNodes, source.parentId, siblingIds));
    } else {
      const found = findWorkflowItem(flow, source.id);
      if (!found) return;
      const nextNodes = [
        ...nodes,
        {
          ...source,
          id: duplicateId,
          selected: false,
          parentId: undefined,
          data: { ...source.data, title: `${source.data.title} 副本` },
        },
      ];
      const nextFlow = insertWorkflowNode(
        flow,
        found.sequenceRef,
        found.index + 1,
        duplicateId,
        source.data.kind,
      );
      commitFlow(nextNodes, nextFlow);
    }
    setSelectedNodeId(duplicateId);
  }, [commitFlow, commitNodes, flow, nodes]);

  const renderedNodes = useMemo(() => {
    const childCounts = new Map<string, number>();
    for (const node of nodes) {
      if (node.parentId) childCounts.set(node.parentId, (childCounts.get(node.parentId) ?? 0) + 1);
    }
    const orderedIds = workflowNodeIds(flow);
    const pinnedTriggerId = rootStepIds[0] && isWorkflowTriggerKind(
      nodes.find((node) => node.id === rootStepIds[0])?.data.kind ?? "",
    )
      ? rootStepIds[0]
      : null;
    const enriched = nodes.map((node) => {
      const stepIndex = orderedIds.indexOf(node.id);
      const dragShiftY = node.parentId ? planDragShift(node.id, planDragPreview) : 0;
      return {
        ...node,
        draggable: Boolean(node.parentId),
        data: {
          ...node.data,
          displayTitle: workflowNodeDisplayTitle(node.data),
          meta: capabilityNodeMeta(
            node.data,
            workflowCapabilities,
            workflowCapabilitiesFailed,
          ),
          configured: workflowNodeConfigured(
            node.data,
            workflowCapabilities,
            workflowCapabilitiesFailed,
          ),
          stepNumber: stepIndex >= 0 ? stepIndex + 1 : undefined,
          isTrigger: node.id === pinnedTriggerId,
          childCount: childCounts.get(node.id) ?? 0,
          dragShiftY,
          dragActive: planDragPreview?.nodeId === node.id,
          settleActive: settlingNodeId === node.id,
          onDuplicate: () => duplicateNode(node.id),
          onDelete: () => deleteNode(node.id),
          onAddChild: node.data.acceptsChildren
            ? () => openPlanStepPicker(node.id)
            : undefined,
        },
      };
    });
    const virtualNodes = layout.virtualNodes.map((virtualNode) => ({
      id: virtualNode.id,
      type: "workflow" as const,
      position: virtualNode.position,
      origin: TOP_CENTER_ORIGIN,
      draggable: false,
      selectable: false,
      deletable: false,
      connectable: false,
      style: { width: END_STEP_HEIGHT, height: END_STEP_HEIGHT },
      initialWidth: END_STEP_HEIGHT,
      initialHeight: END_STEP_HEIGHT,
      measured: { width: END_STEP_HEIGHT, height: END_STEP_HEIGHT },
      data: {
        kind: virtualNode.kind,
        eyebrow: "",
        title: "",
        description: "",
        meta: "",
        icon: "plus" as const,
        tone: "capability" as const,
      },
    }));
    return [
      ...enriched.filter((node) => !node.parentId),
      ...virtualNodes,
      ...enriched.filter((node) => node.parentId),
    ];
  }, [
    deleteNode,
    duplicateNode,
    flow,
    layout.virtualNodes,
    nodes,
    openPlanStepPicker,
    planDragPreview,
    rootStepIds,
    settlingNodeId,
    workflowCapabilities,
    workflowCapabilitiesFailed,
  ]);

  const renderedEdges = useMemo(() => {
    return layout.edges.map((edge) => ({
      ...edge,
      data: {
        ...edge.data,
        onInsert: edge.data.insertion
          ? () => openStepPicker(
              edge.data.insertion!.sequenceRef,
              edge.data.insertion!.index,
            )
          : undefined,
      },
    })) as Edge[];
  }, [layout.edges, openStepPicker]);

  const pickerItems = useMemo(() => {
    if (!pickerTarget) return [];
    if (pickerTarget.kind === "plan") {
      return PALETTE_ITEMS.filter((item) => NESTABLE_TONES.has(item.data.tone));
    }
    if (rootStepIds.length === 0) {
      return PALETTE_ITEMS.filter((item) => item.group === "触发器");
    }
    return PALETTE_ITEMS.filter((item) => item.group !== "触发器");
  }, [pickerTarget, rootStepIds.length]);

  function selectStep(item: PaletteItem) {
    if (!pickerTarget) return;
    const nodeId = `node-${crypto.randomUUID()}`;
    const newNode: WorkflowCanvasNode = {
      id: nodeId,
      type: "workflow",
      position: { x: 0, y: 0 },
      data: { ...item.data },
    };
    if (pickerTarget.kind === "plan") {
      newNode.parentId = pickerTarget.parentId;
      newNode.origin = TOP_LEFT_ORIGIN;
      const childIds = nodes
        .filter((node) => node.parentId === pickerTarget.parentId)
        .sort((left, right) => left.position.y - right.position.y)
        .map((node) => node.id);
      childIds.push(nodeId);
      const nextNodes = layoutPlanChildren([...nodes, newNode], pickerTarget.parentId, childIds);
      commitNodes(nextNodes);
    } else {
      const nextFlow = insertWorkflowNode(
        flow,
        pickerTarget.sequenceRef,
        pickerTarget.index,
        nodeId,
        item.data.kind,
      );
      commitFlow([...nodes, newNode], nextFlow);
    }
    setPickerTarget(null);
    setSelectedNodeId(nodeId);
  }

  function reorderPlanItem(parentId: string, nodeId: string, targetIndex: number) {
    const childIds = nodes
      .filter((node) => node.parentId === parentId)
      .sort((left, right) => left.position.y - right.position.y)
      .map((node) => node.id);
    const nextIds = childIds.filter((id) => id !== nodeId);
    nextIds.splice(Math.max(0, Math.min(targetIndex, nextIds.length)), 0, nodeId);
    commitNodes(layoutPlanChildren(nodes, parentId, nextIds));
  }

  const onNodesChange = useCallback((changes: NodeChange<WorkflowCanvasNode>[]) => {
    setNodes((current) => applyNodeChanges(
      changes.filter((change) => change.type !== "remove" || !isVirtualWorkflowNodeId(change.id)),
      current,
    ));
  }, []);

  const onNodeDragStart = useCallback<OnNodeDrag<WorkflowCanvasNode>>((_, node) => {
    if (!node.parentId) return;
    const sourceOrderIds = nodes
      .filter((candidate) => candidate.parentId === node.parentId)
      .sort((left, right) => left.position.y - right.position.y)
      .map((candidate) => candidate.id);
    const preview = {
      nodeId: node.id,
      parentId: node.parentId,
      sourceOrderIds,
      sourceIndex: sourceOrderIds.indexOf(node.id),
      targetIndex: sourceOrderIds.indexOf(node.id),
    };
    planDragSessionRef.current = preview;
    setPlanDragPreview(preview);
    setSettlingNodeId(null);
  }, [nodes]);

  const onNodeDrag = useCallback<OnNodeDrag<WorkflowCanvasNode>>((_, node) => {
    const instance = flowRef.current;
    const internal = instance?.getInternalNode(node.id);
    if (!instance || !internal) return;
    const height = internal.measured.height ?? (node.parentId ? PLAN_ITEM_HEIGHT : WORKFLOW_STEP_HEIGHT);
    const centerY = internal.internals.positionAbsolute.y + height / 2;
    if (node.parentId && planDragSessionRef.current?.nodeId === node.id) {
      const session = planDragSessionRef.current;
      const siblings = session.sourceOrderIds
        .filter((id) => id !== node.id)
        .map((id) => instance.getInternalNode(id))
        .filter((candidate) => candidate !== undefined);
      const index = siblings.findIndex((candidate) => (
        centerY < candidate.internals.positionAbsolute.y
          + (candidate.measured.height ?? PLAN_ITEM_HEIGHT) / 2
      ));
      const targetIndex = index < 0 ? siblings.length : index;
      setPlanDragPreview({ ...session, targetIndex });
    }
  }, []);

  const onNodeDragStop = useCallback<OnNodeDrag<WorkflowCanvasNode>>((_, node) => {
    if (!node.parentId || planDragSessionRef.current?.nodeId !== node.id) return;
    const session = planDragSessionRef.current;
    reorderPlanItem(session.parentId, node.id, planDragPreview?.targetIndex ?? session.sourceIndex);
    planDragSessionRef.current = null;
    setPlanDragPreview(null);
    setSettlingNodeId(node.id);
    if (settleTimerRef.current !== null) window.clearTimeout(settleTimerRef.current);
    settleTimerRef.current = window.setTimeout(() => setSettlingNodeId(null), 220);
  }, [planDragPreview, reorderPlanItem]);

  function activateWorkflow(workflowId: string) {
    setWorkflowTabMenu(null);
    if (workflowId === activeWorkflowId) return;
    workflowSnapshotsRef.current.set(activeWorkflowId, {
      nodes,
      flow,
      selectedNodeId: null,
    });
    const snapshot = normalizeSnapshot(workflowSnapshotsRef.current.get(workflowId)!);
    setActiveWorkflowId(workflowId);
    setNodes(snapshot.nodes);
    setFlow(snapshot.flow);
    setSelectedNodeId(null);
    setPickerTarget(null);
    requestAnimationFrame(() => {
      void flowRef.current?.fitView({ padding: 0.2, duration: 240, maxZoom: 1 });
    });
  }

  function createWorkflow() {
    setWorkflowTabMenu(null);
    const workflowId = `workflow-${crypto.randomUUID()}`;
    const workflowName = `未命名流程 ${workflowTabs.length + 1}`;
    const emptyFlow = createWorkflowFlow();
    workflowSnapshotsRef.current.set(workflowId, {
      nodes: [],
      flow: emptyFlow,
      selectedNodeId: null,
    });
    setWorkflowTabs((current) => [...current, { id: workflowId, name: workflowName }]);
    setActiveWorkflowId(workflowId);
    setNodes([]);
    setFlow(emptyFlow);
    setSelectedNodeId(null);
    setPickerTarget(null);
  }

  function openWorkflowTabMenu(
    event: ReactMouseEvent<HTMLButtonElement>,
    workflowId: string,
  ) {
    event.preventDefault();
    setWorkflowTabMenu({
      workflowId,
      x: event.clientX,
      y: event.clientY,
    });
  }

  function deleteWorkflow(workflowId: string) {
    if (workflowTabs.length <= 1) return;
    const workflowIndex = workflowTabs.findIndex((workflow) => workflow.id === workflowId);
    const nextTabs = workflowTabs.filter((workflow) => workflow.id !== workflowId);
    workflowSnapshotsRef.current.delete(workflowId);

    if (workflowId !== activeWorkflowId) {
      setWorkflowTabs(nextTabs);
      setWorkflowTabMenu(null);
      return;
    }

    const replacement = workflowTabs[workflowIndex + 1] ?? workflowTabs[workflowIndex - 1];
    const snapshot = normalizeSnapshot(workflowSnapshotsRef.current.get(replacement.id)!);
    setWorkflowTabs(nextTabs);
    setActiveWorkflowId(replacement.id);
    setNodes(snapshot.nodes);
    setFlow(snapshot.flow);
    setSelectedNodeId(null);
    setPickerTarget(null);
    setRenamingWorkflowId(null);
    setWorkflowNameDraft("");
    cancelWorkflowRenameRef.current = false;
    setWorkflowTabMenu(null);
    requestAnimationFrame(() => {
      void flowRef.current?.fitView({ padding: 0.2, duration: 240, maxZoom: 1 });
    });
  }

  function handleWorkflowTabKeyDown(
    event: KeyboardEvent<HTMLButtonElement>,
    workflowId: string,
  ) {
    const currentIndex = workflowTabs.findIndex((workflow) => workflow.id === workflowId);
    let targetIndex = currentIndex;
    if (event.key === "ArrowLeft") targetIndex = (currentIndex - 1 + workflowTabs.length) % workflowTabs.length;
    else if (event.key === "ArrowRight") targetIndex = (currentIndex + 1) % workflowTabs.length;
    else if (event.key === "Home") targetIndex = 0;
    else if (event.key === "End") targetIndex = workflowTabs.length - 1;
    else return;
    event.preventDefault();
    const target = workflowTabs[targetIndex];
    activateWorkflow(target.id);
    requestAnimationFrame(() => {
      document.getElementById(`workflow-tab-${target.id}`)?.focus();
    });
  }

  function startWorkflowRename(workflow: WorkflowTab) {
    if (workflow.id !== activeWorkflowId) activateWorkflow(workflow.id);
    cancelWorkflowRenameRef.current = false;
    setWorkflowNameDraft(workflow.name);
    setRenamingWorkflowId(workflow.id);
  }

  function commitWorkflowRename(workflowId: string) {
    if (cancelWorkflowRenameRef.current) {
      cancelWorkflowRenameRef.current = false;
      return;
    }
    const name = workflowNameDraft.trim();
    if (name) {
      setWorkflowTabs((current) => current.map((workflow) => (
        workflow.id === workflowId ? { ...workflow, name } : workflow
      )));
    }
    setRenamingWorkflowId(null);
    setWorkflowNameDraft("");
  }

  function cancelWorkflowRename(workflowId: string) {
    cancelWorkflowRenameRef.current = true;
    setRenamingWorkflowId(null);
    setWorkflowNameDraft("");
    requestAnimationFrame(() => {
      document.getElementById(`workflow-tab-${workflowId}`)?.focus();
    });
  }

  return (
    <section
      className={`workflow-board${selectedNode ? " has-inspector" : ""}`}
      aria-label="流程看板"
    >
      <div className="workflow-canvas-shell">
        <div className="workflow-canvas-toolbar">
          <div className="workflow-tabs" role="tablist" aria-label={`${projectName} 的流程`}>
            {workflowTabs.map((workflow) => {
              const active = workflow.id === activeWorkflowId;
              if (workflow.id === renamingWorkflowId) {
                return (
                  <div
                    id={`workflow-tab-${workflow.id}`}
                    className={`workflow-tab is-renaming${active ? " is-active" : ""}`}
                    role="tab"
                    aria-controls="workflow-canvas-panel"
                    aria-selected={active}
                    key={workflow.id}
                  >
                    <LinearIcon name="dashboard" />
                    <input
                      ref={workflowNameInputRef}
                      type="text"
                      aria-label="流程名称"
                      value={workflowNameDraft}
                      onChange={(event) => setWorkflowNameDraft(event.target.value)}
                      onBlur={() => commitWorkflowRename(workflow.id)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          event.currentTarget.blur();
                        } else if (event.key === "Escape") {
                          event.preventDefault();
                          cancelWorkflowRename(workflow.id);
                        }
                      }}
                    />
                  </div>
                );
              }
              return (
                <button
                  id={`workflow-tab-${workflow.id}`}
                  className={`workflow-tab${active ? " is-active" : ""}`}
                  type="button"
                  role="tab"
                  aria-controls="workflow-canvas-panel"
                  aria-selected={active}
                  tabIndex={active ? 0 : -1}
                  key={workflow.id}
                  onClick={() => activateWorkflow(workflow.id)}
                  onDoubleClick={() => startWorkflowRename(workflow)}
                  onContextMenu={(event) => openWorkflowTabMenu(event, workflow.id)}
                  onKeyDown={(event) => handleWorkflowTabKeyDown(event, workflow.id)}
                  title="双击重命名"
                >
                  <LinearIcon name="dashboard" />
                  <span>{workflow.name}</span>
                </button>
              );
            })}
            <button
              className="workflow-tab-add"
              type="button"
              aria-label="新建流程"
              title="新建流程"
              onClick={createWorkflow}
            >
              <LinearIcon name="plus" />
            </button>
          </div>
          <div className="workflow-toolbar-status">
            <span className={persistenceError ? "has-error" : ""}>
              <i aria-hidden="true" />
              {persistenceError || "已自动保存"}
            </span>
          </div>
        </div>
        <div
          ref={canvasRef}
          className="workflow-canvas"
          id="workflow-canvas-panel"
          role="tabpanel"
          aria-label="流程编排区"
          aria-labelledby={`workflow-tab-${activeWorkflowId}`}
        >
          <ReactFlow<WorkflowCanvasNode, Edge>
            nodes={renderedNodes}
            edges={renderedEdges}
            nodeTypes={NODE_TYPES}
            edgeTypes={EDGE_TYPES}
            nodeOrigin={TOP_CENTER_ORIGIN}
            onNodesChange={onNodesChange}
            onNodeDragStart={onNodeDragStart}
            onNodeDrag={onNodeDrag}
            onNodeDragStop={onNodeDragStop}
            onNodeClick={(_, node) => {
              if (!isVirtualWorkflowNodeId(node.id)) setSelectedNodeId(node.id);
            }}
            onPaneClick={() => setSelectedNodeId(null)}
            nodesDraggable={false}
            nodesConnectable={false}
            connectOnClick={false}
            deleteKeyCode={null}
            selectionOnDrag={false}
            panOnScroll
            panOnDrag
            zoomOnDoubleClick={false}
            minZoom={0.45}
            maxZoom={1.35}
            translateExtent={translateExtent}
            onMoveEnd={(_, viewport) => setCanvasZoom(viewport.zoom)}
            proOptions={PRO_OPTIONS}
            onInit={(instance) => {
              flowRef.current = instance;
              void instance.setCenter(0, 220, { zoom: 1 });
            }}
          >
            <Background
              color="var(--border-strong)"
              gap={24}
              size={0.75}
              variant={BackgroundVariant.Dots}
            />
            <Controls
              className="workflow-flow-controls"
              position="bottom-left"
              showInteractive={false}
            />
          </ReactFlow>
          {rootStepIds.length === 0 && (
            <button
              className="workflow-empty-add"
              type="button"
              aria-label="添加第一个步骤"
              onClick={() => openStepPicker([], 0)}
            >
              <LinearIcon name="plus" />
              <span>添加触发器</span>
            </button>
          )}
          {pickerTarget && (
            <WorkflowStepPicker
              items={pickerItems}
              onSelect={selectStep}
              onClose={() => setPickerTarget(null)}
            />
          )}
        </div>
      </div>

      {selectedNode && (
        <aside className="workflow-inspector workflow-step-inspector" aria-label="步骤配置">
          <WorkflowInspector
            node={selectedNode}
            projectName={projectName}
            capabilities={workflowCapabilities}
            capabilitiesFailed={workflowCapabilitiesFailed}
            onChange={updateSelectedNode}
            onClose={() => setSelectedNodeId(null)}
          />
        </aside>
      )}

      {workflowTabMenu && createPortal(
        <div
          ref={workflowTabMenuRef}
          className="task-context-menu workflow-tab-context-menu"
          role="menu"
          aria-label="流程操作"
          style={{ left: workflowTabMenu.x, top: workflowTabMenu.y }}
          onContextMenu={(event) => event.preventDefault()}
        >
          <div className="context-menu-group">
            <button
              className="context-menu-item is-danger"
              type="button"
              role="menuitem"
              disabled={workflowTabs.length === 1}
              aria-disabled={workflowTabs.length === 1}
              onClick={() => deleteWorkflow(workflowTabMenu.workflowId)}
            >
              <span className="context-menu-icon"><LinearIcon name="trash" /></span>
              <span className="context-menu-label">删除流程</span>
            </button>
          </div>
        </div>,
        document.body,
      )}
    </section>
  );
}
