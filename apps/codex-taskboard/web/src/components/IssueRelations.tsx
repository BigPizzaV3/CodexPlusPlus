import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";

import type {
  IssueRelationType,
  Task,
  TaskRelationSummary,
} from "../types";
import { ActorAvatar } from "./ActorAvatar";
import { LinearIcon, LinearStatusIcon } from "./LinearIcon";

export interface RelationMutationResult {
  task: Task;
  relatedTask: Task;
}

interface RelationActions {
  task: Task;
  tasks: Task[];
  onOpenTask: (task: TaskRelationSummary) => void;
  onOpenThread?: (threadId: string, workspacePath?: string, threadTitle?: string) => void;
  onAddRelation: (
    task: Task,
    type: IssueRelationType,
    relatedTaskId: string,
  ) => Promise<RelationMutationResult>;
  onRemoveRelation: (
    task: Task,
    type: IssueRelationType,
    relatedTaskId: string,
  ) => Promise<RelationMutationResult>;
}

function IssuePicker({
  label,
  candidates,
  disabled,
  onSelect,
}: {
  label: string;
  candidates: Task[];
  disabled?: boolean;
  onSelect: (task: Task) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [savingId, setSavingId] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const results = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return candidates;
    return candidates.filter((task) => (
      task.identifier.toLocaleLowerCase().includes(normalized)
      || task.title.toLocaleLowerCase().includes(normalized)
    ));
  }, [candidates, query]);

  useEffect(() => {
    if (!open) return;
    setActiveIndex(0);
    requestAnimationFrame(() => inputRef.current?.focus());
    const close = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open]);

  async function choose(task: Task) {
    setSavingId(task.id);
    try {
      await onSelect(task);
      setOpen(false);
      setQuery("");
    } catch {
    } finally {
      setSavingId(null);
    }
  }

  return (
    <div className="issue-relation-picker" ref={rootRef}>
      <button
        className="issue-relation-add"
        type="button"
        disabled={disabled}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <LinearIcon name="plus" />
        <span>{label}</span>
      </button>
      {open && (
        <div className="issue-relation-popover">
          <div className="issue-relation-search">
            <LinearIcon name="search" />
            <input
              ref={inputRef}
              value={query}
              role="combobox"
              aria-expanded="true"
              aria-controls="issue-relation-results"
              aria-activedescendant={results[activeIndex] ? `relation-option-${results[activeIndex].id}` : undefined}
              placeholder="搜索任务…"
              onChange={(event) => {
                setQuery(event.target.value);
                setActiveIndex(0);
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  setOpen(false);
                } else if (event.key === "ArrowDown" && results.length > 0) {
                  event.preventDefault();
                  setActiveIndex((index) => (index + 1) % results.length);
                } else if (event.key === "ArrowUp" && results.length > 0) {
                  event.preventDefault();
                  setActiveIndex((index) => (index - 1 + results.length) % results.length);
                } else if (event.key === "Enter" && results[activeIndex]) {
                  event.preventDefault();
                  void choose(results[activeIndex]);
                }
              }}
            />
          </div>
          <div className="issue-relation-results" id="issue-relation-results" role="listbox">
            {results.length > 0 ? results.map((candidate, index) => (
              <button
                id={`relation-option-${candidate.id}`}
                className={index === activeIndex ? "is-active" : ""}
                type="button"
                role="option"
                aria-selected={index === activeIndex}
                disabled={savingId !== null}
                key={candidate.id}
                onMouseEnter={() => setActiveIndex(index)}
                onClick={() => void choose(candidate)}
              >
                <LinearStatusIcon status={candidate.status} />
                <span className="issue-relation-option-id">{candidate.identifier}</span>
                <span className="issue-relation-option-title">{candidate.title}</span>
              </button>
            )) : (
              <p className="issue-relation-empty">没有匹配的任务</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function descendantIds(task: Task, tasks: Task[]) {
  const descendants = new Set<string>();
  const queue = [...task.relations.subIssues.map((item) => item.id)];
  const taskById = new Map(tasks.map((candidate) => [candidate.id, candidate]));
  while (queue.length > 0) {
    const id = queue.shift()!;
    if (descendants.has(id)) continue;
    descendants.add(id);
    const child = taskById.get(id);
    if (child) queue.push(...child.relations.subIssues.map((item) => item.id));
  }
  return descendants;
}

function IssueRelationRow({
  issue,
  onOpen,
  onOpenThread,
  onRemove,
  removing,
  showAssignee = false,
}: {
  issue: TaskRelationSummary;
  onOpen: () => void;
  onOpenThread?: (threadId: string, workspacePath?: string, threadTitle?: string) => void;
  onRemove: () => void;
  removing: boolean;
  showAssignee?: boolean;
}) {
  return (
    <div className="issue-relation-row">
      <button className="issue-relation-target" type="button" onClick={onOpen}>
        <LinearStatusIcon status={issue.status} />
        <span className="issue-relation-id">{issue.identifier}</span>
        <span className="issue-relation-title">{issue.title}</span>
        {showAssignee && <ActorAvatar actor={issue.assignee} className="issue-relation-assignee" />}
      </button>
      {issue.threadId && onOpenThread && (
        <button
          className="issue-relation-thread"
          type="button"
          aria-label={`查看对话 ${issue.threadId}`}
          title={`查看对话 ${issue.threadId}`}
          onClick={() => onOpenThread(issue.threadId!, undefined, issue.title)}
        >
          <LinearIcon name="conversation" />
        </button>
      )}
      <button
        className="issue-relation-remove"
        type="button"
        aria-label={`移除 ${issue.identifier}`}
        disabled={removing}
        onClick={onRemove}
      >
        <LinearIcon name="close" />
      </button>
    </div>
  );
}

export function IssueParentLink({
  task,
  tasks,
  onOpenTask,
  onOpenThread,
  onAddRelation,
  onRemoveRelation,
}: RelationActions) {
  const [saving, setSaving] = useState(false);
  const parent = task.relations.parent;
  const excluded = descendantIds(task, tasks);
  excluded.add(task.id);
  const candidates = tasks.filter((candidate) => (
    candidate.archivedAt === null
    && !excluded.has(candidate.id)
    && candidate.id !== parent?.id
  ));

  return (
    <div className={`issue-parent-link${parent ? " has-parent" : ""}`}>
      {parent && (
        <>
          <span className="issue-parent-prefix">子任务属于</span>
          <IssueRelationRow
            issue={parent}
            removing={saving}
            onOpen={() => onOpenTask(parent)}
            onOpenThread={onOpenThread}
            onRemove={() => {
              setSaving(true);
              void onRemoveRelation(task, "parent", parent.id)
                .catch(() => undefined)
                .finally(() => setSaving(false));
            }}
          />
        </>
      )}
      <IssuePicker
        label={parent ? "更换父任务" : "设置父任务"}
        candidates={candidates}
        disabled={saving}
        onSelect={async (candidate) => {
          setSaving(true);
          try {
            await onAddRelation(task, "parent", candidate.id);
          } finally {
            setSaving(false);
          }
        }}
      />
    </div>
  );
}

export function IssueSubIssues({
  task,
  tasks,
  onOpenTask,
  onOpenThread,
  onAddRelation,
  onRemoveRelation,
}: RelationActions) {
  const [savingId, setSavingId] = useState<string | null>(null);
  const subIssues = task.relations.subIssues;
  const done = subIssues.filter((issue) => issue.status === "done").length;
  const directIds = new Set(subIssues.map((issue) => issue.id));
  const ancestors = new Set<string>([task.id]);
  let parent = task.relations.parent;
  const taskById = new Map(tasks.map((candidate) => [candidate.id, candidate]));
  while (parent && !ancestors.has(parent.id)) {
    ancestors.add(parent.id);
    parent = taskById.get(parent.id)?.relations.parent ?? null;
  }
  const candidates = tasks.filter((candidate) => (
    candidate.archivedAt === null
    && !ancestors.has(candidate.id)
    && !directIds.has(candidate.id)
  ));
  const progress = subIssues.length > 0 ? Math.round((done / subIssues.length) * 100) : 0;

  return (
    <section className="issue-sub-issues" aria-labelledby="sub-issues-heading">
      <header>
        <div>
          <h2 id="sub-issues-heading">子任务</h2>
          {subIssues.length > 0 && (
            <span className="sub-issue-summary">
              <span
                className="sub-issue-progress"
                style={{ "--sub-issue-progress": `${progress}%` } as CSSProperties}
                aria-hidden="true"
              />
              {done}/{subIssues.length}
            </span>
          )}
        </div>
        <IssuePicker
          label="添加子任务"
          candidates={candidates}
          disabled={savingId !== null}
          onSelect={async (candidate) => {
            setSavingId(candidate.id);
            try {
              await onAddRelation(candidate, "parent", task.id);
            } finally {
              setSavingId(null);
            }
          }}
        />
      </header>
      {subIssues.length > 0 && (
        <div className="issue-sub-issue-list">
          {subIssues.map((issue) => {
            const child = taskById.get(issue.id);
            return (
              <IssueRelationRow
                issue={issue}
                key={issue.id}
                showAssignee
                removing={savingId === issue.id}
                onOpen={() => onOpenTask(issue)}
                onOpenThread={onOpenThread}
                onRemove={() => {
                  if (!child) return;
                  setSavingId(issue.id);
                  void onRemoveRelation(child, "parent", task.id)
                    .catch(() => undefined)
                    .finally(() => setSavingId(null));
                }}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}

const RELATION_GROUPS = [
  { type: "blocked_by", field: "blockedBy", label: "阻塞于", tone: "blocked-by" },
  { type: "blocks", field: "blocks", label: "阻塞", tone: "blocks" },
  { type: "related", field: "related", label: "相关任务", tone: "related" },
] as const;

export function IssueRelationSidebar({
  task,
  tasks,
  onOpenTask,
  onOpenThread,
  onAddRelation,
  onRemoveRelation,
}: RelationActions) {
  const [savingKey, setSavingKey] = useState<string | null>(null);

  return (
    <section className="issue-relation-sidebar" aria-labelledby="relations-heading">
      <h2 id="relations-heading">关系</h2>
      {RELATION_GROUPS.map((group) => {
        const issues = task.relations[group.field];
        const existing = new Set(issues.map((issue) => issue.id));
        const candidates = tasks.filter((candidate) => (
          candidate.archivedAt === null
          && candidate.id !== task.id
          && !existing.has(candidate.id)
        ));
        return (
          <div className={`issue-relation-group is-${group.tone}`} key={group.type}>
            <header>
              <span>
                <LinearIcon name={group.type === "related" ? "link" : "alert"} />
                {group.label}
              </span>
              <IssuePicker
                label={`添加${group.label}`}
                candidates={candidates}
                disabled={savingKey !== null}
                onSelect={async (candidate) => {
                  const key = `${group.type}:${candidate.id}`;
                  setSavingKey(key);
                  try {
                    await onAddRelation(task, group.type, candidate.id);
                  } finally {
                    setSavingKey(null);
                  }
                }}
              />
            </header>
            {issues.map((issue) => (
              <IssueRelationRow
                issue={issue}
                key={issue.id}
                removing={savingKey === `${group.type}:${issue.id}`}
                onOpen={() => onOpenTask(issue)}
                onOpenThread={onOpenThread}
                onRemove={() => {
                  const key = `${group.type}:${issue.id}`;
                  setSavingKey(key);
                  void onRemoveRelation(task, group.type, issue.id)
                    .catch(() => undefined)
                    .finally(() => setSavingKey(null));
                }}
              />
            ))}
          </div>
        );
      })}
    </section>
  );
}
