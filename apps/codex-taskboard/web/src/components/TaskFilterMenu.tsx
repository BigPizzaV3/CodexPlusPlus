import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  EMPTY_TASK_FILTERS,
  matchesTaskFilters,
  matchesTaskSearch,
  taskFilterCount,
  type TaskFilterKey,
  type TaskFilters,
} from "../taskFilters";
import {
  TASK_PRIORITIES,
  TASK_STATUSES,
  type Task,
  type TaskPriority,
  type TaskStatus,
} from "../types";
import { STATUS_DETAILS } from "./BoardColumn";
import { LinearIcon, LinearPriorityIcon, LinearStatusIcon } from "./LinearIcon";

type SubmenuName = TaskFilterKey;

interface TaskFilterMenuProps {
  tasks: Task[];
  search: string;
  labels: string[];
  filters: TaskFilters;
  onChange: (filters: TaskFilters) => void;
}

interface FilterOption {
  id: string;
  label: string;
  category: string;
  keywords?: string;
  count: number;
  selected: boolean;
  icon: ReactNode;
  toggle: () => void;
}

const PRIORITY_LABELS: Record<TaskPriority, string> = {
  none: "无优先级",
  urgent: "紧急",
  high: "高",
  medium: "中",
  low: "低",
};

const LINK_LABELS = {
  linked: "Codex 已处理",
  unlinked: "尚未由 Codex 处理",
} as const;

function labelColor(label: string): string {
  let hash = 0;
  for (const character of label) hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  return `hsl(${hash % 360} 54% 55%)`;
}

function LabelGlyph({ label }: { label: string }) {
  return <span className="filter-label-glyph" style={{ "--label-color": labelColor(label) } as CSSProperties} aria-hidden="true" />;
}

function joinSummary(values: string[], noun: string): string | null {
  if (!values.length) return null;
  if (values.length <= 2) return values.join("、");
  return `${values.length} 个${noun}`;
}

