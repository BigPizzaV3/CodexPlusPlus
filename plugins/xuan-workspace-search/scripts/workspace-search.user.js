(() => {
  const API_KEY = "__codexPlusWorkspaceSearch";
  const ROOT_ID = "codex-plus-workspace-search";
  const STYLE_ID = `${ROOT_ID}-style`;
  const BUTTON_CLASS = `${ROOT_ID}-button`;
  const POLL_INTERVAL_MS = 80;
  const SEARCH_DEBOUNCE_MS = 250;

  try {
    window[API_KEY]?.cleanup?.();
  } catch (_) {}
  if (window.top !== window || window.self !== window || !window.electronBridge || !/^app:\/\/-\//i.test(window.location.href)) return;

  const state = {
    root: null,
    query: null,
    include: null,
    exclude: null,
    caseSensitive: null,
    wholeWord: null,
    regex: null,
    status: null,
    stop: null,
    results: null,
    preview: null,
    projectSelect: null,
    projects: [],
    selectedProjectId: "",
    workspacePath: "",
    selectionMode: "auto",
    searchId: "",
    searchRevision: 0,
    debounceTimer: 0,
    contextTimer: 0,
    pollTimer: 0,
    observer: null,
    previousFocus: null,
  };

  function bridgeCall(path, payload = {}) {
    const route = {
      "/workspace-search/roots": "/v1/search/roots",
      "/workspace-search/projects": "/v1/search/projects",
      "/workspace-search/current-root": "/v1/search/current-root",
      "/workspace-search/start": "/v1/search/start",
      "/workspace-search/poll": "/v1/search/poll",
      "/workspace-search/cancel": "/v1/search/cancel",
      "/workspace-search/preview": "/v1/search/preview",
    }[path];
    if (!route) return Promise.resolve({ status: "failed", message: "Xuan 搜索请求不受支持" });
    return Promise.resolve().then(() => {
      const pageBridge = window.__xuanPluginBridge?.["xuan-workspace-search"];
      if (typeof pageBridge !== "function") throw new Error("搜索插件尚未连接，请确认插件已启用并重新打开任务");
      return pageBridge(route, payload);
    }).catch((error) => ({
      status: "failed",
      message: /\p{Script=Han}/u.test(error?.message || "") ? error.message : "搜索插件请求失败，请重试",
    }));
  }

  function absoluteLocalPath(value) {
    const rawPath = String(value || "").trim();
    const path = /^(?:codex|local):(?=[A-Za-z]:[\\/]|\\\\|\/(?!\/))/i.test(rawPath)
      ? rawPath.replace(/^(?:codex|local):/i, "")
      : rawPath;
    if (!path || path.includes("\0") || path.includes("://")) return "";
    if (/^[A-Za-z]:[\\/]/.test(path) || /^\\\\[^\\]+\\[^\\]+/.test(path) || path.startsWith("/")) return path;
    return "";
  }

  function reactKeys(element) {
    return Object.keys(element || {})
      .filter((key) => key.startsWith("__reactFiber") || key.startsWith("__reactInternalInstance") || key.startsWith("__reactProps"))
      .sort((left, right) => Number(right.startsWith("__reactProps")) - Number(left.startsWith("__reactProps")));
  }

  function workspacePathFromObject(source) {
    if (!source || typeof source !== "object") return "";
    for (const key of ["cwd", "workspaceRoot", "rootPath", "workingDirectory", "workingDir", "displayCwd"]) {
      const path = absoluteLocalPath(source[key]);
      if (path && !/[\\/]\.codex$/i.test(path)) return path;
    }
    return "";
  }

  function walkObject(root, visitor) {
    const visited = new WeakSet();
    const stack = [{ value: root, depth: 0 }];
    let scanned = 0;
    while (stack.length && scanned < 360) {
      const { value, depth } = stack.pop();
      if (!value || typeof value !== "object" || visited.has(value) || depth > 9) continue;
      visited.add(value);
      scanned += 1;
      const result = visitor(value);
      if (result) return result;
      if (value instanceof Element || value === window || value === document) continue;
      for (const key of Object.keys(value).slice(0, 90)) {
        if ([
          "ownerDocument", "parentElement", "parentNode", "children", "childNodes",
          "return", "sibling", "child", "alternate", "stateNode", "_owner",
        ].includes(key)) continue;
        try {
          const child = value[key];
          if (child && typeof child === "object") stack.push({ value: child, depth: depth + 1 });
        } catch (_) {}
      }
    }
    return "";
  }

  function workspacePathFromElement(element) {
    for (const key of reactKeys(element)) {
      const path = walkObject(element[key], workspacePathFromObject);
      if (path) return path;
    }
    return workspacePathFromReactAncestors(element);
  }

  function workspacePathFromReactAncestors(element) {
    for (const key of reactKeys(element)) {
      const seen = new Set();
      let fiber = element[key];
      for (let level = 0; fiber && level < 80; level += 1) {
        if ((typeof fiber !== "object" && typeof fiber !== "function") || seen.has(fiber)) break;
        seen.add(fiber);
        for (const source of [fiber.pendingProps, fiber.memoizedProps, fiber.memoizedState]) {
          const path = workspacePathFromObject(source);
          if (path) return path;
        }
        fiber = fiber.return;
      }
    }
    return "";
  }

  function visibleElement(element) {
    if (!(element instanceof Element)) return false;
    const rect = element.getBoundingClientRect();
    const style = window.getComputedStyle(element);
    return rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden";
  }

  function normalizedElementText(element) {
    return String(element?.innerText || element?.textContent || "").replace(/\s+/g, " ").trim().toLowerCase();
  }

  function projectRows() {
    return [...document.querySelectorAll('[data-app-action-sidebar-project-row]')];
  }

  function projectLabelFromRow(row) {
    if (!row) return "";
    const labels = [
      row.getAttribute("data-app-action-sidebar-project-label"),
      row.getAttribute("aria-label"),
    ];
    for (const label of labels) {
      const value = String(label || "").replace(/\s+/g, " ").trim();
      if (value) return value;
    }
    return "";
  }

  function projectPathFromRow(row) {
    if (!row) return "";
    for (const value of [
      row.getAttribute("data-app-action-sidebar-project-path"),
      row.getAttribute("data-app-action-sidebar-project-id"),
      row.getAttribute("data-workspace-root"),
      row.getAttribute("data-cwd"),
      row.getAttribute("data-root-path"),
    ]) {
      const path = absoluteLocalPath(value);
      if (path) return path;
    }
    return "";
  }

  function activeThreadRow() {
    const rows = [...document.querySelectorAll('[data-app-action-sidebar-thread-id]')];
    return rows.find((row) => row.getAttribute("data-app-action-sidebar-thread-active") === "true")
      || rows.find((row) => row.getAttribute("aria-current") === "page" || row.getAttribute("aria-current") === "true")
      || rows.find((row) => row.querySelector('[aria-current="page"], [aria-current="true"]'))
      || document.querySelector('[data-app-action-sidebar-thread-active="true"]');
  }

  function threadIdFromRow(row) {
    return String(row?.getAttribute?.("data-app-action-sidebar-thread-id") || "").trim();
  }

  function workspacePathFromAncestors(element) {
    for (let node = element; node && node !== document.body; node = node.parentElement) {
      const path = workspacePathFromElement(node);
      if (path) return path;
    }
    return "";
  }

  function currentProjectPathFromActiveThread() {
    const activeRow = activeThreadRow();
    if (!activeRow) return "";
    const projectRow = activeRow.closest('[data-app-action-sidebar-project-row]');
    return projectPathFromRow(projectRow) || workspacePathFromAncestors(activeRow);
  }

  function currentProjectNameFromActiveThread() {
    const activeRow = activeThreadRow();
    return projectLabelFromRow(activeRow?.closest('[data-app-action-sidebar-project-row]'));
  }

  async function currentProjectPathFromThreadHint() {
    const threadId = threadIdFromRow(activeThreadRow());
    if (!threadId) return "";
    const response = await bridgeCall("/workspace-search/current-root", { threadId });
    return response?.status === "ok" ? absoluteLocalPath(response.root) : "";
  }

  function currentProjectPathFromSelectedButton() {
    const rows = projectRows();
    const buttons = [...document.querySelectorAll('button[aria-haspopup="menu"]')]
      .filter(visibleElement)
      .filter((button) => button.getBoundingClientRect().x > 300);
    for (const button of buttons) {
      const label = normalizedElementText(button);
      if (!label) continue;
      const paths = rows.filter((candidate) => {
        const rowPath = projectPathFromRow(candidate);
        const pathLabel = rowPath.split(/[\\/]/).filter(Boolean).pop() || "";
        return [
          candidate.getAttribute("data-app-action-sidebar-project-label"),
          candidate.getAttribute("aria-label"),
          pathLabel,
        ].some((value) => normalizedElementText({ textContent: value }) === label);
      }).map(projectPathFromRow).filter(Boolean);
      const uniquePaths = [...new Map(paths.map((path) => [normalizedPathKey(path), path])).values()];
      if (uniquePaths.length === 1) return uniquePaths[0];
    }
    return "";
  }

  function currentProjectPathFromMainView() {
    const scopes = [document.querySelector("main"), document.querySelector(".composer-footer")].filter(Boolean);
    const paths = [];
    for (const scope of scopes) {
      for (const selector of ["[data-workspace-root]", "[data-cwd]", "[data-root-path]"]) {
        for (const node of scope.querySelectorAll(selector)) {
          const path = absoluteLocalPath(node.dataset?.workspaceRoot || node.dataset?.cwd || node.dataset?.rootPath);
          if (path) paths.push(path);
        }
      }
      const path = workspacePathFromElement(scope);
      if (path) paths.push(path);
    }
    const uniquePaths = [...new Map(paths.map((path) => [normalizedPathKey(path), path])).values()];
    return uniquePaths.length === 1 ? uniquePaths[0] : "";
  }

  function currentProjectPathFromExpandedRow() {
    const rows = projectRows()
      .filter(visibleElement)
      .filter((row) => row.getAttribute("data-app-action-sidebar-project-collapsed") === "false")
      .map(projectPathFromRow)
      .filter(Boolean);
    return rows.length === 1 ? rows[0] : "";
  }

  async function currentWorkspacePath() {
    const activeProjectPath = currentProjectPathFromActiveThread();
    if (activeProjectPath) return activeProjectPath;
    const hintedProjectPath = await currentProjectPathFromThreadHint();
    if (hintedProjectPath) return hintedProjectPath;
    return currentProjectPathFromSelectedButton()
      || currentProjectPathFromMainView()
      || currentProjectPathFromExpandedRow();
  }

  function normalizedPathKey(value) {
    return absoluteLocalPath(value).replace(/[\\/]+$/, "").toLowerCase();
  }

  function projectById(projectId) {
    return state.projects.find((project) => project.id === projectId);
  }

  function normalizedProjects(values) {
    const projects = [];
    const ids = new Set();
    for (const value of Array.isArray(values) ? values : []) {
      const id = String(value?.id || "").trim();
      const name = String(value?.name || "").trim();
      if (!id || !name || ids.has(id)) continue;
      ids.add(id);
      const roots = (Array.isArray(value?.roots) ? value.roots : [])
        .map(absoluteLocalPath)
        .filter(Boolean);
      projects.push({ id, name, root: roots[0] || "" });
    }
    return projects;
  }

  async function loadProjects() {
    const response = await bridgeCall("/workspace-search/projects", {
      threadId: threadIdFromRow(activeThreadRow()),
    });
    if (response?.status !== "ok") {
      state.projects = [];
      renderProjectOptions();
      return { selectedProjectId: "", currentProjectId: "" };
    }
    state.projects = normalizedProjects(response.projects);
    renderProjectOptions();
    return {
      selectedProjectId: String(response.selectedProjectId || "").trim(),
      currentProjectId: String(response.currentProjectId || "").trim(),
    };
  }

  function renderProjectOptions() {
    const select = state.projectSelect;
    if (!select) return;
    select.replaceChildren();
    if (!state.projects.length) {
      const option = document.createElement("option");
      option.value = "";
      option.textContent = "未找到已创建的项目";
      select.appendChild(option);
      select.disabled = true;
      select.title = "请先在 Codex 中创建或打开项目";
      return;
    }
    for (const project of state.projects) {
      const option = document.createElement("option");
      option.value = project.id;
      option.textContent = project.name;
      option.title = project.root || "该项目未关联可访问的工作区";
      option.disabled = !project.root;
      select.appendChild(option);
    }
    select.disabled = false;
    select.value = state.selectedProjectId;
    if (!select.value) select.value = state.projects.find((project) => project.root)?.id || "";
    const selected = projectById(select.value);
    select.title = selected?.root || "该项目未关联可访问的工作区";
  }

  async function setSelectedProject(projectId, { resetOnChange = false } = {}) {
    const project = projectById(projectId);
    const nextProjectId = project?.id || "";
    const nextRoot = project?.root || "";
    const changed = nextProjectId !== state.selectedProjectId
      || normalizedPathKey(nextRoot) !== normalizedPathKey(state.workspacePath);
    state.selectedProjectId = nextProjectId;
    state.workspacePath = nextRoot;
    renderProjectOptions();
    if (changed && resetOnChange) {
      state.searchRevision += 1;
      await cancelActiveSearch();
      renderEmpty(nextRoot ? "输入内容以搜索所选项目" : "所选项目未关联可访问的工作区");
      if (state.preview) state.preview.innerHTML = '<div class="ws-empty">选择搜索结果以预览</div>';
      setStatus(nextRoot ? `已切换至 ${project.name}` : "所选项目不可搜索");
    }
    return nextRoot;
  }

  function colorIsDark(value) {
    const match = String(value || "").match(/rgba?\(\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)(?:\s*,\s*(\d+(?:\.\d+)?))?/i);
    if (!match) return false;
    if (match[4] !== undefined && Number(match[4]) <= 0.05) return false;
    const [red, green, blue] = match.slice(1, 4).map(Number);
    return (red * 0.2126 + green * 0.7152 + blue * 0.0722) < 128;
  }

  function syncTheme() {
    if (!state.root) return;
    const html = document.documentElement;
    const themeHint = [html.className, html.dataset.theme, html.dataset.colorScheme, document.body?.dataset?.theme]
      .join(" ")
      .toLowerCase();
    const explicitDark = /(^|\s)dark(\s|$)/.test(themeHint);
    const explicitLight = /(^|\s)light(\s|$)/.test(themeHint);
    const bodyBackground = document.body ? window.getComputedStyle(document.body).backgroundColor : "";
    const htmlBackground = window.getComputedStyle(html).backgroundColor;
    const dark = explicitDark || (!explicitLight && (colorIsDark(bodyBackground) || colorIsDark(htmlBackground)))
      || (!explicitLight && window.matchMedia?.("(prefers-color-scheme: dark)")?.matches);
    state.root.dataset.theme = dark ? "dark" : "light";
  }

  function ensureStyles() {
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      #${ROOT_ID} { --ws-bg: #ffffff; --ws-surface: #f5f6f8; --ws-surface-hover: #e9eef3; --ws-surface-selected: #dceaf5; --ws-text: #20242a; --ws-muted: #626b77; --ws-border: #c9cdd3; --ws-border-soft: #e1e4e8; --ws-accent: #176fae; --ws-accent-soft: #e1eff9; --ws-mark-bg: #ffd86a; --ws-mark-text: #17191d; --ws-target-bg: #fff0ad; position: fixed; inset: 0; z-index: 2147483100; display: grid; place-items: center; background: rgba(15, 18, 22, .48); color: var(--ws-text); color-scheme: light; font: 13px/1.45 system-ui, sans-serif; }
      #${ROOT_ID}[data-theme="dark"] { --ws-bg: #17191d; --ws-surface: #202329; --ws-surface-hover: #2b3038; --ws-surface-selected: #25445a; --ws-text: #f2f4f7; --ws-muted: #b3bbc6; --ws-border: #444b55; --ws-border-soft: #30353d; --ws-accent: #69b7ec; --ws-accent-soft: #233d50; --ws-mark-bg: #f2c14e; --ws-mark-text: #17191d; --ws-target-bg: #3d3520; color-scheme: dark; }
      #${ROOT_ID}[hidden] { display: none; }
      #${ROOT_ID} * { box-sizing: border-box; letter-spacing: 0; }
      #${ROOT_ID} .ws-dialog { width: min(1040px, calc(100vw - 32px)); height: min(760px, calc(100vh - 32px)); display: grid; grid-template-rows: auto auto minmax(0, 1fr); overflow: hidden; border: 1px solid var(--ws-border); border-radius: 8px; background: var(--ws-bg); box-shadow: 0 18px 55px rgba(0, 0, 0, .32); }
      #${ROOT_ID} .ws-head { min-height: 48px; display: flex; align-items: center; gap: 12px; padding: 8px 12px; border-bottom: 1px solid var(--ws-border-soft); }
      #${ROOT_ID} .ws-head strong { font-size: 14px; }
      #${ROOT_ID} .ws-head small { min-width: 0; overflow: hidden; color: var(--ws-muted); text-overflow: ellipsis; white-space: nowrap; }
      #${ROOT_ID} .ws-head .ws-spacer { flex: 1; }
      #${ROOT_ID} button { appearance: none; color: var(--ws-text) !important; font: inherit; }
      #${ROOT_ID} .ws-icon-button { width: 30px; height: 30px; display: inline-grid; place-items: center; border: 1px solid transparent; border-radius: 5px; background: transparent; cursor: pointer; }
      #${ROOT_ID} .ws-icon-button:hover { border-color: var(--ws-border); background: var(--ws-surface-hover); }
      #${ROOT_ID} .ws-action-button { height: 30px; padding: 0 9px; border: 1px solid var(--ws-border); border-radius: 5px; background: var(--ws-bg); cursor: pointer; }
      #${ROOT_ID} .ws-action-button:hover { background: var(--ws-surface-hover); }
      #${ROOT_ID} .ws-form { display: grid; gap: 8px; padding: 10px 12px; border-bottom: 1px solid var(--ws-border-soft); background: var(--ws-surface); }
      #${ROOT_ID} .ws-query-row { display: grid; grid-template-columns: minmax(0, 1fr) auto auto auto; gap: 6px; }
      #${ROOT_ID} input, #${ROOT_ID} select { min-width: 0; height: 34px; padding: 6px 9px; border: 1px solid var(--ws-border); border-radius: 5px; outline: none; background: var(--ws-bg); color: var(--ws-text); font: inherit; }
      #${ROOT_ID} select { width: min(100%, 360px); cursor: pointer; }
      #${ROOT_ID} select:disabled { color: var(--ws-muted); background: var(--ws-surface); cursor: default; }
      #${ROOT_ID} input:focus, #${ROOT_ID} select:focus { border-color: var(--ws-accent); box-shadow: 0 0 0 1px var(--ws-accent); }
      #${ROOT_ID} .ws-option { min-width: 36px; height: 34px; border: 1px solid var(--ws-border); border-radius: 5px; background: var(--ws-bg); cursor: pointer; }
      #${ROOT_ID} .ws-option[aria-pressed="true"] { border-color: var(--ws-accent); background: var(--ws-accent-soft); color: var(--ws-accent) !important; }
      #${ROOT_ID} .ws-filter-row { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 8px; }
      #${ROOT_ID} .ws-body { min-height: 0; display: grid; grid-template-columns: minmax(300px, 46%) minmax(0, 54%); }
      #${ROOT_ID} .ws-results, #${ROOT_ID} .ws-preview { min-height: 0; overflow: auto; }
      #${ROOT_ID} .ws-results { border-right: 1px solid var(--ws-border-soft); }
      #${ROOT_ID} .ws-empty { padding: 28px 18px; color: var(--ws-muted); text-align: center; }
      #${ROOT_ID} .ws-file { border-bottom: 1px solid var(--ws-border-soft); }
      #${ROOT_ID} .ws-file-head { position: sticky; top: 0; z-index: 1; width: 100%; display: flex; align-items: center; gap: 8px; padding: 7px 10px; border: 0; background: var(--ws-surface); color: var(--ws-text) !important; text-align: left; cursor: pointer; }
      #${ROOT_ID} .ws-file-head strong { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      #${ROOT_ID} .ws-count { margin-left: auto; color: var(--ws-muted); font-variant-numeric: tabular-nums; }
      #${ROOT_ID} .ws-match { appearance: none; width: 100%; display: grid; grid-template-columns: 48px minmax(0, 1fr); gap: 8px; padding: 5px 10px; border: 0; background: var(--ws-bg) !important; color: var(--ws-text) !important; text-align: left; cursor: pointer; }
      #${ROOT_ID} .ws-match:hover { background: var(--ws-surface-hover) !important; }
      #${ROOT_ID} .ws-match.is-selected { background: var(--ws-surface-selected) !important; }
      #${ROOT_ID} .ws-line-number { color: var(--ws-muted) !important; text-align: right; font: 12px/1.5 ui-monospace, SFMono-Regular, Consolas, monospace; }
      #${ROOT_ID} .ws-line-text { min-width: 0; overflow: hidden; color: var(--ws-text) !important; font: 12px/1.5 ui-monospace, SFMono-Regular, Consolas, monospace; text-overflow: ellipsis; white-space: pre; }
      #${ROOT_ID} mark { border-radius: 2px; background: var(--ws-mark-bg) !important; color: var(--ws-mark-text) !important; }
      #${ROOT_ID} .ws-preview-head { position: sticky; top: 0; z-index: 1; display: flex; align-items: center; gap: 8px; min-height: 42px; padding: 7px 10px; border-bottom: 1px solid var(--ws-border-soft); background: var(--ws-bg); color: var(--ws-text); }
      #${ROOT_ID} .ws-preview-head span { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      #${ROOT_ID} .ws-preview-code { min-width: max-content; padding: 8px 0; }
      #${ROOT_ID} .ws-preview-line { display: grid; grid-template-columns: 58px minmax(0, 1fr); min-height: 23px; padding: 2px 12px 2px 0; font: 12px/1.55 ui-monospace, SFMono-Regular, Consolas, monospace; }
      #${ROOT_ID} .ws-preview-line.is-target { background: var(--ws-target-bg); }
      #${ROOT_ID} .ws-preview-line span:first-child { padding-right: 12px; color: var(--ws-muted); text-align: right; user-select: none; }
      #${ROOT_ID} .ws-preview-line code { color: var(--ws-text); white-space: pre; }
      .${BUTTON_CLASS} { -webkit-app-region: no-drag; }
      .${BUTTON_CLASS}[data-native-menu="false"] { height: 24px; display: inline-flex; align-items: center; padding: 4px 10px; border: 1px solid transparent; border-radius: 10px; background: transparent; color: color-mix(in srgb, currentColor 50%, transparent); font: 400 14px/14px -apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI", sans-serif; cursor: pointer; }
      .${BUTTON_CLASS}[data-native-menu="false"]:hover { background: color-mix(in srgb, currentColor 5%, transparent); color: color-mix(in srgb, currentColor 72%, transparent); }
      .${BUTTON_CLASS}[data-native-menu="false"]:focus-visible { outline: 2px solid var(--color-token-border-focus, currentColor); outline-offset: -2px; }
      @media (max-width: 720px) { #${ROOT_ID} .ws-dialog { width: 100vw; height: 100vh; border: 0; border-radius: 0; } #${ROOT_ID} .ws-body { grid-template-columns: 1fr; } #${ROOT_ID} .ws-preview { display: none; } #${ROOT_ID} .ws-results { border-right: 0; } #${ROOT_ID} .ws-filter-row { grid-template-columns: 1fr; } }
    `;
    document.head.appendChild(style);
  }

  function createRoot() {
    let root = document.getElementById(ROOT_ID);
    if (root) return root;
    ensureStyles();
    root = document.createElement("div");
    root.id = ROOT_ID;
    root.hidden = true;
    root.setAttribute("role", "dialog");
    root.setAttribute("aria-modal", "true");
    root.setAttribute("aria-label", "工作区全文搜索");
    root.innerHTML = `
      <div class="ws-dialog">
        <div class="ws-head"><strong>工作区全文搜索</strong><small data-ws-status>Ctrl+Shift+F</small><span class="ws-spacer"></span><button class="ws-action-button" data-ws-stop type="button" hidden>停止</button><button class="ws-icon-button" data-ws-close type="button" title="关闭" aria-label="关闭">×</button></div>
        <div class="ws-form">
          <select data-ws-project aria-label="搜索项目"><option>正在识别当前项目</option></select>
          <div class="ws-query-row"><input data-ws-query aria-label="搜索内容" placeholder="搜索" spellcheck="false"><button class="ws-option" data-ws-case type="button" aria-pressed="false" title="区分大小写">Aa</button><button class="ws-option" data-ws-word type="button" aria-pressed="false" title="全字匹配">ab</button><button class="ws-option" data-ws-regex type="button" aria-pressed="false" title="使用正则表达式">.*</button></div>
          <div class="ws-filter-row"><input data-ws-include aria-label="包含文件" placeholder="包含文件，例如 src/**, *.rs" spellcheck="false"><input data-ws-exclude aria-label="排除文件" placeholder="排除文件，例如 target/**, *.lock" spellcheck="false"></div>
        </div>
        <div class="ws-body"><div class="ws-results" data-ws-results><div class="ws-empty">输入内容以搜索当前工作区</div></div><div class="ws-preview" data-ws-preview><div class="ws-empty">选择搜索结果以预览</div></div></div>
      </div>`;
    document.body.appendChild(root);
    state.root = root;
    state.query = root.querySelector("[data-ws-query]");
    state.include = root.querySelector("[data-ws-include]");
    state.exclude = root.querySelector("[data-ws-exclude]");
    state.caseSensitive = root.querySelector("[data-ws-case]");
    state.wholeWord = root.querySelector("[data-ws-word]");
    state.regex = root.querySelector("[data-ws-regex]");
    state.status = root.querySelector("[data-ws-status]");
    state.stop = root.querySelector("[data-ws-stop]");
    state.results = root.querySelector("[data-ws-results]");
    state.preview = root.querySelector("[data-ws-preview]");
    state.projectSelect = root.querySelector("[data-ws-project]");
    syncTheme();

    root.querySelector("[data-ws-close]").addEventListener("click", close);
    state.stop.addEventListener("click", stopSearch);
    root.addEventListener("mousedown", (event) => { if (event.target === root) close(); });
    [state.query, state.include, state.exclude].forEach((input) => input.addEventListener("input", scheduleSearch));
    state.projectSelect.addEventListener("change", async () => {
      state.selectionMode = "manual";
      await setSelectedProject(state.projectSelect.value, { resetOnChange: true });
      if (state.query?.value.trim()) startSearch();
    });
    state.query.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        startSearch();
      }
    });
    [state.caseSensitive, state.wholeWord, state.regex].forEach((button) => button.addEventListener("click", () => {
      button.setAttribute("aria-pressed", button.getAttribute("aria-pressed") !== "true" ? "true" : "false");
      startSearch();
    }));
    return root;
  }

  function setStatus(message) {
    if (state.status) state.status.textContent = message;
  }

  function setSearching(active) {
    if (state.stop) state.stop.hidden = !active;
  }

  function parseGlobs(value) {
    return String(value || "").split(",").map((item) => item.trim()).filter(Boolean).slice(0, 32);
  }

  async function cancelActiveSearch() {
    window.clearTimeout(state.pollTimer);
    state.pollTimer = 0;
    const searchId = state.searchId;
    state.searchId = "";
    if (searchId) await bridgeCall("/workspace-search/cancel", { searchId });
    setSearching(false);
  }

  async function stopSearch() {
    state.searchRevision += 1;
    await cancelActiveSearch();
    setStatus("已取消");
    renderEmpty("搜索已取消");
  }

  function scheduleSearch() {
    window.clearTimeout(state.debounceTimer);
    state.debounceTimer = window.setTimeout(startSearch, SEARCH_DEBOUNCE_MS);
  }

  async function startSearch() {
    window.clearTimeout(state.debounceTimer);
    const root = await syncCurrentWorkspace({ resetOnChange: true });
    const query = state.query?.value.trim() || "";
    const revision = ++state.searchRevision;
    await cancelActiveSearch();
    if (revision !== state.searchRevision) return;
    if (!query) {
      renderEmpty("输入内容以搜索当前工作区");
      setStatus("Ctrl+Shift+F");
      return;
    }
    if (!root) {
      renderEmpty("所选项目未关联可访问的工作区");
      setStatus("所选项目不可搜索");
      return;
    }
    setStatus("正在搜索…");
    setSearching(true);
    renderEmpty("正在搜索…");
    const response = await bridgeCall("/workspace-search/start", {
      root,
      query,
      caseSensitive: state.caseSensitive?.getAttribute("aria-pressed") === "true",
      wholeWord: state.wholeWord?.getAttribute("aria-pressed") === "true",
      regex: state.regex?.getAttribute("aria-pressed") === "true",
      include: parseGlobs(state.include?.value),
      exclude: parseGlobs(state.exclude?.value),
      maxResults: 2000,
    });
    if (revision !== state.searchRevision) return;
    if (response?.status !== "ok" || !response.searchId) {
      renderEmpty(response?.message || "搜索启动失败");
      setStatus("搜索失败");
      setSearching(false);
      return;
    }
    state.searchId = response.searchId;
    pollSearch(response.searchId, revision);
  }

  async function pollSearch(searchId, revision) {
    if (state.searchId !== searchId || state.searchRevision !== revision) return;
    const response = await bridgeCall("/workspace-search/poll", { searchId });
    if (state.searchId !== searchId || state.searchRevision !== revision) return;
    if (response?.status === "running") {
      state.pollTimer = window.setTimeout(() => pollSearch(searchId, revision), POLL_INTERVAL_MS);
      return;
    }
    state.searchId = "";
    setSearching(false);
    if (response?.status !== "ok") {
      if (response?.status !== "cancelled") renderEmpty(response?.message || "搜索失败");
      setStatus(response?.status === "cancelled" ? "已取消" : "搜索失败");
      return;
    }
    renderResults(response.results || [], response.root || "");
    const suffix = response.truncated ? "，结果已截断" : "";
    setStatus(`${response.results?.length || 0} 个结果，${response.elapsedMs || 0} ms${suffix}`);
  }

  function renderEmpty(message) {
    state.results.replaceChildren();
    const empty = document.createElement("div");
    empty.className = "ws-empty";
    empty.textContent = message;
    state.results.appendChild(empty);
  }

  function appendHighlightedText(target, text, ranges) {
    const characters = Array.from(String(text || ""));
    let cursor = 0;
    for (const range of Array.isArray(ranges) ? ranges : []) {
      const start = Math.max(cursor, Math.min(characters.length, Number(range.start) || 0));
      const end = Math.max(start, Math.min(characters.length, Number(range.end) || start));
      if (start > cursor) target.appendChild(document.createTextNode(characters.slice(cursor, start).join("")));
      const mark = document.createElement("mark");
      mark.textContent = characters.slice(start, end).join("");
      target.appendChild(mark);
      cursor = end;
    }
    if (cursor < characters.length) target.appendChild(document.createTextNode(characters.slice(cursor).join("")));
  }

  function renderResults(results, root) {
    state.results.replaceChildren();
    state.preview.innerHTML = '<div class="ws-empty">选择搜索结果以预览</div>';
    if (!results.length) {
      renderEmpty("未找到匹配内容");
      return;
    }
    const groups = new Map();
    for (const result of results) {
      const key = String(result.relativePath || result.path || "");
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(result);
    }
    for (const [path, matches] of groups) {
      const section = document.createElement("section");
      section.className = "ws-file";
      const head = document.createElement("button");
      head.className = "ws-file-head";
      head.type = "button";
      head.title = path;
      const label = document.createElement("strong");
      label.textContent = path;
      const count = document.createElement("span");
      count.className = "ws-count";
      count.textContent = String(matches.length);
      head.append(label, count);
      const body = document.createElement("div");
      head.addEventListener("click", () => { body.hidden = !body.hidden; });
      for (const match of matches) {
        const button = document.createElement("button");
        button.className = "ws-match";
        button.type = "button";
        const number = document.createElement("span");
        number.className = "ws-line-number";
        number.textContent = String(match.line || "");
        const text = document.createElement("span");
        text.className = "ws-line-text";
        appendHighlightedText(text, match.text, match.ranges);
        button.append(number, text);
        button.addEventListener("click", () => {
          state.results.querySelectorAll(".ws-match.is-selected").forEach((node) => node.classList.remove("is-selected"));
          button.classList.add("is-selected");
          loadPreview(root, match);
        });
        button.addEventListener("dblclick", () => openFile(match));
        body.appendChild(button);
      }
      section.append(head, body);
      state.results.appendChild(section);
    }
  }

  async function loadPreview(root, match) {
    state.preview.innerHTML = '<div class="ws-empty">正在载入预览…</div>';
    const response = await bridgeCall("/workspace-search/preview", { root, path: match.path, line: match.line });
    if (response?.status !== "ok") {
      state.preview.innerHTML = "";
      const empty = document.createElement("div");
      empty.className = "ws-empty";
      empty.textContent = response?.message || "无法预览文件";
      state.preview.appendChild(empty);
      return;
    }
    state.preview.replaceChildren();
    const head = document.createElement("div");
    head.className = "ws-preview-head";
    const label = document.createElement("span");
    label.textContent = `${response.path}:${match.line}`;
    label.title = response.path;
    const open = document.createElement("button");
    open.className = "ws-icon-button";
    open.type = "button";
    open.textContent = "↗";
    open.title = "在 Codex 中打开";
    open.setAttribute("aria-label", "在 Codex 中打开");
    open.addEventListener("click", () => openFile(match));
    head.append(label, open);
    const code = document.createElement("div");
    code.className = "ws-preview-code";
    for (const item of response.lines || []) {
      const row = document.createElement("div");
      row.className = `ws-preview-line${Number(item.number) === Number(match.line) ? " is-target" : ""}`;
      const number = document.createElement("span");
      number.textContent = String(item.number);
      const text = document.createElement("code");
      text.textContent = String(item.text || "");
      row.append(number, text);
      code.appendChild(row);
    }
    state.preview.append(head, code);
  }

  function callCodexApi(method, params) {
    return new Promise((resolve, reject) => {
      const requestId = typeof crypto?.randomUUID === "function" ? crypto.randomUUID() : `workspace-search-${Date.now()}-${Math.random()}`;
      const cleanup = () => {
        window.clearTimeout(timeout);
        window.removeEventListener("message", onMessage);
      };
      const onMessage = (event) => {
        const message = event?.data;
        if (!message || message.type !== "fetch-response" || message.requestId !== requestId) return;
        cleanup();
        if (message.responseType !== "success") reject(new Error(message.error || "Codex open-file failed"));
        else resolve(message.bodyJsonString || "");
      };
      window.addEventListener("message", onMessage);
      const timeout = window.setTimeout(() => { cleanup(); reject(new Error("Codex open-file timed out")); }, 2500);
      Promise.resolve(window.electronBridge.sendMessageFromView({
        type: "fetch",
        requestId,
        method: "POST",
        url: `vscode://codex/${method}`,
        body: JSON.stringify(params),
      })).catch((error) => { cleanup(); reject(error); });
    });
  }

  async function openFile(match) {
    try {
      await callCodexApi("open-file", {
        path: match.path,
        filePath: match.path,
        line: Number(match.line) || 1,
        column: Number(match.column) || 1,
      });
      setStatus(`已打开 ${match.relativePath}:${match.line}`);
    } catch (_) {
      setStatus("当前 Codex 版本不支持直接跳转，已保留文件预览");
    }
  }

  async function syncCurrentWorkspace({ resetOnChange = false } = {}) {
    const context = await loadProjects();
    const manuallySelected = state.selectionMode === "manual"
      && Boolean(projectById(state.selectedProjectId));
    const selectedProjectId = manuallySelected
      ? state.selectedProjectId
      : projectById(context.currentProjectId)?.id
        || projectById(context.selectedProjectId)?.id
        || state.projects.find((project) => project.root)?.id
        || "";
    return setSelectedProject(selectedProjectId, { resetOnChange });
  }

  async function open() {
    const root = createRoot();
    state.previousFocus = document.activeElement;
    root.hidden = false;
    state.selectionMode = "auto";
    syncTheme();
    await syncCurrentWorkspace({ resetOnChange: true });
    window.setTimeout(() => state.query?.focus(), 0);
  }

  function close() {
    if (!state.root || state.root.hidden) return;
    state.root.hidden = true;
    state.searchRevision += 1;
    cancelActiveSearch();
    if (state.previousFocus instanceof HTMLElement && state.previousFocus.isConnected) state.previousFocus.focus();
  }

  function toggle() {
    if (state.root && !state.root.hidden) close();
    else open();
  }

  function ensureHeaderButton() {
    ensureStyles();
    const header = document.querySelector('[class*="ApplicationMenuTopBar"], .app-header-tint, header');
    if (!header) return;
    const nativeMenuClass = [...header.querySelectorAll("button")]
      .filter((candidate) => !candidate.classList.contains(BUTTON_CLASS))
      .find((candidate) => /^(文件|编辑|视图|帮助|file|edit|view|help)$/i.test(candidate.textContent?.trim() || ""))
      ?.className;
    const existing = document.querySelector(`.${BUTTON_CLASS}`);
    if (existing) {
      existing.className = nativeMenuClass ? `${nativeMenuClass} ${BUTTON_CLASS}` : BUTTON_CLASS;
      existing.dataset.nativeMenu = String(Boolean(nativeMenuClass));
      return;
    }
    const button = document.createElement("button");
    button.className = nativeMenuClass ? `${nativeMenuClass} ${BUTTON_CLASS}` : BUTTON_CLASS;
    button.dataset.nativeMenu = String(Boolean(nativeMenuClass));
    button.type = "button";
    button.textContent = "搜索";
    button.title = "工作区全文搜索 (Ctrl+Shift+F)";
    button.addEventListener("click", open);
    header.appendChild(button);
  }

  function refreshChromeContext() {
    ensureHeaderButton();
    syncTheme();
    if (!state.root || state.root.hidden) return;
    window.clearTimeout(state.contextTimer);
    state.contextTimer = window.setTimeout(() => syncCurrentWorkspace({ resetOnChange: true }), 80);
  }

  function onKeyDown(event) {
    if (event.key.toLowerCase() === "f" && event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey) {
      event.preventDefault();
      event.stopImmediatePropagation();
      toggle();
      return;
    }
    if (event.key === "Escape" && state.root && !state.root.hidden) {
      event.preventDefault();
      close();
    }
  }

  function cleanup() {
    document.removeEventListener("keydown", onKeyDown, true);
    state.observer?.disconnect();
    window.clearTimeout(state.debounceTimer);
    window.clearTimeout(state.contextTimer);
    window.clearTimeout(state.pollTimer);
    cancelActiveSearch();
    document.getElementById(ROOT_ID)?.remove();
    document.getElementById(STYLE_ID)?.remove();
    document.querySelectorAll(`.${BUTTON_CLASS}`).forEach((node) => node.remove());
    delete window[API_KEY];
  }

  document.addEventListener("keydown", onKeyDown, true);
  state.observer = new MutationObserver(refreshChromeContext);
  state.observer.observe(document.documentElement, { childList: true, subtree: true });
  ensureHeaderButton();
  window[API_KEY] = { open, close, toggle, cleanup };
})();
