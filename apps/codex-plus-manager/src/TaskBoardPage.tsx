/**
 * 任务面板：记下要做什么，看清每件事走到了哪一步。
 *
 * 数据在 core 的 `task_board`（每个任务一个 Markdown 文件），前端只调 `task_board_request`，
 * 每个动作都返回整个面板。窗口重新获得焦点时重读一次，Agent 或编辑器改过文件也能看到。
 *
 * 布局：工具栏（搜索、视图、显示已取消）→ 看板或列表 → 右侧详情（选中时）→ 底部状态栏。
 */
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Columns3, List, Plus, Trash2, X } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { appConfirm } from "./app-dialog";
import { PageHeader } from "./ManagementPages";
import { CxBanner, CxToolbar } from "./cx-layout";
import { relativeTime } from "./gateway-model";
import { t, tf } from "./i18n";
import {
  TASK_STATUSES,
  boardSummary,
  groupByStatus,
  matchesQuery,
  projectName,
  statusLabel,
  taskKey,
  type Task,
  type TaskBoard,
  type TaskStatus,
} from "./task-board-model";
import "./TaskBoardPage.css";

export type TaskBoardAction = "list" | "create" | "update" | "delete";

export function taskBoardRequest(action: TaskBoardAction, args?: Record<string, unknown>) {
  return invoke<TaskBoard>("task_board_request", { action, args });
}

type TaskPatch = Partial<Pick<Task, "title" | "body" | "status">> & { project?: string };

const ago = (ms: number) => {
  const { text, args } = relativeTime(ms, Date.now());
  return args.length ? tf(text, args) : t(text);
};

const fullTime = (ms: number) => new Date(ms).toLocaleString();

/** 看板上每列最多先画这么多张卡（「完成」会越积越多），其余点「显示全部」。 */
const COLUMN_LIMIT = 30;

