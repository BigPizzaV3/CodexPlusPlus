import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import { LinearIcon, type LinearIconName } from "./LinearIcon";
import { WorkflowMark } from "./WorkflowMark";

export type WorkflowNodeTone =
  | "issue"
  | "capability"
  | "api"
  | "integration"
  | "development"
  | "planning"
  | "result";

export interface WorkflowNodeData extends Record<string, unknown> {
  kind: string;
  eyebrow: string;
  title: string;
  displayTitle?: string;
  description: string;
  meta: string;
  icon: LinearIconName;
  logo?: string;
  logoMonochrome?: boolean;
  tone: WorkflowNodeTone;
  inputLabel?: string;
  outputLabel?: string;
  additionalInstructions?: string;
  selectedSkill?: string;
  selectedMcpServer?: string;
  claudeModel?: string;
  reasoningEffort?: string;
  planningRequirements?: string;
  issueTarget?: string;
  specificIssueId?: string;
  changeStatus?: boolean;
  targetStatus?: string;
  addComment?: boolean;
  commentSource?: string;
  customComment?: string;
  addLabels?: boolean;
  labelsToAdd?: string;
  setPriority?: boolean;
  targetPriority?: string;
  attachArtifacts?: boolean;
  recordConversation?: boolean;
  triggerStatus?: string;
  createIssueTitle?: string;
  createIssueDescription?: string;
  createIssueStatus?: string;
  createIssuePriority?: string;
  createIssueLabels?: string;
  gitOperation?: string;
  gitCommitMessage?: string;
  gitStageAll?: boolean;
  gitRemote?: string;
  gitBranchName?: string;
  gitBaseBranch?: string;
  gitWorktreePath?: string;
  conditionField?: string;
  conditionOperator?: string;
  conditionValue?: string;
  feishuRecipientType?: "self" | "user" | "chat";
  feishuUserId?: string;
  feishuChatId?: string;
  twitterPostContent?: string;
  rssFeedUrl?: string;
  codeRuntime?: "shell" | "javascript" | "python";
  codeContent?: string;
  testScope?: "related" | "all" | "custom";
  testCommand?: string;
  acceptsChildren?: boolean;
  childCount?: number;
  stepNumber?: number;
  configured?: boolean;
  isTrigger?: boolean;
  dragShiftY?: number;
  dragActive?: boolean;
  settleActive?: boolean;
  conditionOutcome?: "true" | "false";
  onDuplicate?: () => void;
  onDelete?: () => void;
  onAddChild?: () => void;
}

export type WorkflowCanvasNode = Node<WorkflowNodeData, "workflow">;

function StepMenu({
  canDelete,
  canDuplicate,
  onDelete,
  onDuplicate,
}: {
  canDelete: boolean;
  canDuplicate: boolean;
  onDelete?: () => void;
  onDuplicate?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ top: 0, left: 0 });
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (!open) return;
    const trigger = triggerRef.current;
    const popover = popoverRef.current;
    if (!trigger || !popover) return;
    const triggerRect = trigger.getBoundingClientRect();
    const popoverRect = popover.getBoundingClientRect();
    setPosition({
      top: Math.min(triggerRect.bottom + 4, window.innerHeight - popoverRect.height - 8),
      left: Math.max(8, Math.min(
        triggerRect.right - popoverRect.width,
        window.innerWidth - popoverRect.width - 8,
      )),
    });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function close(event: PointerEvent) {
      if (
        menuRef.current?.contains(event.target as globalThis.Node)
        || popoverRef.current?.contains(event.target as globalThis.Node)
      ) return;
      setOpen(false);
    }
    function closeOnViewportChange() {
      setOpen(false);
    }
    document.addEventListener("pointerdown", close);
    document.addEventListener("wheel", closeOnViewportChange, true);
    window.addEventListener("resize", closeOnViewportChange);
    window.addEventListener("scroll", closeOnViewportChange, true);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("wheel", closeOnViewportChange, true);
      window.removeEventListener("resize", closeOnViewportChange);
      window.removeEventListener("scroll", closeOnViewportChange, true);
    };
  }, [open]);

  if (!canDelete && !canDuplicate) return null;

  return (
    <div className="workflow-step-menu workflow-node-menu nodrag nopan" ref={menuRef}>
      <button
        ref={triggerRef}
        className="workflow-step-menu-trigger"
        type="button"
        aria-label="步骤操作"
        aria-expanded={open}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((current) => !current);
        }}
      >
        <LinearIcon name="more" />
      </button>
      {open && createPortal(
        <div
          ref={popoverRef}
          className="workflow-step-menu-popover is-portaled nodrag nopan"
          role="menu"
          style={{ top: position.top, left: position.left }}
        >
          {canDuplicate && (
            <button
              type="button"
              role="menuitem"
              onClick={(event) => {
                event.stopPropagation();
                setOpen(false);
                onDuplicate?.();
              }}
            >
              <LinearIcon name="copy" />
              <span>复制步骤</span>
            </button>
          )}
          {canDelete && (
            <button
              className="is-danger"
              type="button"
              role="menuitem"
              onClick={(event) => {
                event.stopPropagation();
                setOpen(false);
                onDelete?.();
              }}
            >
              <LinearIcon name="trash" />
              <span>删除步骤</span>
            </button>
          )}
        </div>,
        document.body,
      )}
    </div>
  );
}

