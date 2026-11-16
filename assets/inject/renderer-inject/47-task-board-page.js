  /**
   * Codex 左侧图标栏的「任务面板」：记下要做什么，看清每件事走到了哪一步。
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
   * 只重画变化的区域（看板 / 详情 / 状态栏），工具栏的搜索框和正在编辑的输入框不被打断。
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
  /** 看板每列先画这么多张卡（「完成」会越积越多），其余点「显示全部」。 */
  const codexPlusTaskColumnLimit = 30;
  const codexPlusTaskDragType = "application/x-codex-plus-task";
  const codexPlusTaskBoardState = {
    data: null,
    error: "",
    query: "",
    view: "board",
    showCancelled: false,
    selectedId: null,
    // 详情里的状态菜单是否展开。
    statusMenu: false,
    expanded: new Set(),
    refocusQuickAdd: "",
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
    board: codexPlusTaskIcon('<rect x="3.5" y="4" width="17" height="16" rx="2.5"/><path d="M9.5 4v16M15 4v9"/>'),
    list: codexPlusTaskIcon('<path d="M9 6h11M9 12h11M9 18h11"/><circle cx="4.5" cy="6" r=".6"/><circle cx="4.5" cy="12" r=".6"/><circle cx="4.5" cy="18" r=".6"/>'),
    folder: codexPlusTaskIcon('<path d="M3.5 7.5a2 2 0 0 1 2-2h3.6l2 2.2h7.4a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>'),
    check: codexPlusTaskIcon('<rect x="4" y="4" width="16" height="16" rx="3.5"/><path d="m8.5 12.2 2.4 2.4 4.8-5"/>'),
    play: codexPlusTaskIcon('<path d="M8 5.5v13l10.5-6.5z"/>', ' data-fill="true"'),
    chevron: codexPlusTaskIcon('<path d="m7 10 5 5 5-5"/>'),
    trash: codexPlusTaskIcon('<path d="M4 7h16M9.5 7V4.5h5V7M6.5 7l.9 12.5h9.2l.9-12.5"/>'),
    close: codexPlusTaskIcon('<path d="M6 6l12 12M18 6 6 18"/>'),
    chat: codexPlusTaskIcon('<path d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8.5A1.5 1.5 0 0 1 19 17h-8l-4.5 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5V7A1.5 1.5 0 0 1 5 5.5z"/>'),
  };

  const codexPlusTaskKey = (id) => `T-${id}`;
  const codexPlusTaskStatusLabel = (status) => codexPlusTaskStatuses.find((entry) => entry.id === status)?.label || status;

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

  /** 卡片上的正文摘要：去掉 Markdown 标记，几行拼成一句。 */
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

  /** 按状态分列；每列里最近改过状态的在前（刚拖进来的在最上面）。 */
  function codexPlusTaskGroups(tasks) {
    const groups = Object.fromEntries(codexPlusTaskStatuses.map((entry) => [entry.id, []]));
    for (const task of tasks) (groups[task.status] || groups.backlog).push(task);
    for (const list of Object.values(groups)) list.sort((a, b) => (b.statusAt - a.statusAt) || (b.id - a.id));
    return groups;
  }

  const codexPlusTasks = () => (Array.isArray(codexPlusTaskBoardState.data?.tasks) ? codexPlusTaskBoardState.data.tasks : []);
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

  /** 卡片 / 列表行上的小标签：项目、清单进度、对话进展。 */
  function codexPlusTaskTags(task) {
    const runs = codexPlusTaskRunsFor(task.id);
    const running = runs.filter((run) => run.state === "running").length;
    const checklist = codexPlusTaskChecklist(task.body);
    return [
      task.project ? `<span class="codex-plus-task-chip" data-kind="project" title="${escapeHtml(task.project)}">${codexPlusTaskIcons.folder}<span>${escapeHtml(codexPlusTaskProjectName(task.project))}</span></span>` : "",
      checklist.total ? `<span class="codex-plus-task-chip" data-complete="${String(checklist.done === checklist.total)}" title="清单完成 ${checklist.done} / ${checklist.total}">${codexPlusTaskIcons.check}<span>${checklist.done}/${checklist.total}</span></span>` : "",
      running
        ? `<span class="codex-plus-task-run-badge" data-tone="running" title="${running} 个 Codex 对话正在跑"><i aria-hidden="true"></i>运行中</span>`
        : runs.length ? `<span class="codex-plus-task-chip" title="${runs.length} 个 Codex 对话">${codexPlusTaskIcons.chat}<span>${runs.length}</span></span>` : "",
    ].join("");
  }

  function codexPlusTaskCard(task) {
    const selected = task.id === codexPlusTaskBoardState.selectedId;
    const excerpt = codexPlusTaskExcerpt(task.body);
    const tags = codexPlusTaskTags(task);
    return `<button type="button" class="codex-plus-task-card" draggable="true" data-codex-task-open="${task.id}" data-selected="${String(selected)}" data-status="${escapeHtml(task.status)}">
      <span class="codex-plus-task-card-head"><code>${codexPlusTaskKey(task.id)}</code><span title="进入这一列：${escapeHtml(codexPlusTaskFullTime(task.statusAt))}">${codexPlusTaskAgo(task.statusAt)}</span></span>
      <span class="codex-plus-task-card-title">${escapeHtml(task.title)}</span>
      ${excerpt ? `<span class="codex-plus-task-card-excerpt">${escapeHtml(excerpt)}</span>` : ""}
      ${tags ? `<span class="codex-plus-task-card-tags">${tags}</span>` : ""}
    </button>`;
  }

  function codexPlusTaskColumn(entry, tasks) {
    const state = codexPlusTaskBoardState;
    const canAdd = entry.id === "backlog" || entry.id === "todo";
    const shown = state.expanded.has(entry.id) ? tasks : tasks.slice(0, codexPlusTaskColumnLimit);
    const quickAdd = canAdd ? `<label class="codex-plus-task-quick-add">
        ${codexPlusTaskIcons.plus}
        <input type="text" data-codex-task-quick-add="${entry.id}" placeholder="${entry.id === "backlog" ? "记一件事，回车保存" : "加一件要做的事"}" title="回车保存；⌘/Ctrl+回车保存并打开详情" aria-label="${escapeHtml(entry.label)}：新建任务">
      </label>` : "";
    const more = tasks.length > shown.length ? `<button type="button" class="codex-plus-task-more" data-codex-task-expand="${entry.id}">显示全部 ${tasks.length} 个</button>` : "";
    const empty = !tasks.length && !canAdd ? `<div class="codex-plus-task-column-empty">${escapeHtml(entry.hint)}</div>` : "";
    return `<section class="codex-plus-task-column" data-codex-task-column="${entry.id}" aria-label="${escapeHtml(entry.label)}">
      <header title="${escapeHtml(entry.hint)}"><span class="codex-plus-task-dot" data-status="${entry.id}"></span><h3>${escapeHtml(entry.label)}</h3><span class="codex-plus-task-count">${tasks.length}</span></header>
      <div class="codex-plus-task-cards">${quickAdd}${shown.map(codexPlusTaskCard).join("")}${more}${empty}</div>
    </section>`;
  }

  function codexPlusTaskListView(groups, statuses) {
    const sections = statuses.filter((entry) => groups[entry.id].length).map((entry) => `<section class="codex-plus-task-list-group">
      <header><span class="codex-plus-task-dot" data-status="${entry.id}"></span><h3>${escapeHtml(entry.label)}</h3><span class="codex-plus-task-count">${groups[entry.id].length}</span></header>
      ${groups[entry.id].map((task) => `<button type="button" class="codex-plus-task-list-row" data-codex-task-open="${task.id}" data-selected="${String(task.id === codexPlusTaskBoardState.selectedId)}" data-status="${escapeHtml(task.status)}">
        <span class="codex-plus-task-dot" data-status="${escapeHtml(task.status)}"></span>
        <code>${codexPlusTaskKey(task.id)}</code>
        <span class="codex-plus-task-list-title">${escapeHtml(task.title)}</span>
        <span class="codex-plus-task-list-tags">${codexPlusTaskTags(task)}</span>
        <span class="codex-plus-task-list-time" title="更新于 ${escapeHtml(codexPlusTaskFullTime(task.updatedAt))}">${codexPlusTaskAgo(task.updatedAt)}</span>
      </button>`).join("")}
    </section>`).join("");
    const empty = codexPlusTaskBoardState.query.trim()
      ? `<div class="codex-plus-task-empty">${codexPlusTaskIcons.search}<strong>没有匹配的任务</strong><span>换个关键词试试，编号写 T-12 或 12 都行。</span></div>`
      : `<div class="codex-plus-task-empty">${codexPlusTaskIcons.board}<strong>还没有任务</strong><span>切到看板，在「收集」列顶上写一句就能记下。</span></div>`;
    return `<div class="codex-plus-task-list">${sections || empty}</div>`;
  }

  function renderCodexPlusTaskMain() {
    const main = document.querySelector("[data-codex-task-main]");
    if (!main) return;
    const state = codexPlusTaskBoardState;
    if (!state.data) {
      main.innerHTML = `<div class="codex-plus-task-empty">${state.error ? '<button type="button" class="codex-plus-action-button" data-codex-task-action="reload">重试</button>' : "正在读取…"}</div>`;
      return;
    }
    const groups = codexPlusTaskGroups(codexPlusTasks().filter((task) => codexPlusTaskMatches(task, state.query)));
    const statuses = codexPlusTaskStatuses.filter((entry) => state.showCancelled || entry.id !== "cancelled");
    const scrolls = Array.from(main.querySelectorAll("[data-codex-task-column] .codex-plus-task-cards"), (node) => [node.closest("[data-codex-task-column]").dataset.codexTaskColumn, node.scrollTop]);
    const scrollX = main.querySelector(".codex-plus-task-columns")?.scrollLeft || 0;
    // 快速新建框正在打字时（定时重读触发重画），保住已经打的字和光标。
    const active = document.activeElement;
    const typing = active instanceof HTMLInputElement && main.contains(active) && active.matches("[data-codex-task-quick-add]")
      ? { status: active.getAttribute("data-codex-task-quick-add"), value: active.value, start: active.selectionStart, end: active.selectionEnd }
      : null;
    main.innerHTML = state.view === "list"
      ? codexPlusTaskListView(groups, statuses)
      : `<div class="codex-plus-task-columns">${statuses.map((entry) => codexPlusTaskColumn(entry, groups[entry.id])).join("")}</div>`;
    // 重画后保住各列的滚动位置，以及刚用过的快速新建输入框的焦点。
    for (const [status, top] of scrolls) {
      const node = main.querySelector(`[data-codex-task-column="${status}"] .codex-plus-task-cards`);
      if (node) node.scrollTop = top;
    }
    const columns = main.querySelector(".codex-plus-task-columns");
    if (columns) columns.scrollLeft = scrollX;
    if (state.refocusQuickAdd) {
      main.querySelector(`[data-codex-task-quick-add="${state.refocusQuickAdd}"]`)?.focus();
      state.refocusQuickAdd = "";
    } else if (typing) {
      const input = main.querySelector(`[data-codex-task-quick-add="${typing.status}"]`);
      if (input) {
        input.value = typing.value;
        input.focus();
        input.setSelectionRange(typing.start, typing.end);
      }
    }
  }

  function renderCodexPlusTaskDetail() {
    const aside = document.querySelector("[data-codex-task-detail]");
    if (!aside) return;
    const state = codexPlusTaskBoardState;
    const task = codexPlusTasks().find((item) => item.id === state.selectedId);
    if (!task) {
      state.selectedId = null;
      aside.hidden = true;
      aside.innerHTML = "";
      return;
    }
    aside.hidden = false;
    aside.dataset.taskId = String(task.id);
    const projects = [...new Set(codexPlusTasks().map((item) => item.project).filter(Boolean))];
    const runs = codexPlusTaskRunsFor(task.id);
    const current = codexPlusTaskStatuses.find((entry) => entry.id === task.status) || codexPlusTaskStatuses[0];
    const checklist = codexPlusTaskChecklist(task.body);
    aside.innerHTML = `<header>
        <span class="codex-plus-task-dot" data-status="${current.id}"></span><code>${codexPlusTaskKey(task.id)}</code><span class="codex-plus-task-spacer"></span>
        <button type="button" class="codex-plus-task-icon-button" data-codex-task-action="delete" title="删除任务" aria-label="删除任务">${codexPlusTaskIcons.trash}</button>
        <button type="button" class="codex-plus-task-icon-button" data-codex-task-action="closeDetail" title="关闭（Esc）" aria-label="关闭（Esc）">${codexPlusTaskIcons.close}</button>
      </header>
      <div class="codex-plus-task-detail-body">
        <textarea class="codex-plus-task-title-input" data-codex-task-field="title" rows="1" aria-label="标题" placeholder="标题">${escapeHtml(task.title)}</textarea>
        <div class="codex-plus-task-props">
          <span class="codex-plus-task-prop-label">状态</span>
          <div class="codex-plus-task-prop-value codex-plus-task-status-field">
            <button type="button" class="codex-plus-task-select" data-codex-task-action="statusMenu" aria-haspopup="menu" aria-expanded="${String(state.statusMenu)}" title="${escapeHtml(current.hint)}">
              <span class="codex-plus-task-dot" data-status="${current.id}"></span><span>${escapeHtml(current.label)}</span>${codexPlusTaskIcons.chevron}</button>
            <span class="codex-plus-task-prop-note" title="${escapeHtml(codexPlusTaskFullTime(task.statusAt))}">${codexPlusTaskAgo(task.statusAt)}进入</span>
            ${state.statusMenu ? codexPlusTaskStatusMenu(task) : ""}
          </div>
          <label class="codex-plus-task-prop-label" for="codex-plus-task-project">项目</label>
          <div class="codex-plus-task-prop-value">
            <input type="text" id="codex-plus-task-project" class="codex-plus-task-ghost-input" data-codex-task-field="project" list="codex-plus-task-projects" value="${escapeHtml(task.project || "")}" placeholder="项目目录，可不填" spellcheck="false">
            <datalist id="codex-plus-task-projects">${projects.map((value) => `<option value="${escapeHtml(value)}"></option>`).join("")}</datalist>
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

  /** 详情里的状态菜单：每项一个状态，带说明；当前状态打勾。 */
  function codexPlusTaskStatusMenu(task) {
    return `<div class="codex-plus-task-menu" role="menu" aria-label="改状态">${codexPlusTaskStatuses.map((entry) => `<button type="button" role="menuitemradio" aria-checked="${String(entry.id === task.status)}" data-codex-task-status="${entry.id}">
        <span class="codex-plus-task-dot" data-status="${entry.id}"></span><span class="codex-plus-task-menu-label">${escapeHtml(entry.label)}</span><span class="codex-plus-task-menu-hint">${escapeHtml(entry.hint)}</span></button>`).join("")}</div>`;
  }

  /** 收起状态菜单：只摘掉菜单节点，不重画详情（正在编辑的输入框不被打断）。 */
  function codexPlusTaskCloseMenu() {
    codexPlusTaskBoardState.statusMenu = false;
    document.querySelector(".codex-plus-task-menu")?.remove();
    document.querySelector('[data-codex-task-action="statusMenu"]')?.setAttribute("aria-expanded", "false");
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
    const open = tasks.filter((task) => task.status !== "done" && task.status !== "cancelled").length;
    const review = count("in_review");
    const blocked = count("blocked");
    const dir = codexPlusTaskBoardState.data?.dir || "";
    bar.innerHTML = `<span>未了结 ${open}</span><span>进行中 ${count("in_progress")}</span>
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
    renderCodexPlusTaskMain();
    const aside = document.querySelector("[data-codex-task-detail]");
    const editing = aside?.contains(document.activeElement) && document.activeElement?.matches("input, textarea");
    const switched = aside && aside.dataset.taskId !== String(codexPlusTaskBoardState.selectedId ?? "");
    if (detail && (!editing || switched)) renderCodexPlusTaskDetail();
    renderCodexPlusTaskStatus();
    const reveal = codexPlusTaskBoardState.revealId;
    codexPlusTaskBoardState.revealId = null;
    if (reveal != null) codexPlusTaskRevealColumn(reveal);
  }

  // ---------- 操作 ----------

  /** 先在本地改掉再写文件：拖卡片时不用等写完才挪过去。 */
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
      // 失败时保留写好的字（重画前已存进 state 之外的输入框，这里补回去）。
      const again = document.querySelector(`[data-codex-task-quick-add="${status}"]`);
      if (again) again.value = title;
      return;
    }
    if (openAfter) {
      codexPlusTaskBoardState.selectedId = data.created;
      renderCodexPlusTaskBoard();
      document.querySelector('[data-codex-task-field="body"]')?.focus();
    } else {
      codexPlusTaskBoardState.refocusQuickAdd = status;
      renderCodexPlusTaskMain();
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
    } else if (field === "project") {
      if (value.trim() !== (task.project || "")) codexPlusTaskUpdate(id, { project: value.trim() });
    } else if (field === "body") {
      if (value.trimEnd() !== task.body) codexPlusTaskUpdate(id, { body: value });
    }
  }

  function codexPlusTaskSelect(id) {
    codexPlusTaskBoardState.selectedId = id;
    codexPlusTaskBoardState.statusMenu = false;
    codexPlusTaskBoardState.revealId = id;
    renderCodexPlusTaskBoard();
  }

  /**
   * 打开详情后看板变窄：选中卡片所在的列不在视野里时，横向滚过去。
   * 由 renderCodexPlusTaskBoard 在真正重画之后调用（鼠标按下期间重画会推迟，提前算位置是错的）。
   */
  function codexPlusTaskRevealColumn(id) {
    const columns = document.querySelector(".codex-plus-task-columns");
    const column = columns?.querySelector(`[data-codex-task-open="${id}"]`)?.closest("[data-codex-task-column]");
    if (!column) return;
    const box = columns.getBoundingClientRect();
    const rect = column.getBoundingClientRect();
    const delta = rect.left < box.left ? rect.left - box.left : rect.right > box.right ? rect.right - box.right : 0;
    if (Math.abs(delta) > 1) columns.scrollBy({ left: delta, behavior: "smooth" });
  }

  async function codexPlusTaskHandleClick(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const state = codexPlusTaskBoardState;
    // 状态菜单开着时，点菜单和触发按钮以外的地方先收起菜单，这一下点击照常生效。
    if (state.statusMenu && !target?.closest('.codex-plus-task-menu, [data-codex-task-action="statusMenu"]')) codexPlusTaskCloseMenu();
    const open = target?.closest("[data-codex-task-open]");
    if (open) {
      codexPlusTaskSelect(Number(open.getAttribute("data-codex-task-open")));
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
    const expand = target?.closest("[data-codex-task-expand]");
    if (expand) {
      state.expanded.add(expand.getAttribute("data-codex-task-expand"));
      renderCodexPlusTaskMain();
      return;
    }
    const view = target?.closest("[data-codex-task-view]");
    if (view) {
      state.view = view.getAttribute("data-codex-task-view") === "list" ? "list" : "board";
      try { localStorage.setItem("codex-plus-task-board-view", state.view); } catch {}
      document.querySelectorAll("[data-codex-task-view]").forEach((node) => node.setAttribute("aria-checked", String(node.getAttribute("data-codex-task-view") === state.view)));
      renderCodexPlusTaskMain();
      return;
    }
    const button = target?.closest("[data-codex-task-action]");
    if (!button) return;
    switch (button.getAttribute("data-codex-task-action")) {
      case "reload":
        await codexPlusTaskRun("list");
        return;
      case "new":
        if (state.view !== "board") {
          state.view = "board";
          document.querySelectorAll("[data-codex-task-view]").forEach((node) => node.setAttribute("aria-checked", String(node.getAttribute("data-codex-task-view") === "board")));
          renderCodexPlusTaskMain();
        }
        document.querySelector('[data-codex-task-quick-add="backlog"]')?.focus();
        return;
      case "closeDetail":
        codexPlusTaskSelect(null);
        return;
      case "statusMenu":
        if (state.statusMenu) {
          codexPlusTaskCloseMenu();
          return;
        }
        state.statusMenu = true;
        renderCodexPlusTaskDetail();
        document.querySelector('.codex-plus-task-menu [aria-checked="true"]')?.focus({ preventScroll: true });
        return;
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
    if (target?.matches('[data-codex-task-field="title"], [data-codex-task-field="project"]') && event.key === "Enter") {
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
    if (event.key !== "Escape") return;
    // Esc：先收起状态菜单。
    if (codexPlusTaskBoardState.statusMenu) {
      codexPlusTaskCloseMenu();
      codexPlusTaskParkFocus('[data-codex-task-action="statusMenu"]');
      return;
    }
    // Esc：详情里正在打字时先离开输入框（失焦即保存），再按一次才关详情。
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
   * 挂在 overlay 上的监听排在它后面，拦不住：焦点在卡片、按钮或 body 上时，一打字就跳进本页下面看不见的输入框。
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

  function codexPlusTaskInstallDrag(overlay) {
    let overColumn = null;
    const setOver = (column) => {
      if (overColumn === column) return;
      overColumn?.removeAttribute("data-over");
      overColumn = column;
      overColumn?.setAttribute("data-over", "true");
    };
    overlay.addEventListener("dragstart", (event) => {
      const card = event.target instanceof Element ? event.target.closest("[data-codex-task-open]") : null;
      if (!card || !event.dataTransfer) return;
      event.dataTransfer.setData(codexPlusTaskDragType, card.getAttribute("data-codex-task-open"));
      event.dataTransfer.effectAllowed = "move";
      card.setAttribute("data-dragging", "true");
    });
    overlay.addEventListener("dragover", (event) => {
      const column = event.target instanceof Element ? event.target.closest("[data-codex-task-column]") : null;
      if (!column || !event.dataTransfer?.types.includes(codexPlusTaskDragType)) return setOver(null);
      event.preventDefault();
      setOver(column);
    });
    overlay.addEventListener("dragleave", (event) => {
      if (!overlay.contains(event.relatedTarget)) setOver(null);
    });
    overlay.addEventListener("dragend", () => {
      setOver(null);
      overlay.querySelectorAll("[data-dragging]").forEach((node) => node.removeAttribute("data-dragging"));
    });
    overlay.addEventListener("drop", (event) => {
      const column = event.target instanceof Element ? event.target.closest("[data-codex-task-column]") : null;
      setOver(null);
      if (!column) return;
      event.preventDefault();
      const id = Number(event.dataTransfer?.getData(codexPlusTaskDragType));
      const status = column.getAttribute("data-codex-task-column");
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
    try { state.view = localStorage.getItem("codex-plus-task-board-view") === "list" ? "list" : "board"; } catch {}
    const overlay = document.createElement("div");
    overlay.className = `${codexPlusPageClass} codex-plus-task-page`;
    overlay.dataset.codexPlusPage = "true";
    markCodexPlusExtensionNode(overlay, "builtin-task-board");
    applyCodexPlusTheme(overlay);
    applyCodexPlusZoom(overlay);
    overlay.innerHTML = `<div class="codex-plus-modal-content" role="region" aria-label="任务面板" data-codex-plus-active-tab="${codexPlusTaskBoardTab}">
      <div class="codex-plus-task-shell" data-codex-task-shell="true" tabindex="-1">
        <div class="codex-plus-task-toolbar">
          <h2 class="codex-plus-task-heading">任务面板</h2>
          <span class="codex-plus-task-spacer"></span>
          <label class="codex-plus-task-search">${codexPlusTaskIcons.search}<input type="search" data-codex-task-search="true" placeholder="搜索编号、标题、内容、项目" aria-label="搜索任务" value="${escapeHtml(state.query)}" spellcheck="false"></label>
          <label class="codex-plus-task-toggle" title="把「已取消」的任务也列出来"><input type="checkbox" data-codex-task-cancelled="true" ${state.showCancelled ? "checked" : ""}>显示已取消</label>
          <div class="codex-plus-task-segmented" role="radiogroup" aria-label="视图">
            <button type="button" role="radio" data-codex-task-view="board" aria-checked="${String(state.view === "board")}">${codexPlusTaskIcons.board}看板</button>
            <button type="button" role="radio" data-codex-task-view="list" aria-checked="${String(state.view === "list")}">${codexPlusTaskIcons.list}列表</button>
          </div>
          <button type="button" class="codex-plus-task-primary" data-codex-task-action="new">${codexPlusTaskIcons.plus}新建任务</button>
        </div>
        <div class="codex-plus-remote-banner" data-tone="warn" role="alert" data-codex-task-banner="true" hidden></div>
        <div class="codex-plus-task-body">
          <div class="codex-plus-task-main" data-codex-task-main="true"></div>
          <aside class="codex-plus-task-detail" data-codex-task-detail="true" aria-label="任务详情" tabindex="-1" hidden></aside>
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
      renderCodexPlusTaskMain();
    });
    overlay.addEventListener("change", (event) => {
      if (!(event.target instanceof Element) || !event.target.matches("[data-codex-task-cancelled]")) return;
      state.showCancelled = event.target.checked;
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
    Object.assign(state, { error: "", statusMenu: false, expanded: new Set(), refocusQuickAdd: "", pointerDown: false, renderPending: false });
    renderCodexPlusTaskBoard();
    void codexPlusTaskRun("list");
  }