export function TaskBoardPage() {
  const [board, setBoard] = useState<TaskBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [view, setView] = useState<"board" | "list">(() => (localStorage.getItem("codex-plus-task-board-view") === "list" ? "list" : "board"));
  const [showCancelled, setShowCancelled] = useState(false);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [focusNew, setFocusNew] = useState(0);

  const run = useCallback(async (action: TaskBoardAction, args?: Record<string, unknown>) => {
    try {
      const next = await taskBoardRequest(action, args);
      setBoard(next);
      setError(null);
      return next;
    } catch (cause) {
      setError(String(cause));
      return null;
    }
  }, []);

  useEffect(() => {
    void run("list");
    const reload = () => void run("list");
    window.addEventListener("focus", reload);
    return () => window.removeEventListener("focus", reload);
  }, [run]);

  useEffect(() => { localStorage.setItem("codex-plus-task-board-view", view); }, [view]);

  const tasks = board?.tasks ?? [];
  const visible = useMemo(() => tasks.filter((task) => matchesQuery(task, query)), [tasks, query]);
  const groups = useMemo(() => groupByStatus(visible), [visible]);
  const columns = TASK_STATUSES.filter((entry) => showCancelled || entry.id !== "cancelled");
  const selected = tasks.find((task) => task.id === selectedId) ?? null;
  const summary = boardSummary(tasks);

  const create = async (title: string, status: TaskStatus) => {
    const next = await run("create", { task: { title, status } });
    return next?.created ?? null;
  };
  const update = (id: number, patch: TaskPatch) => {
    // 先在本地改掉，拖卡片时不用等文件写完才挪过去。
    setBoard((current) => current && { ...current, tasks: current.tasks.map((task) => (task.id === id ? { ...task, ...patch, project: patch.project ?? task.project } : task)) });
    void run("update", { id, patch });
  };
  const remove = async (task: Task) => {
    if (!(await appConfirm(tf("删除 {0}「{1}」？任务文件会被删掉，编号不会再用。不想做了可以改成「已取消」，记录会保留。", [taskKey(task.id), task.title]), { danger: true }))) return;
    if (await run("delete", { id: task.id })) setSelectedId(null);
  };

  useEffect(() => {
    if (selectedId === null) return;
    // Esc：正在详情里打字时先离开输入框（失焦即保存），再按一次才关详情；下拉菜单自己处理 Esc。
    const close = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (target?.closest(".app-select, .app-select-menu, .task-quick-add")) return;
      if (target?.closest(".task-detail") && (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement)) {
        target.blur();
        return;
      }
      setSelectedId(null);
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [selectedId]);

  return (
    <div className="task-board-page">
      <PageHeader
        actions={
          <button className="mg-button mg-primary" onClick={() => { setView("board"); setFocusNew((n) => n + 1); }} type="button">
            <Plus size={14} />{t("新建任务")}
          </button>
        }
      />
      <CxToolbar onQuery={setQuery} placeholder={t("搜索编号、标题、内容、项目")} query={query}>
        <label className="task-board-check">
          <input checked={showCancelled} onChange={(event) => setShowCancelled(event.currentTarget.checked)} type="checkbox" />
          {t("显示已取消")}
        </label>
        <div aria-label={t("视图")} className="mg-segmented" role="tablist">
          <button aria-selected={view === "board"} data-active={view === "board"} onClick={() => setView("board")} role="tab" title={t("看板")} type="button"><Columns3 size={14} />{t("看板")}</button>
          <button aria-selected={view === "list"} data-active={view === "list"} onClick={() => setView("list")} role="tab" title={t("列表")} type="button"><List size={14} />{t("列表")}</button>
        </div>
      </CxToolbar>
      {error ? <div role="alert"><CxBanner icon={<AlertTriangle />} title={error} tone="warn" /></div> : null}
      <div className="task-board-body">
        <div className="task-board-main">
          {view === "board" ? (
            <div className="task-board-columns">
              {columns.map((column) => (
                <BoardColumn
                  focusNew={column.id === "backlog" ? focusNew : 0}
                  key={column.id}
                  onCreate={column.id === "backlog" || column.id === "todo" ? (title) => create(title, column.id) : undefined}
                  onDropTask={(id) => { const task = tasks.find((item) => item.id === id); if (task && task.status !== column.id) update(id, { status: column.id }); }}
                  onSelect={setSelectedId}
                  selectedId={selectedId}
                  status={column.id}
                  tasks={groups[column.id]}
                />
              ))}
            </div>
          ) : (
            <TaskList onSelect={setSelectedId} selectedId={selectedId} statuses={columns.map((column) => column.id)} groups={groups} />
          )}
          {board && !tasks.length && view === "list" ? <p className="task-board-empty">{t("还没有任务。切到看板，在「收集」列顶上写一句就能记下。")}</p> : null}
        </div>
        {selected ? (
          <TaskDetail
            key={selected.id}
            onClose={() => setSelectedId(null)}
            onDelete={() => void remove(selected)}
            onUpdate={(patch) => update(selected.id, patch)}
            projects={[...new Set(tasks.map((task) => task.project).filter((value): value is string => !!value))]}
            task={selected}
          />
        ) : null}
      </div>
      <footer className="task-board-status">
        <span>{tf("未了结 {0}", [summary.open])}</span>
        <span>{tf("进行中 {0}", [summary.inProgress])}</span>
        <span data-tone={summary.inReview ? "warn" : undefined}>{tf("待验收 {0}", [summary.inReview])}</span>
        <span data-tone={summary.blocked ? "danger" : undefined}>{tf("卡住 {0}", [summary.blocked])}</span>
        {board ? <span className="task-board-dir" title={board.dir}>{tf("保存在 {0}", [board.dir])}</span> : null}
      </footer>
    </div>
  );
}

function BoardColumn({ status, tasks, selectedId, onSelect, onDropTask, onCreate, focusNew }: {
  status: TaskStatus;
  tasks: Task[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  onDropTask: (id: number) => void;
  onCreate?: (title: string) => Promise<number | null>;
  focusNew: number;
}) {
  const [over, setOver] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const meta = TASK_STATUSES.find((entry) => entry.id === status)!;
  const shown = expanded ? tasks : tasks.slice(0, COLUMN_LIMIT);
  const drop = (event: DragEvent) => {
    event.preventDefault();
    setOver(false);
    const id = Number(event.dataTransfer.getData("application/x-codex-plus-task"));
    if (id) onDropTask(id);
  };
  return (
    <section
      aria-label={t(meta.label)}
      className="task-board-column"
      data-over={over}
      data-status={status}
      onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node)) setOver(false); }}
      onDragOver={(event) => { if (event.dataTransfer.types.includes("application/x-codex-plus-task")) { event.preventDefault(); setOver(true); } }}
      onDrop={drop}
    >
      <header title={t(meta.hint)}>
        <span className="task-status-dot" data-status={status} />
        <h2>{t(meta.label)}</h2>
        <span className="task-board-count">{tasks.length}</span>
      </header>
      <div className="task-board-cards">
        {onCreate ? <QuickAdd focusSignal={focusNew} onCreate={onCreate} onCreated={onSelect} placeholder={status === "backlog" ? t("记一件事，回车保存") : t("加一件要做的事")} /> : null}
        {shown.map((task) => (
          <button
            className="task-card"
            data-selected={task.id === selectedId}
            draggable
            key={task.id}
            onClick={() => onSelect(task.id)}
            onDragStart={(event) => { event.dataTransfer.setData("application/x-codex-plus-task", String(task.id)); event.dataTransfer.effectAllowed = "move"; }}
            type="button"
          >
            <span className="task-card-title">{task.title}</span>
            <span className="task-card-meta">
              <code>{taskKey(task.id)}</code>
              {task.project ? <span title={task.project}>{projectName(task.project)}</span> : null}
              <span title={`${t("进入这一列")}：${fullTime(task.statusAt)}`}>{ago(task.statusAt)}</span>
            </span>
          </button>
        ))}
        {tasks.length > shown.length ? (
          <button className="task-board-more" onClick={() => setExpanded(true)} type="button">{tf("显示全部 {0} 个", [tasks.length])}</button>
        ) : null}
        {!tasks.length && !onCreate ? <p className="task-board-column-empty">{t(meta.hint)}</p> : null}
      </div>
    </section>
  );
}