export function WorkflowNode({ data, selected, isConnectable, parentId }: NodeProps<WorkflowCanvasNode>) {
  if (data.kind.startsWith("flow-")) {
    const hasOutput = data.kind !== "flow-end";
    return (
      <div className={`workflow-flow-anchor workflow-${data.kind}`} aria-hidden="true">
        <Handle
          className="workflow-sequence-handle workflow-sequence-handle-input"
          type="target"
          position={Position.Top}
          isConnectable={false}
        />
        {hasOutput && (
          <Handle
            className="workflow-sequence-handle workflow-sequence-handle-output"
            type="source"
            position={Position.Bottom}
            isConnectable={false}
          />
        )}
      </div>
    );
  }

  if (parentId) {
    return (
      <article
        className={`workflow-node-compact workflow-node-${data.tone}${selected ? " selected" : ""}${data.dragShiftY ? " is-drag-shifted" : ""}${data.dragActive ? " is-dragging" : ""}${data.settleActive ? " is-settling" : ""}`}
        style={data.dragShiftY ? { transform: `translate3d(0, ${data.dragShiftY}px, 0)` } : undefined}
      >
        <span className="workflow-node-icon" aria-hidden="true">
          <WorkflowMark
            icon={data.icon}
            logo={data.logo}
            logoMonochrome={data.logoMonochrome}
          />
        </span>
        <strong>{data.displayTitle ?? data.title}</strong>
      </article>
    );
  }

  if (data.acceptsChildren) {
    return (
      <article
        className={`workflow-plan-container${selected ? " selected" : ""}${data.dragShiftY ? " is-drag-shifted" : ""}${data.dragActive ? " is-dragging" : ""}${data.settleActive ? " is-settling" : ""}`}
        style={data.dragShiftY ? { transform: `translate3d(0, ${data.dragShiftY}px, 0)` } : undefined}
      >
        <Handle
          className="workflow-sequence-handle workflow-sequence-handle-input"
          type="target"
          position={Position.Top}
          isConnectable={isConnectable}
          aria-label={data.inputLabel ?? "步骤输入"}
        />
        <header className="workflow-plan-container-header">
          <span className="workflow-node-icon" aria-hidden="true">
            <WorkflowMark
              icon={data.icon}
              logo={data.logo}
              logoMonochrome={data.logoMonochrome}
            />
          </span>
          <span className="workflow-node-heading">
            <span>{data.eyebrow}</span>
            <strong>{data.displayTitle ?? data.title}</strong>
          </span>
          <StepMenu
            canDelete={!data.isTrigger}
            canDuplicate={!data.isTrigger && data.kind !== "condition"}
            onDelete={data.onDelete}
            onDuplicate={data.onDuplicate}
          />
        </header>
        <div className="workflow-plan-container-summary">
          <p>{data.description}</p>
        </div>
        <div className="workflow-plan-drop-zone">
          {(data.childCount ?? 0) === 0 && <span>执行计划中还没有步骤</span>}
        </div>
        <footer className="workflow-plan-container-footer">
          <span>从上到下执行 · {data.childCount ?? 0} 步</span>
          <button
            className="workflow-plan-add-inline nodrag nopan"
            type="button"
            aria-label="向执行计划添加步骤"
            onClick={(event) => {
              event.stopPropagation();
              data.onAddChild?.();
            }}
          >
            <LinearIcon name="plus" />
            <span>添加步骤</span>
          </button>
        </footer>
        <Handle
          className="workflow-sequence-handle workflow-sequence-handle-output"
          type="source"
          position={Position.Bottom}
          isConnectable={isConnectable}
          aria-label={data.outputLabel ?? "步骤输出"}
        />
      </article>
    );
  }

  return (
    <article
      className={`workflow-node workflow-node-${data.tone}${selected ? " selected" : ""}${data.dragShiftY ? " is-drag-shifted" : ""}${data.dragActive ? " is-dragging" : ""}${data.settleActive ? " is-settling" : ""}`}
      style={data.dragShiftY ? { transform: `translate3d(0, ${data.dragShiftY}px, 0)` } : undefined}
    >
      <Handle
        className="workflow-sequence-handle workflow-sequence-handle-input"
        type="target"
        position={Position.Top}
        isConnectable={isConnectable}
        aria-label={data.inputLabel ?? "步骤输入"}
      />
      <header className="workflow-node-header">
        <span className="workflow-node-icon" aria-hidden="true">
          <WorkflowMark
            icon={data.icon}
            logo={data.logo}
            logoMonochrome={data.logoMonochrome}
          />
        </span>
        <span className="workflow-node-heading">
          <span>{data.eyebrow}</span>
          <strong>{data.displayTitle ?? data.title}</strong>
        </span>
        <StepMenu
          canDelete={!data.isTrigger}
          canDuplicate={!data.isTrigger && data.kind !== "condition"}
          onDelete={data.onDelete}
          onDuplicate={data.onDuplicate}
        />
      </header>
      <div className="workflow-node-body">
        <p>{data.description}</p>
        <span>{data.meta}</span>
      </div>
      <footer className="workflow-node-footer">
        <span className={`workflow-node-state${data.configured ? " is-configured" : " needs-config"}`}>
          <i aria-hidden="true" />
          {data.configured ? "已配置" : "需要配置"}
        </span>
        <span>{data.isTrigger ? "触发步骤" : `步骤 ${data.stepNumber ?? ""}`}</span>
      </footer>
      <Handle
        className="workflow-sequence-handle workflow-sequence-handle-output"
        type="source"
        position={Position.Bottom}
        isConnectable={isConnectable}
        aria-label={data.outputLabel ?? "步骤输出"}
      />
    </article>
  );
}
