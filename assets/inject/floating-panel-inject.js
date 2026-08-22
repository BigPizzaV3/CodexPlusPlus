(() => {
  "use strict";

  const API_KEY = "__codexFloatingPanel";
  const CORE_KEY = "__codexStepwisePanel";
  const OUTLINE_KEY = "__codexAnswerOutline";
  const VISUAL_KEY = "__codexFloatingPanelVisual";
  const BRIDGE_KEY = "__codexSessionDeleteBridge";
  const STYLE_ID = "codex-floating-panel-structure-style";
  const ROOT_ATTR = "data-codex-floating-panel-root";
  const LEGACY_ROOT_ATTR = "data-codex-stepwise-root";
  const POSITION_KEY = "codex-floating-panel-position-v2";
  const SIZE_KEY = "codex-floating-panel-size-v2";
  const FONT_OFFSET_KEY = "codex-floating-panel-font-offset-v1";
  const CLICK_MODE_KEY = "codex-floating-panel-click-mode-v1";
  const MIN_WIDTH = 300;
  const MAX_WIDTH = 640;
  const MIN_HEIGHT = 340;
  const MAX_HEIGHT = 720;
  const COLLAPSED_WIDTH = 92;
  const COLLAPSED_HEIGHT = 46;
  const FACE_RADIUS = 23;
  const SAFE_MARGIN = 12;
  const HORIZONTAL_MS = 260;
  const VERTICAL_MS = 340;
  const VIEW_MS = 180;
  const SETTINGS_POLL_MS = 1800;

  const previous = window[API_KEY];
  if (previous && typeof previous.destroy === "function") previous.destroy();
  document.querySelectorAll?.(`[${ROOT_ATTR}="true"]`).forEach((node) => node.remove());
  document.getElementById(STYLE_ID)?.remove();

  const state = {
    root: null,
    shell: null,
    header: null,
    body: null,
    activeTab: "next",
    open: false,
    transitioning: false,
    direction: "down",
    anchor: readPosition(),
    size: readSize(),
    fontOffset: readNumber(FONT_OFFSET_KEY, 0),
    clickMode: readClickMode(),
    settings: null,
    outline: emptyOutline(),
    outlineUnsubscribe: null,
    observer: null,
    settingsTimer: 0,
    renderFrame: 0,
    drag: null,
    resize: null,
    destroyed: false,
  };

  function readJson(key, fallback) {
    try {
      const value = JSON.parse(localStorage.getItem(key) || "null");
      return value && typeof value === "object" ? value : fallback;
    } catch {
      return fallback;
    }
  }

  function writeJson(key, value) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {}
  }

  function readNumber(key, fallback) {
    const value = Number(localStorage.getItem(key));
    return Number.isFinite(value) ? value : fallback;
  }

  function writeNumber(key, value) {
    try {
      localStorage.setItem(key, String(value));
    } catch {}
  }

  function readPosition() {
    const stored = readJson(POSITION_KEY, null);
    if (Number.isFinite(stored?.x) && Number.isFinite(stored?.y)) return stored;
    return {
      x: Math.max(SAFE_MARGIN + FACE_RADIUS, window.innerWidth - 82),
      y: Math.max(SAFE_MARGIN + FACE_RADIUS, window.innerHeight * 0.42),
    };
  }

  function readSize() {
    const stored = readJson(SIZE_KEY, null);
    return {
      width: clamp(Number(stored?.width) || 404, MIN_WIDTH, MAX_WIDTH),
      height: clamp(Number(stored?.height) || 420, MIN_HEIGHT, Math.min(MAX_HEIGHT, heightCap())),
    };
  }

  function readClickMode() {
    const value = localStorage.getItem(CLICK_MODE_KEY);
    return ["fill", "direct", "hybrid"].includes(value) ? value : "hybrid";
  }

  function writeClickMode(value) {
    state.clickMode = ["fill", "direct", "hybrid"].includes(value) ? value : "hybrid";
    try {
      localStorage.setItem(CLICK_MODE_KEY, state.clickMode);
    } catch {}
    render();
  }

  function emptyOutline() {
    return { enabled: false, status: "idle", error: "", items: [], messageId: "", sourceHash: "" };
  }

  function clamp(value, min, max) {
    return Math.min(max, Math.max(min, value));
  }

  function escapeHtml(value) {
    return String(value ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  }

  function escapeAttr(value) {
    return escapeHtml(value);
  }

  function bridgeCall(path, payload = {}) {
    if (typeof window[BRIDGE_KEY] !== "function") {
      return Promise.resolve({ status: "failed", error: "page bridge is not installed" });
    }
    return Promise.resolve(window[BRIDGE_KEY](path, payload));
  }

  function coreApi() {
    return window[CORE_KEY] || null;
  }

  function coreState() {
    return coreApi()?.state || null;
  }

  function outlineApi() {
    return window[OUTLINE_KEY] || null;
  }

  function prompts() {
    const values = coreState()?.prompts;
    return Array.isArray(values) ? values.filter((item) => item?.prompt) : [];
  }

  function stepwiseEnabled() {
    return coreState()?.settings?.enabled === true || state.settings?.enabled === true;
  }

  function outlineEnabled() {
    return state.outline.enabled === true
      || coreState()?.settings?.answerOutlineEnabled === true
      || state.settings?.answerOutlineEnabled === true;
  }

  function runtimeEnabled() {
    return stepwiseEnabled() || outlineEnabled();
  }

  function availableTabs() {
    const tabs = [];
    if (stepwiseEnabled()) tabs.push("next");
    if (outlineEnabled()) tabs.push("outline");
    tabs.push("settings");
    return tabs;
  }

  function normalizeTab(value = state.activeTab) {
    const tabs = availableTabs();
    if (tabs.includes(value)) return value;
    return tabs[0] || "settings";
  }

  function hostTheme() {
    const classes = `${document.documentElement.className} ${document.body?.className || ""}`;
    if (/\bdark\b/i.test(classes)) return "dark";
    const background = getComputedStyle(document.body || document.documentElement).backgroundColor;
    const values = background.match(/[\d.]+/g)?.slice(0, 3).map(Number) || [];
    return values.length === 3 && values.reduce((sum, value) => sum + value, 0) < 360 ? "dark" : "light";
  }

  function heightCap() {
    return Math.max(MIN_HEIGHT, window.innerHeight - SAFE_MARGIN * 2);
  }

  function clampAnchor(anchor) {
    return {
      x: clamp(anchor.x, SAFE_MARGIN + FACE_RADIUS, window.innerWidth - SAFE_MARGIN - FACE_RADIUS),
      y: clamp(anchor.y, SAFE_MARGIN + FACE_RADIUS, window.innerHeight - SAFE_MARGIN - FACE_RADIUS),
    };
  }

  function resolveDirection(height = state.size.height) {
    const below = window.innerHeight - state.anchor.y - SAFE_MARGIN;
    const above = state.anchor.y - SAFE_MARGIN;
    return below >= height - FACE_RADIUS || below >= above ? "down" : "up";
  }

  function geometry(open = state.open, width = state.size.width, height = state.size.height) {
    const nextWidth = open ? clamp(width, MIN_WIDTH, MAX_WIDTH) : COLLAPSED_WIDTH;
    const nextHeight = open ? clamp(height, MIN_HEIGHT, heightCap()) : COLLAPSED_HEIGHT;
    const direction = open ? resolveDirection(nextHeight) : state.direction;
    return {
      width: nextWidth,
      height: nextHeight,
      left: state.anchor.x - nextWidth / 2,
      top: direction === "down"
        ? state.anchor.y - FACE_RADIUS
        : state.anchor.y + FACE_RADIUS - nextHeight,
      direction,
    };
  }

  function applyGeometry(frame = geometry()) {
    if (!state.root) return;
    state.direction = frame.direction;
    state.root.dataset.open = String(state.open);
    state.root.dataset.direction = frame.direction;
    state.root.dataset.theme = hostTheme();
    state.root.style.left = `${Math.round(frame.left)}px`;
    state.root.style.top = `${Math.round(frame.top)}px`;
    state.root.style.width = `${Math.round(frame.width)}px`;
    state.root.style.height = `${Math.round(frame.height)}px`;
    state.root.style.setProperty("--cfp-font-offset", `${state.fontOffset}px`);
  }

  function installStyle() {
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      [${ROOT_ATTR}="true"] {
        --cfp-bg: rgba(245,247,250,.92);
        --cfp-border: rgba(24,28,36,.15);
        --cfp-text: rgba(20,24,30,.94);
        --cfp-muted: rgba(20,24,30,.58);
        --cfp-row: rgba(255,255,255,.5);
        --cfp-accent: #3878ff;
        box-sizing: border-box;
        color: var(--cfp-text);
        font: calc(13px + var(--cfp-font-offset, 0px))/1.45 -apple-system,BlinkMacSystemFont,"SF Pro Text","PingFang SC",sans-serif;
        position: fixed;
        z-index: 2147483001;
      }
      [${ROOT_ATTR}="true"][data-theme="dark"] {
        --cfp-bg: rgba(31,34,40,.92);
        --cfp-border: rgba(255,255,255,.14);
        --cfp-text: rgba(247,248,250,.94);
        --cfp-muted: rgba(247,248,250,.58);
        --cfp-row: rgba(255,255,255,.075);
      }
      [${ROOT_ATTR}="true"] *, [${ROOT_ATTR}="true"] *::before, [${ROOT_ATTR}="true"] *::after { box-sizing: border-box; }
      [${ROOT_ATTR}="true"] .cfp-shell {
        background: var(--cfp-bg);
        border: 1px solid var(--cfp-border);
        border-radius: 24px;
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 0;
        overflow: hidden;
        pointer-events: auto;
        width: 100%;
      }
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-shell { border-radius: 999px; cursor: grab; }
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-shell:active { cursor: grabbing; }
      [${ROOT_ATTR}="true"][data-direction="up"] .cfp-shell { flex-direction: column-reverse; }
      [${ROOT_ATTR}="true"] .cfp-head {
        align-items: center;
        display: grid;
        flex: 0 0 46px;
        gap: 8px;
        grid-template-columns: minmax(0,1fr) 46px minmax(0,1fr);
        min-height: 46px;
        padding: 0 10px;
        user-select: none;
      }
      [${ROOT_ATTR}="true"] .cfp-status-face {
        align-items: center;
        color: var(--cfp-accent);
        display: flex;
        font-size: 18px;
        height: 46px;
        justify-content: center;
        justify-self: center;
        width: 46px;
      }
      [${ROOT_ATTR}="true"] .cfp-capsule-copy { min-width: 0; overflow: hidden; white-space: nowrap; }
      [${ROOT_ATTR}="true"] .cfp-capsule-label { font-weight: 650; }
      [${ROOT_ATTR}="true"] .cfp-capsule-count { color: var(--cfp-muted); font-size: .9em; }
      [${ROOT_ATTR}="true"] .cfp-head-tools { align-items: center; display: flex; gap: 3px; justify-content: flex-end; }
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-head { display: flex; gap: 7px; justify-content: center; padding: 0 12px; }
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-status-face { flex: 0 0 24px; height: 32px; width: 24px; }
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-head-tools,
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-tabs { display: none; }
      [${ROOT_ATTR}="true"][data-open="true"] .cfp-capsule-copy { opacity: 0; pointer-events: none; position: absolute; }
      [${ROOT_ATTR}="true"] button { color: inherit; font: inherit; }
      [${ROOT_ATTR}="true"] .cfp-icon,
      [${ROOT_ATTR}="true"] .cfp-tab,
      [${ROOT_ATTR}="true"] .cfp-nav {
        background: transparent;
        border: 0;
        border-radius: 9px;
        cursor: pointer;
        min-height: 28px;
        min-width: 28px;
        padding: 5px 7px;
      }
      [${ROOT_ATTR}="true"] .cfp-icon:hover,
      [${ROOT_ATTR}="true"] .cfp-tab:hover,
      [${ROOT_ATTR}="true"] .cfp-nav:hover { background: var(--cfp-row); }
      [${ROOT_ATTR}="true"] .cfp-head-tools,
      [${ROOT_ATTR}="true"] .cfp-outline-tools { opacity: 0; transition: opacity 140ms ease; }
      [${ROOT_ATTR}="true"]:hover .cfp-head-tools,
      [${ROOT_ATTR}="true"]:focus-within .cfp-head-tools,
      [${ROOT_ATTR}="true"]:hover .cfp-outline-tools,
      [${ROOT_ATTR}="true"]:focus-within .cfp-outline-tools { opacity: 1; }
      [${ROOT_ATTR}="true"] .cfp-tabs { align-items: center; display: flex; gap: 3px; min-width: 0; }
      [${ROOT_ATTR}="true"] .cfp-tab { color: var(--cfp-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      [${ROOT_ATTR}="true"] .cfp-tab[data-active="true"] { background: var(--cfp-row); color: var(--cfp-accent); font-weight: 650; }
      [${ROOT_ATTR}="true"] .cfp-body { display: flex; flex: 1; min-height: 0; opacity: 1; overflow: hidden; }
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-body { opacity: 0; pointer-events: none; }
      [${ROOT_ATTR}="true"] .cfp-view { display: flex; flex: 1; flex-direction: column; min-height: 0; overflow: hidden; padding: 2px 10px 10px; }
      [${ROOT_ATTR}="true"] .cfp-scroll { flex: 1; min-height: 0; overflow: auto; padding: 3px 4px 8px; scrollbar-width: none; }
      [${ROOT_ATTR}="true"] .cfp-scroll::-webkit-scrollbar { display: none; }
      [${ROOT_ATTR}="true"] .cfp-row { background: var(--cfp-row); border-radius: 12px; margin: 5px 0; padding: 10px 11px; }
      [${ROOT_ATTR}="true"] .cfp-row button { background: none; border: 0; cursor: pointer; display: block; padding: 0; text-align: left; width: 100%; }
      [${ROOT_ATTR}="true"] .cfp-row-title { font-weight: 650; }
      [${ROOT_ATTR}="true"] .cfp-row-summary { color: var(--cfp-muted); font-size: .92em; margin-top: 3px; }
      [${ROOT_ATTR}="true"] .cfp-outline-row { align-items: center; display: grid; gap: 7px; grid-template-columns: 7px auto minmax(0,1fr); padding-left: calc(8px + var(--cfp-indent,0px)); }
      [${ROOT_ATTR}="true"] .cfp-outline-dot { background: var(--cfp-accent); border-radius: 50%; height: 5px; opacity: .32; width: 5px; }
      [${ROOT_ATTR}="true"] .cfp-outline-row[data-level="0"] .cfp-outline-dot { height: 6px; opacity: .9; width: 6px; }
      [${ROOT_ATTR}="true"] .cfp-outline-prefix { color: var(--cfp-muted); }
      [${ROOT_ATTR}="true"] .cfp-outline-tools { align-items: center; display: flex; gap: 4px; justify-content: flex-end; padding: 5px 4px 0; }
      [${ROOT_ATTR}="true"] .cfp-empty { align-items: center; color: var(--cfp-muted); display: flex; flex: 1; justify-content: center; min-height: 120px; padding: 18px; text-align: center; }
      [${ROOT_ATTR}="true"] .cfp-settings { display: grid; gap: 9px; }
      [${ROOT_ATTR}="true"] .cfp-setting-row { align-items: center; display: flex; gap: 8px; justify-content: space-between; }
      [${ROOT_ATTR}="true"] .cfp-setting-value { color: var(--cfp-muted); }
      [${ROOT_ATTR}="true"] .cfp-setting-actions { display: flex; flex-wrap: wrap; gap: 6px; }
      [${ROOT_ATTR}="true"] .cfp-resize {
        bottom: 2px;
        cursor: nwse-resize;
        height: 18px;
        position: absolute;
        right: 2px;
        width: 18px;
      }
      [${ROOT_ATTR}="true"][data-direction="up"] .cfp-resize { bottom: auto; cursor: nesw-resize; top: 2px; }
      [${ROOT_ATTR}="true"][data-open="false"] .cfp-resize { display: none; }
      @media (prefers-reduced-motion: reduce) {
        [${ROOT_ATTR}="true"] *, [${ROOT_ATTR}="true"] *::before, [${ROOT_ATTR}="true"] *::after { animation-duration: .001ms !important; transition-duration: .001ms !important; }
      }
    `;
    document.head.appendChild(style);
  }

  function installRoot() {
    if (state.root?.isConnected) return;
    state.root = document.createElement("section");
    state.root.setAttribute(ROOT_ATTR, "true");
    state.root.innerHTML = `
      <div class="cfp-shell" role="complementary" aria-label="Stepwise 悬浮面板">
        <header class="cfp-head" data-drag-surface>
          <div class="cfp-tabs" data-tabs></div>
          <button class="cfp-status-face" type="button" data-action="toggle" aria-label="展开或收起">✦</button>
          <div class="cfp-capsule-copy"><span class="cfp-capsule-label">Stepwise</span><span class="cfp-capsule-count"></span></div>
          <div class="cfp-head-tools">
            <button class="cfp-icon" type="button" data-action="refresh" title="刷新" aria-label="刷新">↻</button>
            <button class="cfp-icon" type="button" data-action="settings" title="设置" aria-label="设置">⚙</button>
            <button class="cfp-icon" type="button" data-action="close" title="收起" aria-label="收起">×</button>
          </div>
        </header>
        <div class="cfp-body" data-body></div>
        <div class="cfp-resize" data-action="resize" aria-hidden="true"></div>
      </div>
    `;
    state.shell = state.root.querySelector(".cfp-shell");
    state.header = state.root.querySelector(".cfp-head");
    state.body = state.root.querySelector("[data-body]");
    document.body.appendChild(state.root);
    state.root.addEventListener("click", onClick);
    state.header.addEventListener("pointerdown", beginDrag);
    state.root.querySelector("[data-action='resize']")?.addEventListener("pointerdown", beginResize);
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("resize", onWindowResize);
    applyGeometry();
  }

  function status() {
    if (state.activeTab === "outline") {
      if (state.outline.status === "pending") return { glyph: "◌", label: "正在生成大纲", count: "" };
      if (state.outline.status === "error") return { glyph: "!", label: "大纲暂不可用", count: "" };
      return { glyph: "⌁", label: "回答大纲", count: state.outline.items.length ? ` · ${state.outline.items.length}` : "" };
    }
    const core = coreState();
    if (core?.bridgeStatus === "pending") return { glyph: "◌", label: "正在生成建议", count: "" };
    if (core?.bridgeStatus === "failed") return { glyph: "!", label: "建议生成失败", count: "" };
    return { glyph: prompts().length ? "✦" : "·", label: "Stepwise", count: prompts().length ? ` · ${prompts().length}` : "" };
  }

  function tabsHtml() {
    return availableTabs().filter((tab) => tab !== "settings").map((tab) => {
      const label = tab === "outline" ? "大纲" : "Stepwise";
      return `<button class="cfp-tab" type="button" data-tab="${tab}" data-active="${state.activeTab === tab}">${label}</button>`;
    }).join("");
  }

  function nextHtml() {
    const items = prompts();
    if (coreState()?.bridgeStatus === "pending") return `<div class="cfp-empty">正在生成建议</div>`;
    if (!items.length) {
      const message = state.settings?.generationMode === "manual" ? "点击刷新生成建议" : "当前回答暂无建议";
      return `<div class="cfp-empty">${escapeHtml(message)}</div>`;
    }
    return `<div class="cfp-view"><div class="cfp-scroll">${items.map((item, index) => `
      <div class="cfp-row" data-prompt-index="${index}">
        <button type="button" data-action="prompt" data-index="${index}" title="${escapeAttr(item.prompt)}">
          <div class="cfp-row-title">${escapeHtml(item.label || item.prompt)}</div>
          ${item.summary ? `<div class="cfp-row-summary">${escapeHtml(item.summary)}</div>` : ""}
        </button>
      </div>`).join("")}</div></div>`;
  }

  function outlineHtml() {
    if (state.outline.status === "pending") return `<div class="cfp-empty">正在生成大纲</div>`;
    if (state.outline.status === "error") return `<div class="cfp-empty">${escapeHtml(state.outline.error || "大纲暂不可用")}</div>`;
    if (!state.outline.items.length) return `<div class="cfp-empty">当前回答暂无可识别标题</div>`;
    return `<div class="cfp-view">
      <div class="cfp-scroll">${state.outline.items.map((item) => `
        <div class="cfp-row cfp-outline-row" data-level="${item.displayLevel || 0}" style="--cfp-indent:${Math.min(3, item.displayLevel || 0) * 12}px">
          <span class="cfp-outline-dot" aria-hidden="true"></span>
          <span class="cfp-outline-prefix">${escapeHtml(item.numberPrefix || "")}</span>
          <button type="button" data-outline-id="${escapeAttr(item.id)}">${escapeHtml(item.labelText || item.text)}</button>
        </div>`).join("")}</div>
      <div class="cfp-outline-tools" role="toolbar" aria-label="本轮导航">
        <button class="cfp-nav" type="button" data-anchor="start" title="本轮开头" aria-label="定位到本轮开头">↑</button>
        <button class="cfp-nav" type="button" data-anchor="end" title="本轮结尾" aria-label="定位到本轮结尾">↓</button>
      </div>
    </div>`;
  }

  function settingsHtml() {
    const material = window[VISUAL_KEY]?.current?.().material || "跟随默认";
    const clickModeLabels = { fill: "填入输入框", direct: "直接发送", hybrid: "单击填入 · 双击发送" };
    return `<div class="cfp-view"><div class="cfp-scroll cfp-settings">
      <div class="cfp-setting-row"><span>建议操作</span><span class="cfp-setting-value">${escapeHtml(clickModeLabels[state.clickMode])}</span></div>
      <div class="cfp-setting-actions">
        <button class="cfp-tab" type="button" data-action="click-mode">切换方式</button>
      </div>
      <div class="cfp-setting-row"><span>字号</span><span class="cfp-setting-value">${state.fontOffset >= 0 ? "+" : ""}${state.fontOffset}px</span></div>
      <div class="cfp-setting-actions">
        <button class="cfp-tab" type="button" data-action="font-down">A−</button>
        <button class="cfp-tab" type="button" data-action="font-up">A＋</button>
      </div>
      <div class="cfp-setting-row"><span>外观</span><span class="cfp-setting-value">${escapeHtml(material)}</span></div>
      <div class="cfp-setting-actions">
        <button class="cfp-tab" type="button" data-action="material">切换主题</button>
        <button class="cfp-tab" type="button" data-action="reset-position">归位</button>
        <button class="cfp-tab" type="button" data-action="open-manager">打开 Manager</button>
      </div>
    </div></div>`;
  }

  function viewHtml() {
    if (state.activeTab === "outline") return outlineHtml();
    if (state.activeTab === "settings") return settingsHtml();
    return nextHtml();
  }

  function render() {
    if (state.destroyed || !state.root?.isConnected) return;
    state.activeTab = normalizeTab();
    const current = status();
    state.root.querySelector("[data-tabs]").innerHTML = tabsHtml();
    state.root.querySelector(".cfp-status-face").textContent = current.glyph;
    state.root.querySelector(".cfp-capsule-label").textContent = current.label;
    state.root.querySelector(".cfp-capsule-count").textContent = current.count;
    const previousScroll = state.body.querySelector(".cfp-scroll")?.scrollTop || 0;
    state.body.innerHTML = viewHtml();
    const nextScroll = state.body.querySelector(".cfp-scroll");
    if (nextScroll) nextScroll.scrollTop = previousScroll;
    applyGeometry();
    window[VISUAL_KEY]?.apply?.();
  }

  function scheduleRender() {
    if (state.renderFrame) return;
    state.renderFrame = requestAnimationFrame(() => {
      state.renderFrame = 0;
      render();
    });
  }

  function morphFrames(open) {
    const collapsed = geometry(false);
    const expanded = geometry(true);
    const horizontal = {
      ...collapsed,
      width: expanded.width,
      left: state.anchor.x - expanded.width / 2,
    };
    return open ? [collapsed, horizontal, expanded] : [expanded, horizontal, collapsed];
  }

  async function setOpen(open) {
    if (state.transitioning || state.open === open) return;
    state.transitioning = true;
    state.direction = resolveDirection();
    const [first, middle, last] = morphFrames(open);
    state.open = open;
    state.root.dataset.transitioning = "true";
    applyGeometry(first);
    await nextFrame();
    await animateGeometry(first, middle, HORIZONTAL_MS);
    await animateGeometry(middle, last, VERTICAL_MS);
    state.transitioning = false;
    state.root.dataset.transitioning = "false";
    applyGeometry(last);
    render();
  }

  function nextFrame() {
    return new Promise((resolve) => requestAnimationFrame(() => resolve()));
  }

  function animateGeometry(from, to, duration) {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      applyGeometry(to);
      return Promise.resolve();
    }
    const started = performance.now();
    return new Promise((resolve) => {
      const tick = (now) => {
        if (state.destroyed) return resolve();
        const progress = clamp((now - started) / duration, 0, 1);
        const eased = 1 - Math.pow(1 - progress, 3);
        applyGeometry({
          width: from.width + (to.width - from.width) * eased,
          height: from.height + (to.height - from.height) * eased,
          left: from.left + (to.left - from.left) * eased,
          top: from.top + (to.top - from.top) * eased,
          direction: to.direction,
        });
        if (progress < 1) requestAnimationFrame(tick);
        else resolve();
      };
      requestAnimationFrame(tick);
    });
  }

  function switchTab(tab) {
    const next = normalizeTab(tab);
    if (next === state.activeTab) return;
    const body = state.body;
    if (!body || matchMedia("(prefers-reduced-motion: reduce)").matches) {
      state.activeTab = next;
      render();
      return;
    }
    const direction = availableTabs().indexOf(next) > availableTabs().indexOf(state.activeTab) ? 1 : -1;
    const old = body.firstElementChild;
    state.activeTab = next;
    body.innerHTML = viewHtml();
    const current = body.firstElementChild;
    current?.animate([
      { opacity: 0, transform: `translateX(${direction * 12}px)` },
      { opacity: 1, transform: "translateX(0)" },
    ], { duration: VIEW_MS, easing: "cubic-bezier(.2,.8,.2,1)" });
    old?.remove();
    render();
  }

  function composer() {
    return document.querySelector("textarea, [contenteditable='true'].ProseMirror, [contenteditable='true']");
  }

  function writeComposer(prompt, submit) {
    const target = composer();
    if (!target) return false;
    target.focus();
    if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
      const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(target), "value")?.set;
      setter?.call(target, prompt);
    } else {
      target.textContent = prompt;
    }
    target.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: prompt }));
    if (submit) {
      target.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true }));
      target.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", code: "Enter", bubbles: true }));
    }
    return true;
  }

  function selectPrompt(index, clickDetail) {
    const item = prompts()[index];
    if (!item?.prompt) return;
    const submit = state.clickMode === "direct" || (state.clickMode === "hybrid" && clickDetail > 1);
    writeComposer(item.prompt, submit);
  }

  function refreshCurrent() {
    if (state.activeTab === "outline") return outlineApi()?.refresh?.({ force: true });
    return coreApi()?.forceRefresh?.() || coreApi()?.scan?.();
  }

  function onClick(event) {
    const target = event.target?.closest?.("[data-action],[data-tab],[data-outline-id],[data-anchor]");
    if (!target || !state.root.contains(target)) return;
    if (target.dataset.tab) return switchTab(target.dataset.tab);
    if (target.dataset.outlineId) return outlineApi()?.jumpTo?.(target.dataset.outlineId);
    if (target.dataset.anchor) return outlineApi()?.jumpToAnchor?.(target.dataset.anchor);
    switch (target.dataset.action) {
      case "toggle": void setOpen(!state.open); break;
      case "close": void setOpen(false); break;
      case "refresh": void refreshCurrent(); break;
      case "settings": switchTab("settings"); break;
      case "prompt": selectPrompt(Number(target.dataset.index), event.detail); break;
      case "click-mode": writeClickMode(state.clickMode === "hybrid" ? "fill" : state.clickMode === "fill" ? "direct" : "hybrid"); break;
      case "font-down": setFontOffset(state.fontOffset - 1); break;
      case "font-up": setFontOffset(state.fontOffset + 1); break;
      case "material": window[VISUAL_KEY]?.cycle?.(); scheduleRender(); break;
      case "reset-position": resetPosition(); break;
      case "open-manager": void bridgeCall("/manager/open-transient", { page: "settings", section: "stepwise" }); break;
      default: break;
    }
  }

  function setFontOffset(value) {
    state.fontOffset = clamp(value, -3, 9);
    writeNumber(FONT_OFFSET_KEY, state.fontOffset);
    render();
  }

  function resetPosition() {
    state.anchor = {
      x: Math.max(SAFE_MARGIN + FACE_RADIUS, window.innerWidth - 82),
      y: Math.max(SAFE_MARGIN + FACE_RADIUS, window.innerHeight * 0.42),
    };
    writeJson(POSITION_KEY, state.anchor);
    applyGeometry();
  }

  function beginDrag(event) {
    if (event.button !== 0 || event.target?.closest?.("button:not(.cfp-status-face)")) return;
    state.drag = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      anchorX: state.anchor.x,
      anchorY: state.anchor.y,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function beginResize(event) {
    if (event.button !== 0) return;
    event.stopPropagation();
    state.resize = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      width: state.size.width,
      height: state.size.height,
      direction: state.direction,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onPointerMove(event) {
    if (state.drag?.pointerId === event.pointerId) {
      state.anchor = clampAnchor({
        x: state.drag.anchorX + event.clientX - state.drag.startX,
        y: state.drag.anchorY + event.clientY - state.drag.startY,
      });
      applyGeometry();
    }
    if (state.resize?.pointerId === event.pointerId) {
      const deltaY = event.clientY - state.resize.startY;
      state.size = {
        width: clamp(state.resize.width + (event.clientX - state.resize.startX) * 2, MIN_WIDTH, MAX_WIDTH),
        height: clamp(
          state.resize.height + (state.resize.direction === "up" ? -deltaY : deltaY),
          MIN_HEIGHT,
          heightCap(),
        ),
      };
      applyGeometry();
    }
  }

  function onPointerUp(event) {
    if (state.drag?.pointerId === event.pointerId) {
      state.drag = null;
      writeJson(POSITION_KEY, state.anchor);
    }
    if (state.resize?.pointerId === event.pointerId) {
      state.resize = null;
      writeJson(SIZE_KEY, state.size);
    }
  }

  function onWindowResize() {
    state.anchor = clampAnchor(state.anchor);
    state.size.height = clamp(state.size.height, MIN_HEIGHT, heightCap());
    applyGeometry();
  }

  function hideLegacyPanel() {
    document.querySelectorAll?.(`[${LEGACY_ROOT_ATTR}="true"]`).forEach((node) => {
      node.style.setProperty("display", "none", "important");
      node.dataset.codexFloatingPanelHidden = "true";
    });
  }

  function restoreLegacyPanel() {
    document.querySelectorAll?.("[data-codex-floating-panel-hidden='true']").forEach((node) => {
      node.style.removeProperty("display");
      delete node.dataset.codexFloatingPanelHidden;
    });
  }

  async function loadSettings() {
    const response = await bridgeCall("/stepwise/settings", {});
    if (response?.settings) state.settings = response.settings;
    return state.settings;
  }

  function connectOutline() {
    state.outlineUnsubscribe?.();
    const api = outlineApi();
    if (!api?.subscribe) {
      state.outline = emptyOutline();
      return;
    }
    state.outlineUnsubscribe = api.subscribe((snapshot) => {
      state.outline = snapshot || emptyOutline();
      scheduleRender();
    });
  }

  function observe() {
    state.observer?.disconnect();
    state.observer = new MutationObserver((records) => {
      hideLegacyPanel();
      if (records.some((record) => !state.root?.contains(record.target))) scheduleRender();
      if (!state.outlineUnsubscribe && outlineApi()?.subscribe) connectOutline();
    });
    state.observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  }

  async function syncSettings(patch = {}) {
    state.settings = { ...(state.settings || {}), ...patch };
    if (!runtimeEnabled()) {
      destroyUi();
      return state.settings;
    }
    if (!state.root?.isConnected) startUi();
    render();
    return state.settings;
  }

  function startUi() {
    if (!document.body || state.destroyed) return;
    installStyle();
    installRoot();
    connectOutline();
    hideLegacyPanel();
    observe();
    render();
  }

  function destroyUi() {
    state.observer?.disconnect();
    state.observer = null;
    state.outlineUnsubscribe?.();
    state.outlineUnsubscribe = null;
    state.root?.remove();
    state.root = null;
    state.shell = null;
    state.header = null;
    state.body = null;
    restoreLegacyPanel();
    document.getElementById(STYLE_ID)?.remove();
  }

  function destroy() {
    state.destroyed = true;
    window.clearTimeout(state.settingsTimer);
    if (state.renderFrame) cancelAnimationFrame(state.renderFrame);
    destroyUi();
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
    window.removeEventListener("resize", onWindowResize);
    if (window[API_KEY]?.instanceId === instanceId) delete window[API_KEY];
  }

  async function start() {
    await loadSettings();
    if (state.destroyed || !runtimeEnabled()) return;
    if (!document.body) {
      document.addEventListener("DOMContentLoaded", () => void start(), { once: true });
      return;
    }
    startUi();
    state.settingsTimer = window.setInterval(async () => {
      await loadSettings();
      if (runtimeEnabled()) {
        coreApi()?.syncSettings?.(state.settings || {});
        outlineApi()?.syncSettings?.(state.settings || {});
        if (!state.root?.isConnected) startUi();
        scheduleRender();
      } else {
        destroyUi();
      }
    }, SETTINGS_POLL_MS);
  }

  const instanceId = `floating-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  window[API_KEY] = {
    version: "2.0.0-structure",
    instanceId,
    state,
    start,
    destroy,
    syncSettings,
    setOpen,
    switchTab,
    render,
  };
  void start();
})();