function QuickAdd({ placeholder, onCreate, onCreated, focusSignal }: {
  placeholder: string;
  onCreate: (title: string) => Promise<number | null>;
  onCreated: (id: number) => void;
  focusSignal: number;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { if (focusSignal) input.current?.focus(); }, [focusSignal]);
  const submit = async (open: boolean) => {
    const title = value.trim();
    if (!title || busy) return;
    setBusy(true);
    const id = await onCreate(title);
    setBusy(false);
    if (id === null) return;
    setValue("");
    if (open) onCreated(id);
  };
  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" && !event.nativeEvent.isComposing) {
      event.preventDefault();
      // ⌘/Ctrl + 回车：建好后打开详情接着写。
      void submit(event.metaKey || event.ctrlKey);
    } else if (event.key === "Escape") {
      setValue("");
      event.currentTarget.blur();
    }
  };
  return (
    <div className="task-quick-add">
      <Plus aria-hidden="true" size={13} />
      <input aria-label={placeholder} disabled={busy} onChange={(event) => setValue(event.currentTarget.value)} onKeyDown={onKey} placeholder={placeholder} ref={input} title={t("回车保存；⌘/Ctrl+回车保存并打开详情")} value={value} />
    </div>
  );
}

function TaskList({ groups, statuses, selectedId, onSelect }: {
  groups: Record<TaskStatus, Task[]>;
  statuses: TaskStatus[];
  selectedId: number | null;
  onSelect: (id: number) => void;
}) {
  return (
    <div className="task-list">
      {statuses.filter((status) => groups[status].length).map((status) => (
        <section key={status}>
          <header>
            <span className="task-status-dot" data-status={status} />
            <h2>{t(statusLabel(status))}</h2>
            <span className="task-board-count">{groups[status].length}</span>
          </header>
          {groups[status].map((task) => (
            <button className="task-list-row" data-selected={task.id === selectedId} key={task.id} onClick={() => onSelect(task.id)} type="button">
              <code>{taskKey(task.id)}</code>
              <span className="task-list-title">{task.title}</span>
              <span className="task-list-project" title={task.project ?? ""}>{projectName(task.project)}</span>
              <span className="task-list-time" title={fullTime(task.updatedAt)}>{ago(task.updatedAt)}</span>
            </button>
          ))}
        </section>
      ))}
    </div>
  );
}