export function TaskFilterMenu({ tasks, search, labels, filters, onChange }: TaskFilterMenuProps) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const submenuRef = useRef<HTMLDivElement>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const [open, setOpen] = useState(false);
  const [submenu, setSubmenu] = useState<SubmenuName | null>(null);
  const [rootQuery, setRootQuery] = useState("");
  const [submenuQuery, setSubmenuQuery] = useState("");
  const [contentDraft, setContentDraft] = useState("");
  const [position, setPosition] = useState({ left: 0, top: 0, ready: false });
  const [submenuSide, setSubmenuSide] = useState<"left" | "right" | "overlay">("left");
  const [submenuShift, setSubmenuShift] = useState(0);
  const activeCount = taskFilterCount(filters);

  function closeMenu() {
    if (hoverTimerRef.current !== null) window.clearTimeout(hoverTimerRef.current);
    setOpen(false);
    setSubmenu(null);
    setRootQuery("");
    setSubmenuQuery("");
  }

  function openMenu() {
    setRootQuery("");
    setSubmenu(null);
    setPosition((current) => ({ ...current, ready: false }));
    setOpen(true);
  }

  function openSubmenu(name: SubmenuName, focus = false) {
    if (hoverTimerRef.current !== null) window.clearTimeout(hoverTimerRef.current);
    setSubmenu(name);
    setSubmenuQuery("");
    if (name === "content") setContentDraft(filters.content);
    if (focus) {
      requestAnimationFrame(() => submenuRef.current?.querySelector<HTMLInputElement>("input")?.focus());
    }
  }

  function scheduleSubmenu(name: SubmenuName) {
    if (hoverTimerRef.current !== null) window.clearTimeout(hoverTimerRef.current);
    hoverTimerRef.current = window.setTimeout(() => openSubmenu(name), 160);
  }

  function countFor(key: TaskFilterKey, predicate: (task: Task) => boolean): number {
    return tasks.filter(
      (task) => matchesTaskSearch(task, search) && matchesTaskFilters(task, filters, key) && predicate(task),
    ).length;
  }

  function toggleStatus(status: TaskStatus) {
    const selected = new Set(filters.statuses);
    if (selected.has(status)) selected.delete(status);
    else selected.add(status);
    onChange({ ...filters, statuses: TASK_STATUSES.filter((value) => selected.has(value)) });
  }

  function togglePriority(priority: TaskPriority) {
    const selected = new Set(filters.priorities);
    if (selected.has(priority)) selected.delete(priority);
    else selected.add(priority);
    onChange({ ...filters, priorities: TASK_PRIORITIES.filter((value) => selected.has(value)) });
  }

  function toggleLabel(label: string) {
    const selected = new Set(filters.labels);
    if (selected.has(label)) selected.delete(label);
    else selected.add(label);
    onChange({ ...filters, labels: labels.filter((value) => selected.has(value)) });
  }

  function toggleLink(link: "linked" | "unlinked") {
    onChange({ ...filters, link: filters.link === link ? "all" : link });
  }

  const statusOptions = useMemo<FilterOption[]>(() => TASK_STATUSES.map((status) => ({
    id: `status-${status}`,
    label: STATUS_DETAILS[status].label,
    category: "状态",
    keywords: status,
    count: countFor("statuses", (task) => task.status === status),
    selected: filters.statuses.includes(status),
    icon: <span className={`filter-status-icon status-${status}`}><LinearStatusIcon status={status} /></span>,
    toggle: () => toggleStatus(status),
  })), [filters, search, tasks]);

  const priorityOptions = useMemo<FilterOption[]>(() => TASK_PRIORITIES.map((priority) => ({
    id: `priority-${priority}`,
    label: PRIORITY_LABELS[priority],
    category: "优先级",
    keywords: priority,
    count: countFor("priorities", (task) => task.priority === priority),
    selected: filters.priorities.includes(priority),
    icon: <LinearPriorityIcon priority={priority} />,
    toggle: () => togglePriority(priority),
  })), [filters, search, tasks]);

  const labelOptions = useMemo<FilterOption[]>(() => labels.map((label) => ({
    id: `label-${label}`,
    label,
    category: "标签",
    count: countFor("labels", (task) => task.labels.includes(label)),
    selected: filters.labels.includes(label),
    icon: <LabelGlyph label={label} />,
    toggle: () => toggleLabel(label),
  })), [filters, labels, search, tasks]);

  const linkOptions = useMemo<FilterOption[]>(() => ([
    {
      id: "link-linked",
      label: LINK_LABELS.linked,
      category: "Codex 对话",
      keywords: "codex thread task 已处理",
      count: countFor("link", (task) => Boolean(task.threadId)),
      selected: filters.link === "linked",
      icon: <LinearIcon name="link" />,
      toggle: () => toggleLink("linked"),
    },
    {
      id: "link-unlinked",
      label: LINK_LABELS.unlinked,
      category: "Codex 对话",
      keywords: "codex thread task 未处理",
      count: countFor("link", (task) => !task.threadId),
      selected: filters.link === "unlinked",
      icon: <LinearIcon name="linkOff" />,
      toggle: () => toggleLink("unlinked"),
    },
  ]), [filters, search, tasks]);

  const optionsBySubmenu: Partial<Record<SubmenuName, FilterOption[]>> = {
    statuses: statusOptions,
    priorities: priorityOptions,
    labels: labelOptions,
    link: linkOptions,
  };

  const categories = [
    {
      id: "statuses" as const,
      label: "状态",
      keywords: "status workflow",
      icon: <LinearIcon name="status" />,
      summary: joinSummary(filters.statuses.map((status) => STATUS_DETAILS[status].label), "状态"),
    },
    {
      id: "priorities" as const,
      label: "优先级",
      keywords: "priority urgent high medium low",
      icon: <LinearIcon name="priority" />,
      summary: joinSummary(filters.priorities.map((priority) => PRIORITY_LABELS[priority]), "优先级"),
    },
    {
      id: "labels" as const,
      label: "标签",
      keywords: "label tag",
      icon: <LinearIcon name="label" />,
      summary: joinSummary(filters.labels, "标签"),
    },
    {
      id: "link" as const,
      label: "Codex 关联",
      keywords: "link linked thread task codex 关联",
      icon: <LinearIcon name="link" />,
      summary: filters.link === "all" ? null : LINK_LABELS[filters.link],
    },
    {
      id: "content" as const,
      label: "内容",
      keywords: "content title description text 内容 标题 描述",
      icon: <LinearIcon name="write" />,
      summary: filters.content.trim() || null,
    },
  ];

  const normalizedRootQuery = rootQuery.trim().toLowerCase();
  const visibleCategories = categories.filter((category) => {
    if (!normalizedRootQuery) return true;
    return `${category.label} ${category.keywords}`.toLowerCase().includes(normalizedRootQuery);
  });
  const quickOptions = normalizedRootQuery
    ? [...statusOptions, ...priorityOptions, ...labelOptions, ...linkOptions].filter((option) =>
      `${option.label} ${option.keywords ?? ""}`.toLowerCase().includes(normalizedRootQuery),
    )
    : [];

  useEffect(() => {
    function handleShortcut(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (target?.matches("input, textarea, select, [contenteditable='true']")) return;
      if (event.key.toLowerCase() === "f" && !event.metaKey && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        open ? closeMenu() : openMenu();
      }
    }

    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [open]);

  useLayoutEffect(() => {
    if (!open || !triggerRef.current || !menuRef.current) return;
    const triggerRect = triggerRef.current.getBoundingClientRect();
    const menuRect = menuRef.current.getBoundingClientRect();
    const left = Math.max(8, Math.min(triggerRect.right - menuRect.width, window.innerWidth - menuRect.width - 8));
    const below = triggerRect.bottom + 6;
    const top = below + menuRect.height <= window.innerHeight - 8
      ? below
      : Math.max(8, triggerRect.top - menuRect.height - 6);
    const rightSpace = window.innerWidth - (left + menuRect.width);
    setPosition({ left, top, ready: true });
    setSubmenuSide(
      rightSpace >= menuRect.width + 4
        ? "right"
        : left >= menuRect.width + 12
          ? "left"
          : "overlay",
    );
  }, [activeCount, open, normalizedRootQuery]);

  useLayoutEffect(() => {
    if (!submenu || !submenuRef.current) {
      setSubmenuShift(0);
      return;
    }
    const rect = submenuRef.current.getBoundingClientRect();
    const shift = rect.bottom > window.innerHeight - 8
      ? window.innerHeight - 8 - rect.bottom
      : rect.top < 8
        ? 8 - rect.top
        : 0;
    setSubmenuShift(shift);
  }, [filters, submenu, submenuQuery]);

  useEffect(() => {
    if (!open) return;
    requestAnimationFrame(() => menuRef.current?.querySelector<HTMLInputElement>(".task-filter-search input")?.focus());

    function closeFromOutside(event: PointerEvent) {
      if (!menuRef.current?.contains(event.target as Node) && !triggerRef.current?.contains(event.target as Node)) {
        closeMenu();
      }
    }
    function closeFromViewportChange(event: Event) {
      if (event.type === "scroll" && menuRef.current?.contains(event.target as Node)) return;
      closeMenu();
    }

    document.addEventListener("pointerdown", closeFromOutside);
    window.addEventListener("blur", closeFromViewportChange);
    window.addEventListener("resize", closeFromViewportChange);
    window.addEventListener("scroll", closeFromViewportChange, true);
    return () => {
      document.removeEventListener("pointerdown", closeFromOutside);
      window.removeEventListener("blur", closeFromViewportChange);
      window.removeEventListener("resize", closeFromViewportChange);
      window.removeEventListener("scroll", closeFromViewportChange, true);
    };
  }, [open]);

  useEffect(() => () => {
    if (hoverTimerRef.current !== null) window.clearTimeout(hoverTimerRef.current);
  }, []);

  function focusSibling(button: HTMLButtonElement, direction: 1 | -1) {
    const level = button.dataset.filterLevel;
    const buttons = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>(
      `button[data-filter-level="${level}"]:not(:disabled)`,
    ) ?? []);
    const index = buttons.indexOf(button);
    buttons[(Math.max(0, index) + direction + buttons.length) % buttons.length]?.focus();
  }

  function handleMenuKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const target = event.target as HTMLElement;
    if (event.key === "Escape") {
      event.preventDefault();
      if (submenu) {
        const owner = submenu;
        setSubmenu(null);
        requestAnimationFrame(() => menuRef.current?.querySelector<HTMLButtonElement>(`[data-submenu-name="${owner}"]`)?.focus());
      } else {
        closeMenu();
        triggerRef.current?.focus();
      }
      return;
    }

    if (event.key === "Tab") {
      const focusable = Array.from(menuRef.current?.querySelectorAll<HTMLElement>("input:not(:disabled), button:not(:disabled)") ?? [])
        .filter((element) => element.getClientRects().length > 0);
      const index = focusable.indexOf(target);
      const direction = event.shiftKey ? -1 : 1;
      if (focusable.length) {
        event.preventDefault();
        focusable[(Math.max(0, index) + direction + focusable.length) % focusable.length]?.focus();
      }
      return;
    }

    if (target instanceof HTMLInputElement && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      const level = target.dataset.filterLevel;
      const buttons = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>(
        `button[data-filter-level="${level}"]:not(:disabled)`,
      ) ?? []);
      if (buttons.length) {
        event.preventDefault();
        (event.key === "ArrowDown" ? buttons[0] : buttons.at(-1))?.focus();
      }
      return;
    }

    const button = target.closest<HTMLButtonElement>("button[data-filter-level]");
    if (!button) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      focusSibling(button, event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "ArrowRight" && button.dataset.submenuName) {
      event.preventDefault();
      openSubmenu(button.dataset.submenuName as SubmenuName, true);
    } else if (event.key === "ArrowLeft" && button.dataset.filterLevel === "submenu") {
      event.preventDefault();
      const owner = submenu;
      setSubmenu(null);
      requestAnimationFrame(() => menuRef.current?.querySelector<HTMLButtonElement>(`[data-submenu-name="${owner}"]`)?.focus());
    }
  }

  function renderOption(option: FilterOption, level: "root" | "submenu") {
    return (
      <button
        key={option.id}
        type="button"
        role="menuitemcheckbox"
        aria-checked={option.selected}
        className={`task-filter-item filter-value-item${option.selected ? " is-selected" : ""}`}
        data-filter-level={level}
        onPointerEnter={() => {
          if (level === "root") setSubmenu(null);
        }}
        onClick={option.toggle}
      >
        <span className="task-filter-item-icon">{option.icon}</span>
        <span className="task-filter-item-label">{option.label}</span>
        {level === "root" && <span className="task-filter-item-category">{option.category}</span>}
        <span className="task-filter-item-count">{option.count}</span>
        <span className="task-filter-item-check">{option.selected && <LinearIcon name="check" />}</span>
      </button>
    );
  }

  function renderValueSubmenu(name: Exclude<SubmenuName, "content">) {
    const options = optionsBySubmenu[name] ?? [];
    const needle = submenuQuery.trim().toLowerCase();
    const visible = options.filter((option) => `${option.label} ${option.keywords ?? ""}`.toLowerCase().includes(needle));
    const matching = visible.filter((option) => option.count > 0 || option.selected);
    const unmatched = visible.filter((option) => option.count === 0 && !option.selected);

    return (
      <>
        <label className="task-filter-search submenu-filter-search">
          <span className="sr-only">筛选值</span>
          <input
            data-filter-level="submenu"
            value={submenuQuery}
            onChange={(event) => setSubmenuQuery(event.target.value)}
            placeholder="筛选…"
          />
        </label>
        <div className="task-filter-scroll" role="menu">
          {matching.map((option) => renderOption(option, "submenu"))}
          {unmatched.length > 0 && (
            <>
              <div className="task-filter-section-label">当前视图中无匹配</div>
              <div className="task-filter-unmatched">{unmatched.map((option) => renderOption(option, "submenu"))}</div>
            </>
          )}
          {visible.length === 0 && <div className="task-filter-no-results">没有匹配的筛选值</div>}
        </div>
      </>
    );
  }

  function renderContentSubmenu() {
    const value = contentDraft.trim();
    function applyContent() {
      if (!value) return;
      onChange({ ...filters, content: value });
      closeMenu();
    }

    return (
      <>
        <label className="task-filter-search submenu-filter-search">
          <span className="sr-only">按内容筛选</span>
          <input
            data-filter-level="submenu"
            value={contentDraft}
            onChange={(event) => setContentDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && value) {
                event.preventDefault();
                applyContent();
              }
            }}
            placeholder="输入标题或描述…"
          />
        </label>
        <div className="task-filter-scroll" role="menu">
          <button
            type="button"
            className="task-filter-item filter-content-apply"
            data-filter-level="submenu"
            disabled={!value}
            onClick={applyContent}
          >
            <span className="task-filter-item-icon"><LinearIcon name="write" /></span>
            <span className="task-filter-item-label">{value ? `包含“${value}”` : "输入内容以创建筛选"}</span>
            <kbd>↵</kbd>
          </button>
          {filters.content && (
            <button
              type="button"
              className="task-filter-item filter-content-clear"
              data-filter-level="submenu"
              onClick={() => {
                onChange({ ...filters, content: "" });
                setContentDraft("");
              }}
            >
              <span className="task-filter-item-icon"><LinearIcon name="close" /></span>
              <span className="task-filter-item-label">移除内容筛选</span>
            </button>
          )}
        </div>
      </>
    );
  }

  const menu = open ? createPortal(
    <div
      ref={menuRef}
      className="task-filter-menu"
      data-has-submenu={submenu ? "true" : undefined}
      data-submenu-side={submenuSide}
      style={{ left: position.left, top: position.top, visibility: position.ready ? "visible" : "hidden" }}
      onKeyDown={handleMenuKeyDown}
    >
      <label className="task-filter-search root-filter-search">
        <span className="sr-only">添加筛选</span>
        <input
          data-filter-level="root"
          value={rootQuery}
          onChange={(event) => {
            setRootQuery(event.target.value);
            setSubmenu(null);
          }}
          placeholder="添加筛选…"
        />
        <kbd>F</kbd>
      </label>

      <div className="task-filter-root-list" role="menu">
        {visibleCategories.map((category) => (
          <div className="task-filter-anchor" key={category.id}>
            <button
              type="button"
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={submenu === category.id}
              className={`task-filter-item task-filter-category${submenu === category.id ? " is-open" : ""}${category.summary ? " is-active" : ""}`}
              data-filter-level="root"
              data-submenu-name={category.id}
              onPointerEnter={() => scheduleSubmenu(category.id)}
              onClick={() => submenu === category.id ? setSubmenu(null) : openSubmenu(category.id, true)}
            >
              <span className="task-filter-item-icon">{category.icon}</span>
              <span className="task-filter-item-label">{category.label}</span>
              {category.summary && <span className="task-filter-item-summary">{category.summary}</span>}
              <span className="task-filter-chevron"><LinearIcon name="chevronRight" /></span>
            </button>

            {submenu === category.id && (
              <div
                ref={submenuRef}
                className="task-filter-submenu"
                role="presentation"
                style={{ "--submenu-shift": `${submenuShift}px` } as CSSProperties}
              >
                {category.id === "content"
                  ? renderContentSubmenu()
                  : renderValueSubmenu(category.id)}
              </div>
            )}
          </div>
        ))}

        {quickOptions.length > 0 && (
          <>
            {visibleCategories.length > 0 && <div className="task-filter-section-label">筛选值</div>}
            {quickOptions.map((option) => renderOption(option, "root"))}
          </>
        )}

        {visibleCategories.length === 0 && quickOptions.length === 0 && (
          <div className="task-filter-no-results">没有匹配的筛选项</div>
        )}
      </div>

      {activeCount > 0 && (
        <div className="task-filter-footer">
          <button
            type="button"
            className="task-filter-clear-all"
            data-filter-level="root"
            onClick={() => onChange(EMPTY_TASK_FILTERS)}
          >
            清除所有筛选
            <span>{activeCount}</span>
          </button>
        </div>
      )}
    </div>,
    document.body,
  ) : null;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`task-filter-trigger${activeCount ? " is-active" : ""}${open ? " is-open" : ""}`}
        aria-label={activeCount ? `筛选任务，已启用 ${activeCount} 个条件` : "筛选任务"}
        aria-haspopup="menu"
        aria-expanded={open}
        title={activeCount ? `已启用 ${activeCount} 个筛选条件 (F)` : "筛选任务 (F)"}
        onClick={() => open ? closeMenu() : openMenu()}
      >
        <LinearIcon name="filter" className="filter-icon" />
        {activeCount > 0 && <span className="task-filter-active-dot" aria-hidden="true" />}
      </button>
      {menu}
    </>
  );
}
