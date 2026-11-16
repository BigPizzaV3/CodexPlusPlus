  /**
   * Codex 左侧图标栏的「任务面板」：记下要做什么，看清每件事走到了哪一步。
   *
   * 三栏布局（像 macOS 邮件 / 提醒事项）：左边状态侧栏带计数，中间是当前分类的任务列表，右边是详情。
   * 改状态：详情里的状态菜单，或者把列表里的任务拖到侧栏的某个状态上。
   *
   * 分工：
   *   - 存储在 core `task_board`：每个任务一个 Markdown 文件（<状态目录>/tasks/N.md），编号对外写作 T-N。
   *     本页经 bridge 路由 `/task-board` `{ action, ...args }` 调用，每个动作都返回整个面板。
   *   - 路由不在拓展白名单里（能新建、改、删任务文件）。
   *   - 任务文件可能被编辑器或 Agent 直接改：页面开着时定时重读，有变化才重画。
   *   - 「在 Codex 中开始」由 core 打开 `codex://threads/new` 链接：新对话预填 `[T-N] 标题`，用户自己发送。
   *     core 按首条消息里的 `[T-N]` 把对话认回任务（返回里的 runs），并按对话进展推状态：
   *     开始跑 → 进行中，跑完 → 待验收。点对话行跳到那条对话。
   *
   * 页面 DOM 每次打开重建，状态只放在 codexPlusTaskBoardState 里（见 01-registry 的生命周期约定）。
   * 只重画变化的区域（侧栏 / 列表 / 详情 / 状态栏），搜索框和正在编辑的输入框不被打断。
   */
  const codexPlusTaskStatuses = [
    { id: "backlog", label: "收集", hint: "刚记下，还没决定要不要做" },
    { id: "todo", label: "待办", hint: "要做，还没开始" },
    { id: "in_progress", label: "进行中", hint: "正在做" },
    { id: "in_review", label: "待验收", hint: "做完了，等你确认" },
    { id: "blocked", label: "卡住", hint: "暂时没法继续" },
    { id: "done", label: "完成", hint: "已确认完成" },
    { id: "cancelled", label: "已取消", hint: "不做了，记录保留" },
  ];
  /** 侧栏最上面的汇总分类：除完成、已取消以外的全部任务，按状态分组。 */
  const codexPlusTaskOpenFilter = { id: "open", label: "未了结", hint: "除完成和已取消以外的全部任务" };
  /** 列表先画这么多条（「完成」会越积越多），其余点「显示全部」。 */
  const codexPlusTaskListLimit = 200;
  const codexPlusTaskDragType = "application/x-codex-plus-task";
  const codexPlusTaskBoardState = {
    data: null,
    error: "",
    query: "",
    // 侧栏选中的分类：open 或某个状态 id。
    filter: "open",
    selectedId: null,
    // 详情里的状态菜单、项目菜单是否展开。
    statusMenu: false,
    projectMenu: false,
    showAll: false,
    refocusQuickAdd: false,
    // 鼠标按下期间不重画：失焦保存触发的重画会把正要点的按钮换掉，这一下点击就丢了。
    pointerDown: false,
    renderPending: false,
    lastWrite: null,
    revealId: null,
    pollTimer: null,
    onFocus: null,
    onRelease: null,
  };

  /** 页面里用到的小图标（线框，跟随文字颜色）。 */
  const codexPlusTaskIcon = (paths, extra = "") => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"${extra}>${paths}</svg>`;
  const codexPlusTaskIcons = {
    search: codexPlusTaskIcon('<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>'),
    plus: codexPlusTaskIcon('<path d="M12 5v14M5 12h14"/>'),
    tray: codexPlusTaskIcon('<path d="M4 13.5 6.2 5.8A1.5 1.5 0 0 1 7.6 4.8h8.8a1.5 1.5 0 0 1 1.4 1L20 13.5V18a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18z"/><path d="M4 13.5h4.5l1.5 2.5h4l1.5-2.5H20"/>'),
    folder: codexPlusTaskIcon('<path d="M3.5 7.5a2 2 0 0 1 2-2h3.6l2 2.2h7.4a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>'),
    check: codexPlusTaskIcon('<rect x="4" y="4" width="16" height="16" rx="3.5"/><path d="m8.5 12.2 2.4 2.4 4.8-5"/>'),
    play: codexPlusTaskIcon('<path d="M8 5.5v13l10.5-6.5z"/>', ' data-fill="true"'),
    chevron: codexPlusTaskIcon('<path d="m7 10 5 5 5-5"/>'),
    trash: codexPlusTaskIcon('<path d="M4 7h16M9.5 7V4.5h5V7M6.5 7l.9 12.5h9.2l.9-12.5"/>'),
    close: codexPlusTaskIcon('<path d="M6 6l12 12M18 6 6 18"/>'),
    chat: codexPlusTaskIcon('<path d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8.5A1.5 1.5 0 0 1 19 17h-8l-4.5 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5V7A1.5 1.5 0 0 1 5 5.5z"/>'),
    note: codexPlusTaskIcon('<rect x="5" y="3.5" width="14" height="17" rx="2"/><path d="M8.5 8h7M8.5 12h7M8.5 16h4"/>'),
  };

  const codexPlusTaskKey = (id) => `T-${id}`;
  const codexPlusTaskStatusLabel = (status) => codexPlusTaskStatuses.find((entry) => entry.id === status)?.label || status;
  const codexPlusTaskIsOpen = (task) => task.status !== "done" && task.status !== "cancelled";

  function codexPlusTaskProjectName(project) {
    if (!project) return "";
    const parts = String(project).split(/[\\/]+/).filter(Boolean);
    return parts[parts.length - 1] || String(project);
  }

  function codexPlusTaskAgo(ms) {
    const diff = Math.max(0, Date.now() - Number(ms || 0));
    if (diff < 60_000) return "刚刚";
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
    if (diff < 30 * 86_400_000) return `${Math.floor(diff / 86_400_000)} 天前`;
    return new Date(Number(ms)).toLocaleDateString();
  }

  const codexPlusTaskFullTime = (ms) => new Date(Number(ms || 0)).toLocaleString();

  /** 搜索：编号（T-12 / 12）、标题、正文、项目，忽略大小写，空格分开的词都要命中。 */
  function codexPlusTaskMatches(task, query) {
    const words = String(query || "").trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return true;
    const haystack = [codexPlusTaskKey(task.id), String(task.id), task.title, task.body, task.project || ""].join("\n").toLowerCase();
    return words.every((word) => haystack.includes(word));
  }

  /** 列表里的正文摘要：去掉 Markdown 标记，几行拼成一句。 */
  function codexPlusTaskExcerpt(body) {
    return String(body || "")
      .split(/\r?\n/)
      .map((line) => line
        .replace(/^\s*(?:#{1,6}\s+|>\s?|[-*+]\s+(?:\[[ xX]\]\s+)?|\d+[.)]\s+)/, "")
        .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
        .replace(/[*_`~]+/g, "")
        .trim())
      .filter(Boolean)
      .join(" ")
      .slice(0, 200);
  }

  /** 正文里 `- [ ]` / `- [x]` 清单的完成数。 */
  function codexPlusTaskChecklist(body) {
    const items = String(body || "").match(/^[ \t]*[-*+][ \t]+\[[ xX]\]/gm) || [];
    return { done: items.filter((item) => /\[[xX]\]$/.test(item)).length, total: items.length };
  }

  /** 按状态分组；每组里最近改过状态的在前（刚改过来的在最上面）。 */
  function codexPlusTaskGroups(tasks) {
    const groups = Object.fromEntries(codexPlusTaskStatuses.map((entry) => [entry.id, []]));
    for (const task of tasks) (groups[task.status] || groups.backlog).push(task);
    for (const list of Object.values(groups)) list.sort((a, b) => (b.statusAt - a.statusAt) || (b.id - a.id));
    return groups;
  }

  /**
   * 某个分类下、命中搜索的任务，按列表显示顺序排好：`[{ status, tasks }]`，空组不出现。
   * 「未了结」按状态分组（收集 → 卡住）；单个状态只有一组。
   */
  function codexPlusTaskSections(tasks, filter, query) {
    const groups = codexPlusTaskGroups(tasks.filter((task) => codexPlusTaskMatches(task, query)));
    const statuses = filter === "open"
      ? codexPlusTaskStatuses.filter((entry) => entry.id !== "done" && entry.id !== "cancelled")
      : codexPlusTaskStatuses.filter((entry) => entry.id === filter);
    return statuses.map((entry) => ({ status: entry.id, tasks: groups[entry.id] })).filter((section) => section.tasks.length);
  }

  const codexPlusTasks = () => (Array.isArray(codexPlusTaskBoardState.data?.tasks) ? codexPlusTaskBoardState.data.tasks : []);
  const codexPlusTaskFilterEntry = (filter) => (filter === "open" ? codexPlusTaskOpenFilter : codexPlusTaskStatuses.find((entry) => entry.id === filter)) || codexPlusTaskOpenFilter;
  /** 当前列表里按显示顺序排的任务（键盘上下切换用）。 */
  const codexPlusTaskVisible = () => {
    const state = codexPlusTaskBoardState;
    const all = codexPlusTaskSections(codexPlusTasks(), state.filter, state.query).flatMap((section) => section.tasks);
    return state.showAll ? all : all.slice(0, codexPlusTaskListLimit);
  };
  const codexPlusTaskRunStates = {
    running: { label: "运行中", tone: "running" },
    finished: { label: "已跑完", tone: "done" },
    aborted: { label: "已中断", tone: "warn" },
    idle: { label: "未开始", tone: "idle" },
  };

  /** 认回这个任务的 Codex 对话，最近有动静的在前。 */
  function codexPlusTaskRunsFor(id, runs = codexPlusTaskBoardState.data?.runs) {
    const list = Array.isArray(runs) ? runs.filter((run) => run.taskId === id) : [];
    const at = (run) => Math.max(Number(run.finishedAt || 0), Number(run.startedAt || 0), Number(run.updatedAt || 0));
    return list.sort((a, b) => at(b) - at(a));
  }

  /** 打开一条 Codex 对话：先关掉本页，再让 Codex 自己的路由跳过去。 */
  function codexPlusTaskOpenThread(threadId) {
    if (!threadId) return;
    closeCodexPlusPage();
    window.dispatchEvent(new MessageEvent("message", { data: { type: "navigate-to-route", path: `/local/${threadId}` } }));
  }

  async function codexPlusTaskRequest(action, args = {}) {
    const result = await postJson("/task-board", { action, ...args });
    if (result?.status !== "ok") throw new Error(result?.message || "任务面板操作失败");
    return result;
  }

  /** 执行一个动作并刷新；失败时显示原因并重读一次，免得本地乐观改动和文件对不上。 */
  async function codexPlusTaskRun(action, args = {}) {
    const state = codexPlusTaskBoardState;
    try {
      const data = await codexPlusTaskRequest(action, args);
      state.data = data;
      state.error = "";
      return data;
    } catch (error) {
      state.error = error?.message || String(error);
      try { state.data = await codexPlusTaskRequest("list"); } catch {}
      return null;
    } finally {
      renderCodexPlusTaskBoard();
    }
  }

  // ---------- 渲染 ----------

  /** 左侧栏：「未了结」汇总 + 七个状态，各带计数；状态项也是拖放目标。 */
  function renderCodexPlusTaskSidebar() {
    const nav = document.querySelector("[data-codex-task-sidebar]");
    if (!nav) return;
    const state = codexPlusTaskBoardState;
    const tasks = codexPlusTasks();
    const count = (status) => tasks.filter((task) => task.status === status).length;
    const item = (entry, icon, total, drop) => `<button type="button" class="codex-plus-task-nav" data-codex-task-filter="${entry.id}"${drop ? ` data-codex-task-drop="${entry.id}"` : ""} aria-current="${String(state.filter === entry.id)}" title="${escapeHtml(entry.hint)}">
        ${icon}<span class="codex-plus-task-nav-label">${escapeHtml(entry.label)}</span><span class="codex-plus-task-nav-count">${total || ""}</span></button>`;
    nav.innerHTML = `<div class="codex-plus-task-sidebar-title">任务</div>
      ${item(codexPlusTaskOpenFilter, codexPlusTaskIcons.tray, tasks.filter(codexPlusTaskIsOpen).length, false)}
      <div class="codex-plus-task-sidebar-section">状态</div>
      ${codexPlusTaskStatuses.map((entry) => item(entry, `<span class="codex-plus-task-dot" data-status="${entry.id}"></span>`, count(entry.id), true)).join("")}
      <p class="codex-plus-task-sidebar-tip">把任务拖到状态上即可改状态</p>`;
  }

  /** 列表行：第一行标题和时间，第二行编号、项目、清单、对话，有正文时第三行是摘要。 */
  function codexPlusTaskRow(task) {
    const selected = task.id === codexPlusTaskBoardState.selectedId;
    const runs = codexPlusTaskRunsFor(task.id);
    const running = runs.filter((run) => run.state === "running").length;
    const checklist = codexPlusTaskChecklist(task.body);
    const excerpt = codexPlusTaskExcerpt(task.body);
    const meta = [
      `<code>${codexPlusTaskKey(task.id)}</code>`,
      task.project ? `<span class="codex-plus-task-meta-item" title="${escapeHtml(task.project)}">${codexPlusTaskIcons.folder}<span>${escapeHtml(codexPlusTaskProjectName(task.project))}</span></span>` : "",
      checklist.total ? `<span class="codex-plus-task-meta-item" data-complete="${String(checklist.done === checklist.total)}" title="清单完成 ${checklist.done} / ${checklist.total}">${codexPlusTaskIcons.check}<span>${checklist.done}/${checklist.total}</span></span>` : "",
      running
        ? `<span class="codex-plus-task-meta-item" data-tone="running" title="${running} 个 Codex 对话正在跑"><i aria-hidden="true"></i><span>运行中</span></span>`
        : runs.length ? `<span class="codex-plus-task-meta-item" title="${runs.length} 个 Codex 对话">${codexPlusTaskIcons.chat}<span>${runs.length}</span></span>` : "",
    ].filter(Boolean).join("");
    return `<button type="button" class="codex-plus-task-row" draggable="true" data-codex-task-open="${task.id}" data-selected="${String(selected)}" data-status="${escapeHtml(task.status)}">
      <span class="codex-plus-task-dot" data-status="${escapeHtml(task.status)}" title="${escapeHtml(codexPlusTaskStatusLabel(task.status))}"></span>
      <span class="codex-plus-task-row-title">${escapeHtml(task.title)}</span>
      <span class="codex-plus-task-row-time" title="进入「${escapeHtml(codexPlusTaskStatusLabel(task.status))}」：${escapeHtml(codexPlusTaskFullTime(task.statusAt))}">${codexPlusTaskAgo(task.statusAt)}</span>
      <span class="codex-plus-task-row-meta">${meta}</span>
      ${excerpt ? `<span class="codex-plus-task-row-excerpt">${escapeHtml(excerpt)}</span>` : ""}
    </button>`;
  }

  function renderCodexPlusTaskMain() {
    const main = document.querySelector("[data-codex-task-main]");
    if (!main) return;
    const state = codexPlusTaskBoardState;
    const entry = codexPlusTaskFilterEntry(state.filter);
    const heading = document.querySelector("[data-codex-task-heading]");
    if (heading) heading.textContent = entry.label;
    if (!state.data) {
      main.innerHTML = `<div class="codex-plus-task-empty">${state.error ? '<button type="button" class="codex-plus-task-plain-button" data-codex-task-action="reload">重试</button>' : "正在读取…"}</div>`;
      return;
    }
    const sections = codexPlusTaskSections(codexPlusTasks(), state.filter, state.query);
    const total = sections.reduce((sum, section) => sum + section.tasks.length, 0);
    const countNode = document.querySelector("[data-codex-task-list-count]");
    if (countNode) countNode.textContent = total ? `${total} 个${state.query.trim() ? "匹配" : ""}` : "";
    // 快速新建只在能「新建到这里」的分类出现：未了结（进收集）和未了结的各状态。
    const target = state.filter === "open" ? "backlog" : state.filter;
    const canAdd = target !== "done" && target !== "cancelled";
    const quickAdd = canAdd ? `<label class="codex-plus-task-quick-add">${codexPlusTaskIcons.plus}
        <input type="text" data-codex-task-quick-add="${target}" placeholder="新建任务到「${escapeHtml(codexPlusTaskStatusLabel(target))}」，回车保存" title="回车保存；⌘/Ctrl+回车保存并打开详情" aria-label="新建任务"></label>` : "";
    let budget = state.showAll ? Infinity : codexPlusTaskListLimit;
    const body = sections.map((section) => {
      const shown = section.tasks.slice(0, Math.max(0, budget));
      budget -= shown.length;
      if (!shown.length) return "";
      const header = state.filter === "open"
        ? `<header class="codex-plus-task-group-head"><span class="codex-plus-task-dot" data-status="${section.status}"></span><span>${escapeHtml(codexPlusTaskStatusLabel(section.status))}</span><span class="codex-plus-task-count">${section.tasks.length}</span></header>`
        : "";
      return `<section class="codex-plus-task-group">${header}${shown.map(codexPlusTaskRow).join("")}</section>`;
    }).join("");
    const more = !state.showAll && total > codexPlusTaskListLimit
      ? `<button type="button" class="codex-plus-task-plain-button codex-plus-task-more" data-codex-task-action="showAll">显示全部 ${total} 个</button>` : "";
    const empty = state.query.trim()
      ? `<div class="codex-plus-task-empty">${codexPlusTaskIcons.search}<strong>没有匹配的任务</strong><span>换个关键词试试，编号写 T-12 或 12 都行。</span></div>`
      : `<div class="codex-plus-task-empty">${codexPlusTaskIcons.note}<strong>「${escapeHtml(entry.label)}」里没有任务</strong><span>${escapeHtml(entry.hint)}</span></div>`;
    const scrollTop = main.scrollTop;
    // 快速新建框正在打字时（定时重读触发重画），保住已经打的字和光标。
    const active = document.activeElement;
    const typing = active instanceof HTMLInputElement && main.contains(active) && active.matches("[data-codex-task-quick-add]")
      ? { value: active.value, start: active.selectionStart, end: active.selectionEnd }
      : null;
    main.innerHTML = `${quickAdd}${total ? body + more : empty}`;
    main.scrollTop = scrollTop;
    const input = main.querySelector("[data-codex-task-quick-add]");
    if (state.refocusQuickAdd) {
      input?.focus();
      state.refocusQuickAdd = false;
    } else if (typing && input) {
      input.value = typing.value;
      input.focus();
      input.setSelectionRange(typing.start, typing.end);
    }
  }

  function renderCodexPlusTaskDetail() {
    const aside = document.querySelector("[data-codex-task-detail]");
    if (!aside) return;
    const state = codexPlusTaskBoardState;
    const task = codexPlusTasks().find((item) => item.id === state.selectedId);
    if (!task) {
      state.selectedId = null;
      aside.dataset.taskId = "";
      aside.innerHTML = `<div class="codex-plus-task-empty">${codexPlusTaskIcons.note}<strong>没有选中任务</strong><span>在左边列表里点一个任务，或用 ↑ ↓ 切换。</span></div>`;
      return;
    }
    aside.dataset.taskId = String(task.id);
    const runs = codexPlusTaskRunsFor(task.id);
    const current = codexPlusTaskStatuses.find((entry) => entry.id === task.status) || codexPlusTaskStatuses[0];
    const checklist = codexPlusTaskChecklist(task.body);
    aside.innerHTML = `<header class="codex-plus-task-pane-bar">
        <code>${codexPlusTaskKey(task.id)}</code><span class="codex-plus-task-spacer"></span>
        <button type="button" class="codex-plus-task-icon-button" data-codex-task-action="delete" title="删除任务" aria-label="删除任务">${codexPlusTaskIcons.trash}</button>
        <button type="button" class="codex-plus-task-icon-button" data-codex-task-action="closeDetail" title="取消选中（Esc）" aria-label="取消选中（Esc）">${codexPlusTaskIcons.close}</button>
      </header>
      <div class="codex-plus-task-detail-body">
        <textarea class="codex-plus-task-title-input" data-codex-task-field="title" rows="1" aria-label="标题" placeholder="标题">${escapeHtml(task.title)}</textarea>
        <div class="codex-plus-task-props">
          <span class="codex-plus-task-prop-label">状态</span>
          <div class="codex-plus-task-prop-value">
            <button type="button" class="codex-plus-task-select" data-codex-task-action="statusMenu" aria-haspopup="menu" aria-expanded="${String(state.statusMenu)}" title="${escapeHtml(current.hint)}">
              <span class="codex-plus-task-dot" data-status="${current.id}"></span><span>${escapeHtml(current.label)}</span>${codexPlusTaskIcons.chevron}</button>
            <span class="codex-plus-task-prop-note" title="${escapeHtml(codexPlusTaskFullTime(task.statusAt))}">${codexPlusTaskAgo(task.statusAt)}进入</span>
            ${state.statusMenu ? codexPlusTaskStatusMenu(task) : ""}
          </div>
          <span class="codex-plus-task-prop-label">项目</span>
          <div class="codex-plus-task-prop-value">
            <button type="button" class="codex-plus-task-select" data-codex-task-action="projectMenu" aria-haspopup="menu" aria-expanded="${String(state.projectMenu)}" title="${escapeHtml(task.project || "没填项目时，用 Codex 当前选中的项目")}">
              ${codexPlusTaskIcons.folder}<span${task.project ? "" : ' class="codex-plus-task-placeholder"'}>${escapeHtml(task.project ? codexPlusTaskProjectName(task.project) : "不指定")}</span>${codexPlusTaskIcons.chevron}</button>
            ${state.projectMenu ? codexPlusTaskProjectMenu(task) : ""}
          </div>
          <span class="codex-plus-task-prop-label">时间</span>
          <div class="codex-plus-task-prop-value codex-plus-task-prop-note">
            <span title="${escapeHtml(codexPlusTaskFullTime(task.createdAt))}">${codexPlusTaskAgo(task.createdAt)}创建</span><span aria-hidden="true">·</span><span title="${escapeHtml(codexPlusTaskFullTime(task.updatedAt))}">${codexPlusTaskAgo(task.updatedAt)}更新</span>
          </div>
        </div>
        ${codexPlusTaskStartSection(task, runs)}
        <div class="codex-plus-task-section-head"><label for="codex-plus-task-body">描述</label>${checklist.total ? `<span title="正文里的 - [ ] 清单">清单 ${checklist.done}/${checklist.total}</span>` : ""}</div>
        <textarea id="codex-plus-task-body" class="codex-plus-task-body-input" data-codex-task-field="body" placeholder="要做什么、为什么、做到什么程度算完。支持 Markdown，- [ ] 写清单。">${escapeHtml(task.body)}</textarea>
        ${codexPlusTaskRunList(runs)}
      </div>`;
  }

  /** 详情里的状态菜单：每项一个状态，带说明；当前状态高亮。 */
  function codexPlusTaskStatusMenu(task) {
    return `<div class="codex-plus-task-menu" role="menu" aria-label="改状态">${codexPlusTaskStatuses.map((entry) => `<button type="button" role="menuitemradio" aria-checked="${String(entry.id === task.status)}" data-codex-task-status="${entry.id}">
        <span class="codex-plus-task-dot" data-status="${entry.id}"></span><span class="codex-plus-task-menu-label">${escapeHtml(entry.label)}</span><span class="codex-plus-task-menu-hint">${escapeHtml(entry.hint)}</span></button>`).join("")}</div>`;
  }

  /**
   * 项目候选：本任务当前的项目、其它任务用过的项目、core 给的最近在 Codex 里用过的项目（`projects`），去重。
   * 已经选中的排最前；其余按「任务里用过的」在前、最近对话的在后。
   */
  function codexPlusTaskProjectChoices(task, data = codexPlusTaskBoardState.data) {
    const key = (path) => String(path).replace(/[\\/]+$/, "");
    const seen = new Set();
    const list = [];
    const add = (path) => {
      if (!path || typeof path !== "string" || seen.has(key(path))) return;
      seen.add(key(path));
      list.push(path);
    };
    add(task?.project);
    const tasks = Array.isArray(data?.tasks) ? [...data.tasks].sort((a, b) => b.updatedAt - a.updatedAt) : [];
    tasks.forEach((item) => add(item.project));
    (Array.isArray(data?.projects) ? data.projects : []).forEach(add);
    return list;
  }

  /** 详情里的项目菜单：候选项目、「不指定」、「选择其他文件夹…」。 */
  function codexPlusTaskProjectMenu(task) {
    const choices = codexPlusTaskProjectChoices(task);
    const current = task.project ? String(task.project).replace(/[\\/]+$/, "") : "";
    const rows = choices.map((path) => `<button type="button" role="menuitemradio" aria-checked="${String(path.replace(/[\\/]+$/, "") === current)}" data-codex-task-project="${escapeHtml(path)}" title="${escapeHtml(path)}">
        ${codexPlusTaskIcons.folder}<span class="codex-plus-task-menu-label">${escapeHtml(codexPlusTaskProjectName(path))}</span><span class="codex-plus-task-menu-hint">${escapeHtml(path)}</span></button>`).join("");
    return `<div class="codex-plus-task-menu codex-plus-task-project-menu" role="menu" aria-label="选择项目">
        ${rows || '<p class="codex-plus-task-menu-empty">还没有在 Codex 里用过的项目</p>'}
        <div class="codex-plus-task-menu-separator" role="separator"></div>
        <button type="button" role="menuitemradio" aria-checked="${String(!current)}" data-codex-task-project="">${codexPlusTaskIcons.close}<span class="codex-plus-task-menu-label">不指定</span><span class="codex-plus-task-menu-hint">用 Codex 当前选中的项目</span></button>
        <button type="button" role="menuitem" data-codex-task-action="pickProject">${codexPlusTaskIcons.plus}<span class="codex-plus-task-menu-label">选择其他文件夹…</span><span class="codex-plus-task-menu-hint"></span></button>
      </div>`;
  }

  /** 收起状态 / 项目菜单：只摘掉菜单节点，不重画详情（正在编辑的输入框不被打断）。 */
  function codexPlusTaskCloseMenu() {
    codexPlusTaskBoardState.statusMenu = false;
    codexPlusTaskBoardState.projectMenu = false;
    document.querySelectorAll(".codex-plus-task-menu").forEach((node) => node.remove());
    document.querySelectorAll('[data-codex-task-action="statusMenu"], [data-codex-task-action="projectMenu"]').forEach((node) => node.setAttribute("aria-expanded", "false"));
  }

  /** 「在 Codex 中开始」：新开一个对话，输入框里填好 `[T-N] 标题` 和正文，由用户确认后发送。 */
  function codexPlusTaskStartSection(task, runs) {
    const where = task.project
      ? `在「${escapeHtml(codexPlusTaskProjectName(task.project))}」里新开对话`
      : "没填项目，用 Codex 当前选中的项目";
    const label = runs.length ? "再开一个 Codex 对话" : "在 Codex 中开始";
    return `<div class="codex-plus-task-start">
        <button type="button" class="codex-plus-task-primary" data-codex-task-action="start">${codexPlusTaskIcons.play}${label}</button>
        <span class="codex-plus-task-start-hint" title="新对话的输入框里会填好任务内容，你确认后发送。对话开始跑，任务进「进行中」；跑完进「待验收」。">${where}，发送后自动跟进状态</span>
      </div>`;
  }

  /** 认回这个任务的 Codex 对话：点一行跳过去。 */
  function codexPlusTaskRunList(runs) {
    if (!runs.length) return "";
    const rows = runs.map((run) => {
      const state = codexPlusTaskRunStates[run.state] || codexPlusTaskRunStates.idle;
      const at = run.state === "running" ? run.startedAt : (run.finishedAt || run.updatedAt);
      const when = at ? `<span class="codex-plus-task-run-time" title="${escapeHtml(codexPlusTaskFullTime(at))}">${codexPlusTaskAgo(at)}</span>` : "";
      const last = run.lastMessage ? `<span class="codex-plus-task-run-last">${escapeHtml(run.lastMessage)}</span>` : "";
      return `<button type="button" class="codex-plus-task-run" data-codex-task-thread="${escapeHtml(run.threadId)}" title="打开这个对话${run.cwd ? `（${escapeHtml(run.cwd)}）` : ""}">
          <span class="codex-plus-task-run-badge" data-tone="${state.tone}">${state.label}</span>
          <span class="codex-plus-task-run-title">${escapeHtml(run.title || run.threadId)}</span>${when}${last}
        </button>`;
    }).join("");
    return `<div class="codex-plus-task-section-head"><span>Codex 对话</span><span>${runs.length}</span></div><div class="codex-plus-task-runs">${rows}</div>`;
  }

  function renderCodexPlusTaskStatus() {
    const bar = document.querySelector("[data-codex-task-summary]");
    if (!bar) return;
    const tasks = codexPlusTasks();
    const count = (status) => tasks.filter((task) => task.status === status).length;
    const review = count("in_review");
    const blocked = count("blocked");
    const dir = codexPlusTaskBoardState.data?.dir || "";
    bar.innerHTML = `<span>未了结 ${tasks.filter(codexPlusTaskIsOpen).length}</span><span>进行中 ${count("in_progress")}</span>
      <span${review ? ' data-tone="warn"' : ""}>待验收 ${review}</span><span${blocked ? ' data-tone="danger"' : ""}>卡住 ${blocked}</span>
      ${dir ? `<span class="codex-plus-task-dir" title="${escapeHtml(dir)}">保存在 ${escapeHtml(dir)}</span>` : ""}`;
  }

  function renderCodexPlusTaskBanner() {
    const banner = document.querySelector("[data-codex-task-banner]");
    if (!banner) return;
    const error = codexPlusTaskBoardState.error;
    banner.hidden = !error;
    banner.textContent = error;
  }

  /** 详情里有输入框正在编辑时不重画详情，免得打断输入；失焦保存后会再画。 */
  function renderCodexPlusTaskBoard({ detail = true } = {}) {
    if (codexPlusTaskBoardState.pointerDown) {
      codexPlusTaskBoardState.renderPending = true;
      return;
    }
    renderCodexPlusTaskBanner();
    renderCodexPlusTaskSidebar();
    renderCodexPlusTaskMain();
    const aside = document.querySelector("[data-codex-task-detail]");
    const editing = aside?.contains(document.activeElement) && document.activeElement?.matches("input, textarea");
    const switched = aside && aside.dataset.taskId !== String(codexPlusTaskBoardState.selectedId ?? "");
    if (detail && (!editing || switched)) renderCodexPlusTaskDetail();
    renderCodexPlusTaskStatus();
    const reveal = codexPlusTaskBoardState.revealId;
    codexPlusTaskBoardState.revealId = null;
    if (reveal != null) document.querySelector(`[data-codex-task-main] [data-codex-task-open="${reveal}"]`)?.scrollIntoView({ block: "nearest" });
  }

  // ---------- 操作 ----------

  /** 先在本地改掉再写文件：拖任务改状态时不用等写完才挪过去。 */
  function codexPlusTaskUpdate(id, patch) {
    const task = codexPlusTasks().find((item) => item.id === id);
    if (!task) return;
    Object.assign(task, patch);
    if (patch.status) task.statusAt = Date.now();
    renderCodexPlusTaskBoard();
    // 记下最近一次写入：「在 Codex 中开始」要等它写完，新对话里才是刚改的标题和正文。
    codexPlusTaskBoardState.lastWrite = codexPlusTaskRun("update", { id, patch });
  }

  async function codexPlusTaskCreate(input, openAfter) {
    const title = input.value.trim();
    const status = input.getAttribute("data-codex-task-quick-add");
    if (!title || input.disabled) return;
    input.disabled = true;
    const data = await codexPlusTaskRun("create", { task: { title, status } });
    if (!data) {
      // 失败时保留写好的字（重画换掉了输入框，这里补回去）。
      const again = document.querySelector("[data-codex-task-quick-add]");
      if (again) again.value = title;
      return;
    }
    if (openAfter) {
      codexPlusTaskSelect(data.created);
      document.querySelector('[data-codex-task-field="body"]')?.focus();
    } else {
      codexPlusTaskBoardState.refocusQuickAdd = true;
      codexPlusTaskBoardState.revealId = data.created;
      renderCodexPlusTaskBoard();
    }
  }

  function codexPlusTaskCommitField(input) {
    const id = codexPlusTaskBoardState.selectedId;
    const task = codexPlusTasks().find((item) => item.id === id);
    const field = input.getAttribute("data-codex-task-field");
    if (!task || !field) return;
    const value = input.value;
    if (field === "title") {
      const title = value.trim();
      if (!title) { input.value = task.title; return; }
      if (title !== task.title) codexPlusTaskUpdate(id, { title });
    } else if (field === "body") {
      if (value.trimEnd() !== task.body) codexPlusTaskUpdate(id, { body: value });
    }
  }

  function codexPlusTaskSelect(id) {
    codexPlusTaskBoardState.selectedId = id;
    codexPlusTaskBoardState.statusMenu = false;
    codexPlusTaskBoardState.projectMenu = false;
    codexPlusTaskBoardState.revealId = id;
    renderCodexPlusTaskBoard();
  }

  /** 切换侧栏分类。选中的任务不在新分类里时取消选中。 */
  function codexPlusTaskSetFilter(filter) {
    const state = codexPlusTaskBoardState;
    state.filter = filter === "open" || codexPlusTaskStatuses.some((entry) => entry.id === filter) ? filter : "open";
    state.showAll = false;
    try { localStorage.setItem("codex-plus-task-board-filter", state.filter); } catch {}
    const selected = codexPlusTasks().find((task) => task.id === state.selectedId);
    const visible = selected && (state.filter === "open" ? codexPlusTaskIsOpen(selected) : selected.status === state.filter);
    if (!visible) state.selectedId = null;
    state.statusMenu = false;
    state.projectMenu = false;
    document.querySelector("[data-codex-task-main]")?.scrollTo({ top: 0 });
    renderCodexPlusTaskBoard();
  }

  /** ↑ ↓ 在列表里换选中的任务，并把焦点留在那一行上。 */
  function codexPlusTaskStep(delta) {
    const list = codexPlusTaskVisible();
    if (!list.length) return;
    const index = list.findIndex((task) => task.id === codexPlusTaskBoardState.selectedId);
    const next = index < 0 ? (delta > 0 ? list[0] : list[list.length - 1]) : list[Math.min(list.length - 1, Math.max(0, index + delta))];
    if (next.id !== codexPlusTaskBoardState.selectedId) codexPlusTaskSelect(next.id);
    document.querySelector(`[data-codex-task-main] [data-codex-task-open="${next.id}"]`)?.focus({ preventScroll: true });
  }

  async function codexPlusTaskHandleClick(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const state = codexPlusTaskBoardState;
    // 菜单开着时，点菜单和触发按钮以外的地方先收起菜单，这一下点击照常生效。
    if ((state.statusMenu || state.projectMenu) && !target?.closest('.codex-plus-task-menu, [data-codex-task-action="statusMenu"], [data-codex-task-action="projectMenu"]')) codexPlusTaskCloseMenu();
    const open = target?.closest("[data-codex-task-open]");
    if (open) {
      codexPlusTaskSelect(Number(open.getAttribute("data-codex-task-open")));
      return;
    }
    const filter = target?.closest("[data-codex-task-filter]");
    if (filter) {
      codexPlusTaskSetFilter(filter.getAttribute("data-codex-task-filter"));
      return;
    }
    const thread = target?.closest("[data-codex-task-thread]");
    if (thread) {
      codexPlusTaskOpenThread(thread.getAttribute("data-codex-task-thread"));
      return;
    }
    const statusButton = target?.closest("[data-codex-task-status]");
    if (statusButton && state.selectedId != null) {
      const status = statusButton.getAttribute("data-codex-task-status");
      const task = codexPlusTasks().find((item) => item.id === state.selectedId);
      codexPlusTaskCloseMenu();
      if (task && task.status !== status) codexPlusTaskUpdate(task.id, { status });
      document.querySelector('[data-codex-task-action="statusMenu"]')?.focus({ preventScroll: true });
      return;
    }
    const projectButton = target?.closest("[data-codex-task-project]");
    if (projectButton && state.selectedId != null) {
      const project = projectButton.getAttribute("data-codex-task-project") || "";
      const task = codexPlusTasks().find((item) => item.id === state.selectedId);
      codexPlusTaskCloseMenu();
      if (task && project !== (task.project || "")) codexPlusTaskUpdate(task.id, { project });
      document.querySelector('[data-codex-task-action="projectMenu"]')?.focus({ preventScroll: true });
      return;
    }
    const button = target?.closest("[data-codex-task-action]");
    if (!button) return;
    switch (button.getAttribute("data-codex-task-action")) {
      case "reload":
        await codexPlusTaskRun("list");
        return;
      case "showAll":
        state.showAll = true;
        renderCodexPlusTaskMain();
        return;
      case "new":
        // 已完成 / 已取消下面不能新建：回到「未了结」，新建进收集。
        if (state.filter === "done" || state.filter === "cancelled") codexPlusTaskSetFilter("open");
        document.querySelector("[data-codex-task-quick-add]")?.focus();
        return;
      case "closeDetail":
        codexPlusTaskSelect(null);
        return;
      case "statusMenu":
      case "projectMenu": {
        const key = button.getAttribute("data-codex-task-action");
        const wasOpen = state[key];
        codexPlusTaskCloseMenu();
        if (wasOpen) return;
        state[key] = true;
        renderCodexPlusTaskDetail();
        const menu = document.querySelector(".codex-plus-task-menu");
        (menu?.querySelector('[aria-checked="true"]') || menu?.querySelector("button"))?.focus({ preventScroll: true });
        return;
      }
      case "pickProject": {
        const task = codexPlusTasks().find((item) => item.id === state.selectedId);
        codexPlusTaskCloseMenu();
        if (!task) return;
        // 系统窗口开着期间别的操作照常；选好后 core 直接写进任务，返回整个面板。
        await state.lastWrite;
        await codexPlusTaskRun("pickProject", { id: task.id });
        return;
      }
      case "start": {
        const task = codexPlusTasks().find((item) => item.id === state.selectedId);
        if (!task || button.disabled) return;
        // 点按钮时编辑框已失焦保存（或用键盘触发时还在编辑）：等写完，新对话里填的才是最新的标题和正文。
        const editing = document.activeElement;
        if (editing instanceof Element && editing.matches("[data-codex-task-field]")) codexPlusTaskCommitField(editing);
        button.disabled = true;
        await state.lastWrite;
        // Codex 收到链接后会跳到新对话；这里关掉本页，免得盖在输入框上面。
        if (await codexPlusTaskRun("start", { id: task.id })) closeCodexPlusPage();
        return;
      }
      case "delete": {
        const task = codexPlusTasks().find((item) => item.id === state.selectedId);
        if (!task) return;
        const answer = await codexPlusAsk({
          title: `删除 ${codexPlusTaskKey(task.id)}「${task.title}」？`,
          message: "任务文件会被删掉，编号不会再用。不想做了可以改成「已取消」，记录会保留。",
          options: [{ value: "delete", label: "删除" }],
        });
        if (answer && await codexPlusTaskRun("delete", { id: task.id })) codexPlusTaskSelect(null);
        return;
      }
      default:
    }
  }

  /**
   * Codex 自己的 Esc 处理会把焦点塞回它的输入框（就在本页下面，看不见），之后打的字会落进去。
   * 处理完 Esc 后下一拍把焦点放回本页的一个可聚焦容器（tabindex=-1，不进 Tab 顺序）。
   */
  function codexPlusTaskParkFocus(selector) {
    setTimeout(() => {
      const node = document.querySelector(selector) || document.querySelector("[data-codex-task-shell]");
      if (node?.isConnected) node.focus({ preventScroll: true });
    }, 0);
  }

  function codexPlusTaskHandleKeydown(event) {
    const target = event.target instanceof Element ? event.target : null;
    if (event.isComposing) return;
    const quickAdd = target?.closest("[data-codex-task-quick-add]");
    if (quickAdd) {
      if (event.key === "Enter") {
        event.preventDefault();
        void codexPlusTaskCreate(quickAdd, event.metaKey || event.ctrlKey);
      } else if (event.key === "Escape") {
        quickAdd.value = "";
        quickAdd.blur();
        codexPlusTaskParkFocus("[data-codex-task-shell]");
      }
      return;
    }
    if (target?.matches('[data-codex-task-field="title"]') && event.key === "Enter") {
      event.preventDefault();
      target.blur();
      return;
    }
    // 状态菜单里上下键换项。
    const menu = target?.closest(".codex-plus-task-menu");
    if (menu && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault();
      const items = Array.from(menu.querySelectorAll("button"));
      const index = items.indexOf(target.closest("button"));
      items[(index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
      return;
    }
    // 不在输入框里时，上下键在列表里换任务。
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && !target?.matches("input, textarea") && !event.metaKey && !event.ctrlKey && !event.altKey) {
      event.preventDefault();
      codexPlusTaskStep(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (event.key !== "Escape") return;
    // Esc：先收起状态 / 项目菜单，焦点回到打开它的按钮。
    if (codexPlusTaskBoardState.statusMenu || codexPlusTaskBoardState.projectMenu) {
      const trigger = codexPlusTaskBoardState.projectMenu ? "projectMenu" : "statusMenu";
      codexPlusTaskCloseMenu();
      codexPlusTaskParkFocus(`[data-codex-task-action="${trigger}"]`);
      return;
    }
    // Esc：详情里正在打字时先离开输入框（失焦即保存），再按一次才取消选中。
    if (target?.closest("[data-codex-task-detail]") && target.matches("input, textarea")) {
      target.blur();
      codexPlusTaskParkFocus("[data-codex-task-detail]");
      return;
    }
    if (codexPlusTaskBoardState.selectedId != null) codexPlusTaskSelect(null);
    codexPlusTaskParkFocus("[data-codex-task-shell]");
  }

  /**
   * 页面开着时在 window 捕获阶段接管按键。Codex 在 document 捕获阶段有「随处打字就聚焦输入框」和 Esc 处理，
   * 挂在 overlay 上的监听排在它后面，拦不住：焦点在列表行、按钮或 body 上时，一打字就跳进本页下面看不见的输入框。
   * 只管来自本页（或焦点停在 body）的按键；弹窗、菜单等别处的按键不碰。带 ⌘/Ctrl/Alt 的组合键照常交给 Codex。
   */
  function codexPlusTaskCaptureKeydown(event) {
    const overlay = document.querySelector(".codex-plus-task-page");
    if (!overlay) return;
    const target = event.target instanceof Element ? event.target : null;
    const inside = Boolean(target && overlay.contains(target));
    const idle = event.target === document.body || event.target === document.documentElement;
    if (!inside && !idle) return;
    const typing = Boolean(target?.matches("input, textarea"));
    if (typing || !(event.metaKey || event.ctrlKey || event.altKey)) event.stopPropagation();
    codexPlusTaskHandleKeydown(event);
  }

  /** 拖列表里的任务到侧栏的某个状态上：改成那个状态。 */
  function codexPlusTaskInstallDrag(overlay) {
    let over = null;
    const setOver = (node) => {
      if (over === node) return;
      over?.removeAttribute("data-over");
      over = node;
      over?.setAttribute("data-over", "true");
    };
    overlay.addEventListener("dragstart", (event) => {
      const row = event.target instanceof Element ? event.target.closest("[data-codex-task-open]") : null;
      if (!row || !event.dataTransfer) return;
      event.dataTransfer.setData(codexPlusTaskDragType, row.getAttribute("data-codex-task-open"));
      event.dataTransfer.effectAllowed = "move";
      row.setAttribute("data-dragging", "true");
      overlay.setAttribute("data-dragging", "true");
    });
    overlay.addEventListener("dragover", (event) => {
      const drop = event.target instanceof Element ? event.target.closest("[data-codex-task-drop]") : null;
      if (!drop || !event.dataTransfer?.types.includes(codexPlusTaskDragType)) return setOver(null);
      event.preventDefault();
      setOver(drop);
    });
    overlay.addEventListener("dragleave", (event) => {
      if (!overlay.contains(event.relatedTarget)) setOver(null);
    });
    overlay.addEventListener("dragend", () => {
      setOver(null);
      overlay.removeAttribute("data-dragging");
      overlay.querySelectorAll("[data-dragging]").forEach((node) => node.removeAttribute("data-dragging"));
    });
    overlay.addEventListener("drop", (event) => {
      const drop = event.target instanceof Element ? event.target.closest("[data-codex-task-drop]") : null;
      setOver(null);
      if (!drop) return;
      event.preventDefault();
      const id = Number(event.dataTransfer?.getData(codexPlusTaskDragType));
      const status = drop.getAttribute("data-codex-task-drop");
      const task = codexPlusTasks().find((item) => item.id === id);
      if (task && task.status !== status) codexPlusTaskUpdate(id, { status });
    });
  }

  /** 编辑器或 Agent 改过任务文件：静默重读，内容变了才重画。 */
  function codexPlusTaskPoll() {
    const state = codexPlusTaskBoardState;
    if (!document.querySelector("[data-codex-task-main]") || document.hidden) return;
    void codexPlusTaskRequest("list").then((data) => {
      if (JSON.stringify([data.tasks, data.runs]) === JSON.stringify([state.data?.tasks, state.data?.runs])) return;
      state.data = data;
      state.error = "";
      renderCodexPlusTaskBoard();
    }).catch(() => {});
  }

  function openCodexPlusTaskBoardPage() {
    closeCodexPlusPage();
    document.querySelectorAll('.codex-plus-modal-overlay, [data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const state = codexPlusTaskBoardState;
    try {
      const saved = localStorage.getItem("codex-plus-task-board-filter");
      state.filter = saved && (saved === "open" || codexPlusTaskStatuses.some((entry) => entry.id === saved)) ? saved : "open";
    } catch {}
    const overlay = document.createElement("div");
    overlay.className = `${codexPlusPageClass} codex-plus-task-page`;
    overlay.dataset.codexPlusPage = "true";
    markCodexPlusExtensionNode(overlay, "builtin-task-board");
    applyCodexPlusTheme(overlay);
    applyCodexPlusZoom(overlay);
    overlay.innerHTML = `<div class="codex-plus-modal-content" role="region" aria-label="任务面板" data-codex-plus-active-tab="${codexPlusTaskBoardTab}">
      <div class="codex-plus-task-shell" data-codex-task-shell="true" tabindex="-1">
        <div class="codex-plus-task-panes">
          <nav class="codex-plus-task-sidebar" data-codex-task-sidebar="true" aria-label="任务分类"></nav>
          <section class="codex-plus-task-list-pane" aria-label="任务列表">
            <div class="codex-plus-task-pane-bar">
              <h2 class="codex-plus-task-heading" data-codex-task-heading="true"></h2>
              <span class="codex-plus-task-count" data-codex-task-list-count="true"></span>
              <span class="codex-plus-task-spacer"></span>
              <label class="codex-plus-task-search">${codexPlusTaskIcons.search}<input type="search" data-codex-task-search="true" placeholder="搜索" title="搜索编号、标题、内容、项目" aria-label="搜索任务" value="${escapeHtml(state.query)}" spellcheck="false"></label>
              <button type="button" class="codex-plus-task-icon-button" data-codex-task-action="new" title="新建任务" aria-label="新建任务">${codexPlusTaskIcons.plus}</button>
            </div>
            <div class="codex-plus-remote-banner" data-tone="warn" role="alert" data-codex-task-banner="true" hidden></div>
            <div class="codex-plus-task-main" data-codex-task-main="true"></div>
          </section>
          <aside class="codex-plus-task-detail" data-codex-task-detail="true" aria-label="任务详情" tabindex="-1"></aside>
        </div>
        <footer class="codex-plus-task-status" data-codex-task-summary="true"></footer>
      </div></div>`;
    overlay.addEventListener("click", (event) => { void codexPlusTaskHandleClick(event); });
    overlay.addEventListener("focusout", (event) => {
      if (event.target instanceof Element && event.target.matches("[data-codex-task-field]")) codexPlusTaskCommitField(event.target);
    });
    overlay.addEventListener("input", (event) => {
      if (!(event.target instanceof Element) || !event.target.matches("[data-codex-task-search]")) return;
      state.query = event.target.value;
      state.showAll = false;
      renderCodexPlusTaskMain();
    });
    codexPlusTaskInstallDrag(overlay);
    overlay.addEventListener("pointerdown", () => { state.pointerDown = true; }, true);
    document.body.appendChild(overlay);
    positionCodexPlusPage(overlay);
    observeCodexPlusPageLayout(overlay);
    document.querySelectorAll('[data-codex-plus-ext-rail-active="true"]').forEach((node) => {
      node.removeAttribute("data-codex-plus-ext-rail-active");
      const button = node.querySelector("button") || node;
      button.dataset.active = "false";
      button.removeAttribute("data-selected");
      button.removeAttribute("aria-current");
    });
    setCodexPlusSidebarNavActive(true, "tasks");
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = () => positionCodexPlusPage(document.querySelector(`.${codexPlusPageClass}`));
    window.addEventListener("resize", window.__codexPlusPageResizeHandler);
    // 松开（或拖拽结束、被取消）之后再补画：放到 setTimeout 里，排在这次 click 之后。
    window.removeEventListener("pointerup", state.onRelease, true);
    window.removeEventListener("pointercancel", state.onRelease, true);
    window.removeEventListener("dragend", state.onRelease, true);
    state.onRelease = () => setTimeout(() => {
      if (!state.pointerDown) return;
      state.pointerDown = false;
      if (state.renderPending) {
        state.renderPending = false;
        renderCodexPlusTaskBoard();
      }
    }, 0);
    window.addEventListener("pointerup", state.onRelease, true);
    window.addEventListener("pointercancel", state.onRelease, true);
    window.addEventListener("dragend", state.onRelease, true);
    clearInterval(state.pollTimer);
    state.pollTimer = setInterval(codexPlusTaskPoll, 5000);
    window.removeEventListener("focus", state.onFocus);
    state.onFocus = codexPlusTaskPoll;
    window.addEventListener("focus", state.onFocus);
    window.removeEventListener("keydown", codexPlusTaskCaptureKeydown, true);
    window.addEventListener("keydown", codexPlusTaskCaptureKeydown, true);
    // 关页时由 closeCodexPlusPage 调用（92-extension-host.js）。
    window.__codexPlusExtensionPageCleanup = () => {
      clearInterval(state.pollTimer);
      state.pollTimer = null;
      window.removeEventListener("focus", state.onFocus);
      state.onFocus = null;
      window.removeEventListener("keydown", codexPlusTaskCaptureKeydown, true);
      window.removeEventListener("pointerup", state.onRelease, true);
      window.removeEventListener("pointercancel", state.onRelease, true);
      window.removeEventListener("dragend", state.onRelease, true);
      state.onRelease = null;
      // 关页前把正在编辑的字段存掉（节点马上被移除，不会再有 focusout）。
      const editing = document.activeElement;
      if (editing instanceof Element && editing.matches("[data-codex-task-field]")) codexPlusTaskCommitField(editing);
    };
    Object.assign(state, { error: "", statusMenu: false, projectMenu: false, showAll: false, refocusQuickAdd: false, pointerDown: false, renderPending: false });
    renderCodexPlusTaskBoard();
    void codexPlusTaskRun("list");
  }