/** 右侧详情：标题、状态、项目、正文。文字类字段失焦时保存，状态改了立刻保存。 */
function TaskDetail({ task, projects, onUpdate, onDelete, onClose }: {
  task: Task;
  projects: string[];
  onUpdate: (patch: TaskPatch) => void;
  onDelete: () => void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState(task.title);
  const [project, setProject] = useState(task.project ?? "");
  const [body, setBody] = useState(task.body);
  // 外部改了文件（焦点回来时重读）而本地没在改时，跟上新内容。
  const editing = useRef<string | null>(null);
  useEffect(() => { if (editing.current !== "title") setTitle(task.title); }, [task.title]);
  useEffect(() => { if (editing.current !== "project") setProject(task.project ?? ""); }, [task.project]);
  useEffect(() => { if (editing.current !== "body") setBody(task.body); }, [task.body]);

  const commitTitle = () => {
    editing.current = null;
    const next = title.trim();
    if (!next) setTitle(task.title);
    else if (next !== task.title) onUpdate({ title: next });
  };
  const commitProject = () => {
    editing.current = null;
    if (project.trim() !== (task.project ?? "")) onUpdate({ project: project.trim() });
  };
  const commitBody = () => {
    editing.current = null;
    if (body.trimEnd() !== task.body) onUpdate({ body });
  };

  return (
    <aside aria-label={taskKey(task.id)} className="task-detail">
      <header>
        <code>{taskKey(task.id)}</code>
        <span className="task-detail-spacer" />
        <button aria-label={t("删除任务")} className="mg-icon-button" onClick={onDelete} title={t("删除任务")} type="button"><Trash2 size={14} /></button>
        <button aria-label={t("关闭（Esc）")} className="mg-icon-button" onClick={onClose} title={t("关闭（Esc）")} type="button"><X size={14} /></button>
      </header>
      <div className="task-detail-body">
        <input
          aria-label={t("标题")}
          className="task-detail-title"
          onBlur={commitTitle}
          onChange={(event) => setTitle(event.currentTarget.value)}
          onFocus={() => { editing.current = "title"; }}
          onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) event.currentTarget.blur(); }}
          value={title}
        />
        <dl className="task-detail-fields">
          <dt>{t("状态")}</dt>
          <dd>
            <AppSelect<TaskStatus>
              ariaLabel={t("状态")}
              menuWidth={220}
              onChange={(status) => onUpdate({ status })}
              options={TASK_STATUSES.map((entry) => ({ value: entry.id, label: <span className="task-status-option"><span className="task-status-dot" data-status={entry.id} />{t(entry.label)}</span>, hint: t(entry.hint) }))}
              size="sm"
              value={task.status}
            />
            <span className="task-detail-since" title={fullTime(task.statusAt)}>{tf("{0}进入", [ago(task.statusAt)])}</span>
          </dd>
          <dt>{t("项目")}</dt>
          <dd>
            <input
              aria-label={t("项目")}
              list="task-board-projects"
              onBlur={commitProject}
              onChange={(event) => setProject(event.currentTarget.value)}
              onFocus={() => { editing.current = "project"; }}
              onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) event.currentTarget.blur(); }}
              placeholder={t("项目目录，可不填")}
              value={project}
            />
            <datalist id="task-board-projects">{projects.map((value) => <option key={value} value={value} />)}</datalist>
          </dd>
        </dl>
        <textarea
          aria-label={t("内容")}
          className="task-detail-text"
          onBlur={commitBody}
          onChange={(event) => setBody(event.currentTarget.value)}
          onFocus={() => { editing.current = "body"; }}
          placeholder={t("要做什么、为什么、做到什么程度算完（Markdown）")}
          value={body}
        />
        <p className="task-detail-times">
          <span title={fullTime(task.createdAt)}>{tf("创建于 {0}", [ago(task.createdAt)])}</span>
          <span title={fullTime(task.updatedAt)}>{tf("更新于 {0}", [ago(task.updatedAt)])}</span>
        </p>
      </div>
    </aside>
  );
}
