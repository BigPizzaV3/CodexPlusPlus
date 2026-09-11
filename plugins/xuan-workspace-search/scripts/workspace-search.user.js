(() => {
  const globalKey = "__xuanWorkspaceSearchUi";
  const buttonMarker = "data-xuan-workspace-search-button";
  const panelMarker = "data-xuan-workspace-search-panel";
  const styleMarker = "data-xuan-workspace-search-style";
  const bridgeUrl = window.__XUAN_BRIDGE_URL__ || "http://127.0.0.1:57324";
  const bridgeToken = window.__XUAN_BRIDGE_TOKEN__ || "";

  window[globalKey]?.destroy?.();

  const state = {
    button: null,
    panel: null,
    input: null,
    scope: null,
    status: null,
    results: null,
    searchButton: null,
    options: {},
    root: "",
    busy: false,
    activePreview: null,
  };

  const createElement = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  };

  const ensureStyles = () => {
    if (document.querySelector(`[${styleMarker}]`)) return;
    const style = document.createElement("style");
    style.setAttribute(styleMarker, "true");
    style.textContent = `
      [${panelMarker}] { position: fixed; z-index: 2147483000; overflow: hidden; border: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); border-radius: 8px; background: var(--color-background-elevated-primary-opaque, #fff); color: var(--color-text-secondary-solid, #222); box-shadow: var(--shadow-2xl, 0 16px 32px rgba(0,0,0,.18)); font-family: var(--font-sans-default, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif); font-size: 14px; }
      [${panelMarker}] *, [${panelMarker}] *::before, [${panelMarker}] *::after { box-sizing: border-box; letter-spacing: 0; }
      [${panelMarker}][hidden] { display: none !important; }
      .xuan-search-header { display: flex; min-width: 0; align-items: center; gap: 8px; padding: 10px 12px 8px; border-bottom: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); }
      .xuan-search-heading { min-width: 0; flex: 1; }
      .xuan-search-title { font-size: 14px; font-weight: 600; line-height: 20px; }
      .xuan-search-scope { overflow: hidden; color: var(--color-text-tertiary, #777); font-size: 12px; line-height: 16px; text-overflow: ellipsis; white-space: nowrap; }
      .xuan-search-close, .xuan-search-result, .xuan-search-action { font: inherit; color: inherit; }
      .xuan-search-close { display: inline-flex; width: 28px; height: 28px; flex: 0 0 28px; align-items: center; justify-content: center; border: 0; border-radius: 8px; background: transparent; color: var(--color-text-tertiary, #777); cursor: pointer; }
      .xuan-search-close:hover, .xuan-search-close:focus-visible, .xuan-search-result:hover, .xuan-search-result:focus-visible { background: var(--color-background-primary-ghost-focus, rgba(0,0,0,.06)); color: inherit; outline: none; }
      .xuan-search-controls { padding: 10px 12px 8px; }
      .xuan-search-input-row { display: flex; gap: 8px; }
      .xuan-search-input { width: 100%; min-width: 0; height: 32px; border: 1px solid var(--color-token-input-border, rgba(0,0,0,.18)); border-radius: 7px; outline: none; background: var(--color-background-secondary-soft, rgba(0,0,0,.04)); color: inherit; padding: 0 10px; font: inherit; }
      .xuan-search-input:focus { border-color: var(--color-text-accent, #1677d2); box-shadow: 0 0 0 1px var(--color-text-accent, #1677d2); }
      .xuan-search-action { height: 32px; flex: 0 0 auto; border: 1px solid var(--color-token-border-heavy, rgba(0,0,0,.18)); border-radius: 7px; background: var(--color-background-primary-solid, #222); color: var(--color-text-inverse, #fff); padding: 0 12px; cursor: pointer; }
      .xuan-search-action:disabled { cursor: default; opacity: .55; }
      .xuan-search-options { display: flex; flex-wrap: wrap; gap: 6px 14px; padding-top: 8px; color: var(--color-text-tertiary, #777); font-size: 12px; }
      .xuan-search-option { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
      .xuan-search-option input { width: 14px; height: 14px; margin: 0; accent-color: var(--color-background-info-solid, #1677d2); }
      .xuan-search-status { min-height: 29px; padding: 6px 12px 7px; border-top: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); color: var(--color-text-tertiary, #777); font-size: 12px; line-height: 16px; }
      .xuan-search-status[data-kind="error"] { color: var(--color-text-error, #c33); }
      .xuan-search-results { overflow: auto; border-top: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); }
      .xuan-search-empty { padding: 22px 12px; color: var(--color-text-tertiary, #777); text-align: center; }
      .xuan-search-result-row { border-bottom: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); }
      .xuan-search-result-row:last-child { border-bottom: 0; }
      .xuan-search-result { display: block; width: 100%; border: 0; border-radius: 0; background: transparent; padding: 8px 12px; text-align: left; cursor: pointer; }
      .xuan-search-path { overflow: hidden; color: var(--color-text-secondary-solid, #444); font-size: 12px; font-weight: 600; line-height: 17px; text-overflow: ellipsis; white-space: nowrap; }
      .xuan-search-snippet { overflow: hidden; margin-top: 2px; color: var(--color-text-tertiary, #777); font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; line-height: 18px; text-overflow: ellipsis; white-space: nowrap; }
      .xuan-search-snippet mark { border-radius: 2px; background: var(--color-background-info-soft, rgba(51,156,255,.22)); color: inherit; }
      .xuan-search-preview { overflow: auto; max-height: 190px; padding: 7px 12px 9px; background: var(--color-background-elevated-secondary, rgba(0,0,0,.035)); color: var(--color-text-tertiary, #777); font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; line-height: 18px; }
      .xuan-search-preview-line { display: grid; grid-template-columns: 42px minmax(0, 1fr); gap: 10px; min-width: max-content; }
      .xuan-search-preview-line[data-active="true"] { color: var(--color-text-secondary-solid, #333); background: var(--color-background-info-soft, rgba(51,156,255,.18)); }
      .xuan-search-line-number { color: var(--color-text-tertiary, #888); text-align: right; user-select: none; }
    `;
    document.head.appendChild(style);
  };

  const helpButton = () => document.querySelector("button[aria-label='帮助'], button[aria-label='Help']");

  const isAbsolutePath = (value) => {
    const text = typeof value === "string" ? value.trim() : "";
    return /^(?:[A-Za-z]:[\\/]|\\\\[^\\]+\\[^\\]+|\/[^/])/.test(text)
      && !/[\\/]\.codex[\\/]?$/.test(text);
  };

  const directWorkspaceRoot = (source) => {
    if (!source || typeof source !== "object") return "";
    for (const key of ["cwd", "workspaceRoot", "rootPath", "workingDirectory", "workingDir", "displayCwd", "envTooltip"]) {
      const value = source[key];
      if (isAbsolutePath(value)) return value.trim();
    }
    return "";
  };

  const workspaceRootFromElement = (element) => {
    if (!element) return "";
    const roots = Object.keys(element)
      .filter((key) => key.startsWith("__reactFiber") || key.startsWith("__reactProps"))
      .map((key) => element[key]);
    const visited = new WeakSet();
    const stack = roots.map((value) => ({ value, depth: 0 }));
    let scanned = 0;
    while (stack.length && scanned < 1_200) {
      const { value, depth } = stack.pop();
      if (!value || typeof value !== "object" || visited.has(value) || depth > 12) continue;
      visited.add(value);
      scanned += 1;
      const direct = directWorkspaceRoot(value);
      if (direct) return direct;
      if (value instanceof Element) continue;
      for (const key of Object.keys(value).slice(0, 120)) {
        if (["ownerDocument", "parentElement", "parentNode", "children", "childNodes"].includes(key)) continue;
        let child;
        try {
          child = value[key];
        } catch {
          continue;
        }
        if (child && typeof child === "object") stack.push({ value: child, depth: depth + 1 });
      }
    }
    return "";
  };

  const resolveWorkspaceRoot = () => {
    const activeRow = document.querySelector(
      '[data-app-action-sidebar-thread-active="true"], [data-app-action-sidebar-thread-selected="true"], [aria-current="page"][data-app-action-sidebar-thread-row]'
    );
    const fromRow = workspaceRootFromElement(activeRow);
    if (fromRow) return fromRow;
    const projectButton = document.querySelector("button[aria-label^='项目：'], button[aria-label^='Project:']");
    return workspaceRootFromElement(projectButton);
  };

  const workspaceName = (root) => root.split(/[\\/]/).filter(Boolean).pop() || root;

  const bridgeRequest = async (path, payload) => {
    let body;
    if (typeof window.__codexSessionDeleteBridge === "function") {
      body = await window.__codexSessionDeleteBridge(path, payload);
    } else {
      const headers = { "content-type": "application/json" };
      if (bridgeToken) headers["x-xuan-bridge-token"] = bridgeToken;
      const response = await fetch(`${bridgeUrl}${path}`, {
        method: "POST",
        headers,
        body: JSON.stringify(payload),
      });
      body = await response.json().catch(() => null);
      if (!response.ok) body = body || { status: "failed" };
    }
    if (body?.status === "failed" || body?.error) {
      const error = new Error("bridge request failed");
      error.code = body?.error?.code || "request_failed";
      throw error;
    }
    return body;
  };

  const messageForError = (error) => {
    const messages = {
      dependency_missing: "当前环境缺少文件搜索组件。",
      permission_denied: "当前工作区不在允许搜索的范围内。",
      timeout: "搜索用时过长，请缩小关键词范围后重试。",
      invalid_request: "搜索条件无效，请调整后重试。",
      not_found: "当前工作区已不可用。",
    };
    return messages[error?.code] || "暂时无法搜索当前工作区。";
  };

  const setStatus = (message, kind = "normal") => {
    if (!state.status) return;
    state.status.textContent = message;
    state.status.dataset.kind = kind;
  };

  const appendHighlightedText = (node, text, ranges) => {
    const normalized = Array.isArray(ranges)
      ? ranges.map((range) => ({
        start: Math.max(0, Number(range?.start) || 0),
        end: Math.min(text.length, Math.max(0, Number(range?.end) || 0)),
      })).filter((range) => range.end > range.start).sort((left, right) => left.start - right.start)
      : [];
    let cursor = 0;
    normalized.forEach((range) => {
      if (range.start > cursor) node.append(document.createTextNode(text.slice(cursor, range.start)));
      const mark = document.createElement("mark");
      mark.textContent = text.slice(range.start, range.end);
      node.append(mark);
      cursor = Math.max(cursor, range.end);
    });
    if (cursor < text.length) node.append(document.createTextNode(text.slice(cursor)));
  };

  const renderPreview = async (container, item) => {
    if (state.activePreview && state.activePreview !== container) {
      state.activePreview.hidden = true;
      state.activePreview.replaceChildren();
    }
    if (!container.hidden) {
      container.hidden = true;
      container.replaceChildren();
      state.activePreview = null;
      return;
    }
    state.activePreview = container;
    container.hidden = false;
    container.textContent = "正在加载预览…";
    try {
      const payload = await bridgeRequest("/v1/search/preview", {
        root: state.root,
        path: item.path,
        line: item.line,
      });
      container.replaceChildren();
      for (const line of payload?.lines || []) {
        const row = createElement("div", "xuan-search-preview-line");
        if (Number(line.number) === Number(item.line)) row.dataset.active = "true";
        row.append(
          createElement("span", "xuan-search-line-number", String(line.number || "")),
          createElement("span", "", String(line.text || ""))
        );
        container.append(row);
      }
    } catch (error) {
      container.textContent = messageForError(error);
    }
  };

  const renderResults = (items) => {
    state.results.replaceChildren();
    state.activePreview = null;
    if (!items.length) {
      state.results.append(createElement("div", "xuan-search-empty", "没有找到匹配内容"));
      return;
    }
    for (const item of items) {
      const row = createElement("div", "xuan-search-result-row");
      const button = createElement("button", "xuan-search-result");
      button.type = "button";
      button.setAttribute("aria-label", `${item.relativePath || item.path}，第 ${item.line} 行`);
      const path = createElement("div", "xuan-search-path", `${item.relativePath || item.path}:${item.line}`);
      const snippet = createElement("div", "xuan-search-snippet");
      appendHighlightedText(snippet, String(item.text || ""), item.ranges);
      const preview = createElement("div", "xuan-search-preview");
      preview.hidden = true;
      button.append(path, snippet);
      button.addEventListener("click", () => renderPreview(preview, item));
      row.append(button, preview);
      state.results.append(row);
    }
  };

  const runSearch = async () => {
    if (state.busy) return;
    const query = state.input?.value.trim() || "";
    if (!query) {
      setStatus("请输入搜索内容。", "error");
      state.input?.focus();
      return;
    }
    const root = resolveWorkspaceRoot();
    if (!root) {
      setStatus("无法识别当前工作区，请先打开一个项目任务。", "error");
      return;
    }
    state.root = root;
    state.scope.textContent = `范围：${workspaceName(root)}`;
    state.scope.title = root;
    state.busy = true;
    state.searchButton.disabled = true;
    state.results.replaceChildren();
    setStatus("正在搜索…");
    try {
      const payload = await bridgeRequest("/v1/search/start", {
        root,
        query,
        caseSensitive: state.options.caseSensitive.checked,
        wholeWord: state.options.wholeWord.checked,
        regex: state.options.regex.checked,
        maxResults: 200,
      });
      const result = payload?.result || {};
      const items = Array.isArray(result.results) ? result.results : [];
      renderResults(items);
      const elapsed = Number(result.elapsedMs);
      const duration = Number.isFinite(elapsed) ? `，${elapsed} 毫秒` : "";
      const suffix = result.truncated ? "，结果已截断" : "";
      setStatus(`找到 ${items.length} 项${duration}${suffix}`);
    } catch (error) {
      renderResults([]);
      setStatus(messageForError(error), "error");
    } finally {
      state.busy = false;
      state.searchButton.disabled = false;
    }
  };

  const optionControl = (key, label) => {
    const wrapper = createElement("label", "xuan-search-option");
    const input = document.createElement("input");
    input.type = "checkbox";
    wrapper.append(input, document.createTextNode(label));
    state.options[key] = input;
    return wrapper;
  };

  const ensurePanel = () => {
    if (state.panel?.isConnected) return state.panel;
    ensureStyles();
    const panel = createElement("section");
    panel.id = "xuan-workspace-search-panel";
    panel.setAttribute(panelMarker, "true");
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-label", "工作区搜索");
    panel.hidden = true;

    const header = createElement("div", "xuan-search-header");
    const heading = createElement("div", "xuan-search-heading");
    const title = createElement("div", "xuan-search-title", "搜索");
    const scope = createElement("div", "xuan-search-scope", "当前工作区");
    const close = createElement("button", "xuan-search-close", "×");
    close.type = "button";
    close.title = "关闭搜索";
    close.setAttribute("aria-label", "关闭搜索");
    close.addEventListener("click", () => setOpen(false));
    heading.append(title, scope);
    header.append(heading, close);

    const controls = createElement("div", "xuan-search-controls");
    const inputRow = createElement("div", "xuan-search-input-row");
    const input = createElement("input", "xuan-search-input");
    input.type = "search";
    input.placeholder = "搜索文件内容";
    input.setAttribute("aria-label", "搜索文件内容");
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.isComposing) runSearch();
    });
    const searchButton = createElement("button", "xuan-search-action", "搜索");
    searchButton.type = "button";
    searchButton.addEventListener("click", runSearch);
    inputRow.append(input, searchButton);
    const options = createElement("div", "xuan-search-options");
    options.append(
      optionControl("caseSensitive", "区分大小写"),
      optionControl("wholeWord", "全字匹配"),
      optionControl("regex", "正则表达式")
    );
    controls.append(inputRow, options);

    const status = createElement("div", "xuan-search-status", "输入内容后按回车搜索");
    status.setAttribute("aria-live", "polite");
    const results = createElement("div", "xuan-search-results");
    results.append(createElement("div", "xuan-search-empty", "暂无搜索结果"));
    panel.append(header, controls, status, results);
    document.body.append(panel);

    state.panel = panel;
    state.input = input;
    state.scope = scope;
    state.status = status;
    state.results = results;
    state.searchButton = searchButton;
    return panel;
  };

  const positionPanel = () => {
    if (!state.button || !state.panel || state.panel.hidden) return;
    const rect = state.button.getBoundingClientRect();
    const width = Math.min(620, Math.max(300, window.innerWidth - 16));
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    const top = rect.bottom + 6;
    state.panel.style.width = `${width}px`;
    state.panel.style.left = `${left}px`;
    state.panel.style.top = `${top}px`;
    state.panel.style.maxHeight = `${Math.max(220, window.innerHeight - top - 10)}px`;
    state.results.style.maxHeight = `${Math.max(150, window.innerHeight - top - 190)}px`;
  };

  const setOpen = (open) => {
    const panel = ensurePanel();
    panel.hidden = !open;
    state.button?.setAttribute("aria-expanded", String(open));
    if (!open) return;
    const root = resolveWorkspaceRoot();
    state.root = root;
    state.scope.textContent = root ? `范围：${workspaceName(root)}` : "未识别到工作区";
    state.scope.title = root;
    positionPanel();
    window.setTimeout(() => state.input?.focus(), 0);
  };

  const mountButton = () => {
    const help = helpButton();
    if (!help?.parentElement) return;
    let button = document.querySelector(`[${buttonMarker}]`);
    if (!button) {
      button = document.createElement("button");
      button.type = "button";
      button.setAttribute(buttonMarker, "true");
      button.textContent = "搜索";
      button.setAttribute("aria-label", "搜索");
      button.setAttribute("aria-haspopup", "dialog");
      button.setAttribute("aria-controls", "xuan-workspace-search-panel");
      button.addEventListener("click", (event) => {
        event.stopPropagation();
        setOpen(state.panel?.hidden !== false);
      });
    }
    button.className = help.className;
    state.button = button;
    if (help.previousElementSibling !== button) help.insertAdjacentElement("beforebegin", button);
  };

  const onPointerDown = (event) => {
    if (!state.panel || state.panel.hidden) return;
    if (state.panel.contains(event.target) || state.button?.contains(event.target)) return;
    setOpen(false);
  };

  const onKeyDown = (event) => {
    if (event.key !== "Escape" || !state.panel || state.panel.hidden) return;
    setOpen(false);
    state.button?.focus();
  };

  const observer = new MutationObserver(mountButton);
  observer.observe(document.documentElement, { childList: true, subtree: true });
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("resize", positionPanel);
  mountButton();

  window[globalKey] = {
    destroy() {
      observer.disconnect();
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", positionPanel);
      document.querySelector(`[${buttonMarker}]`)?.remove();
      document.querySelector(`[${panelMarker}]`)?.remove();
      document.querySelector(`[${styleMarker}]`)?.remove();
    },
  };
})();
