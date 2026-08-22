(() => {
  "use strict";

  const API_KEY = "__codexFloatingPanel";
  const CORE_KEY = "__codexStepwisePanel";
  const BRIDGE_KEY = "__codexSessionDeleteBridge";
  const STYLE_ID = "codex-floating-panel-style";
  const ROOT_ATTR = "data-codex-floating-panel-root";
  const LEGACY_ROOT_ATTR = "data-codex-stepwise-root";
  const POSITION_KEY = "codex-floating-panel-position-v1";
  const SIZE_KEY = "codex-floating-panel-size-v1";
  const MATERIAL_KEY = "codex-floating-panel-material-v1";
  const MAX_OUTLINE_ITEMS = 24;
  const OUTLINE_TOP_OFFSET = 28;
  const BRIDGE_TIMEOUT_MS = 26000;
  const MIN_WIDTH = 300;
  const MAX_WIDTH = 620;
  const MIN_HEIGHT = 220;
  const MAX_HEIGHT = 720;
  const MATERIALS = ["frosted", "clear", "liquid", "crystal", "matte"];

  const previous = window[API_KEY];
  if (previous && typeof previous.destroy === "function") previous.destroy();
  document.querySelectorAll?.(`[${ROOT_ATTR}="true"]`).forEach((node) => node.remove());
  document.getElementById(STYLE_ID)?.remove();

  const state = {
    root: null,
    capsule: null,
    panel: null,
    core: null,
    settings: null,
    open: false,
    activeTab: "next",
    position: readJson(POSITION_KEY, null),
    size: readJson(SIZE_KEY, { width: 360, height: 360 }),
    material: readMaterial(),
    outline: [],
    outlineTargets: new Map(),
    outlineHash: "",
    outlineStatus: "idle",
    outlineError: "",
    latestMessage: null,
    observer: null,
    timer: 0,
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

  function readMaterial() {
    const value = localStorage.getItem(MATERIAL_KEY);
    return MATERIALS.includes(value) ? value : "frosted";
  }

  function isCurrent() {
    return !state.destroyed && window[API_KEY]?.instanceId === instanceId;
  }

  function bridgeCall(path, payload = {}) {
    if (typeof window[BRIDGE_KEY] !== "function") {
      return Promise.resolve({ status: "failed", error: "page bridge is not installed" });
    }
    let timer = 0;
    const timeout = new Promise((resolve) => {
      timer = window.setTimeout(() => resolve({ status: "failed", error: "page bridge timed out" }), BRIDGE_TIMEOUT_MS);
    });
    const request = Promise.resolve(window[BRIDGE_KEY](path, payload));
    return Promise.race([request, timeout]).finally(() => window.clearTimeout(timer));
  }

  function normalizeText(value) {
    return String(value || "")
      .replace(/\u00a0/g, " ")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n{3,}/g, "\n\n")
      .replace(/[ \t]{2,}/g, " ")
      .trim();
  }

  function hashText(value) {
    const text = normalizeText(value).slice(-12000);
    let hash = 2166136261;
    for (let index = 0; index < text.length; index += 1) {
      hash ^= text.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return (hash >>> 0).toString(36);
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

  function visible(node) {
    if (!(node instanceof Element)) return false;
    const rect = node.getBoundingClientRect();
    return rect.width > 8 && rect.height > 8 && rect.bottom > 0 && rect.top < window.innerHeight;
  }

  function coreState() {
    state.core = window[CORE_KEY]?.state || state.core;
    return state.core;
  }

  function stepwiseEnabled() {
    return coreState()?.settings?.enabled === true || state.settings?.enabled === true;
  }

  function outlineEnabled() {
    return coreState()?.settings?.answerOutlineEnabled === true
      || state.settings?.answerOutlineEnabled === true;
  }

  function runtimeEnabled() {
    return stepwiseEnabled() || outlineEnabled();
  }

  function prompts() {
    const values = coreState()?.prompts;
    return Array.isArray(values) ? values.filter((item) => item?.prompt) : [];
  }

  function statusLabel() {
    const core = coreState();
    if (state.activeTab === "outline") {
      if (state.outlineStatus === "pending") return "正在整理大纲";
      if (state.outlineError) return "大纲暂不可用";
      return state.outline.length ? `${state.outline.length} 个标题` : "回答大纲";
    }
    if (core?.bridgeStatus === "pending") return "正在生成建议";
    if (core?.bridgeStatus === "failed") return "建议生成失败";
    return prompts().length ? `${prompts().length} 条建议` : "Stepwise";
  }

  function statusGlyph() {
    if (state.activeTab === "outline") return state.outlineStatus === "pending" ? "◌" : "⌁";
    const status = coreState()?.bridgeStatus;
    if (status === "pending") return "◌";
    if (status === "failed") return "!";
    return prompts().length ? "✦" : "·";
  }

  function installStyle() {
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      [${ROOT_ATTR}="true"] {
        --cfp-bg: rgba(247, 248, 250, .92);
        --cfp-panel-bg: rgba(247, 248, 250, .88);
        --cfp-text: rgba(21, 24, 29, .94);
        --cfp-muted: rgba(21, 24, 29, .58);
        --cfp-border: rgba(21, 24, 29, .14);
        --cfp-accent: #3878ff;
        --cfp-row: rgba(255, 255, 255, .44);
        --cfp-shadow: 0 18px 48px rgba(24, 32, 48, .18);
        color: var(--cfp-text);
        font: 13px/1.45 -apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", sans-serif;
        pointer-events: none;
        position: fixed;
        inset: 0;
        z-index: 2147483001;
      }
      [${ROOT_ATTR}="true"][data-theme="dark"] {
        --cfp-bg: rgba(31, 34, 40, .9);
        --cfp-panel-bg: rgba(31, 34, 40, .84);
        --cfp-text: rgba(247, 248, 250, .94);
        --cfp-muted: rgba(247, 248, 250, .58);
        --cfp-border: rgba(255, 255, 255, .14);
        --cfp-row: rgba(255, 255, 255, .07);
        --cfp-shadow: 0 22px 60px rgba(0, 0, 0, .42);
      }
      [${ROOT_ATTR}="true"] .cfp-legacy-hidden { display: none !important; }
      [${ROOT_ATTR}="true"] .cfp-shell { pointer-events: auto; position: fixed; }
      [${ROOT_ATTR}="true"] .cfp-capsule,
      [${ROOT_ATTR}="true"] .cfp-panel {
        -webkit-backdrop-filter: blur(24px) saturate(1.08);
        backdrop-filter: blur(24px) saturate(1.08);
        background: var(--cfp-bg);
        border: 1px solid var(--cfp-border);
        box-shadow: var(--cfp-shadow);
      }
      [${ROOT_ATTR}="true"] .cfp-capsule {
        align-items: center;
        border-radius: 999px;
        cursor: grab;
        display: inline-flex;
        gap: 8px;
        min-height: 44px;
        padding: 0 14px;
        user-select: none;
        white-space: nowrap;
      }
      [${ROOT_ATTR}="true"] .cfp-capsule:active { cursor: grabbing; }
      [${ROOT_ATTR}="true"] .cfp-glyph { color: var(--cfp-accent); font-size: 18px; line-height: 1; }
      [${ROOT_ATTR}="true"] .cfp-capsule-label { font-weight: 650; }
      [${ROOT_ATTR}="true"] .cfp-capsule-count { color: var(--cfp-muted); font-size: 12px; }
      [${ROOT_ATTR}="true"] .cfp-panel {
        border-radius: 22px;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        resize: none;
        transition: opacity 180ms ease, transform 220ms cubic-bezier(.2,.8,.2,1), width 220ms ease, height 220ms ease;
      }
      [${ROOT_ATTR}="true"] .cfp-panel[data-open="false"] { opacity: 0; pointer-events: none; transform: translateY(8px) scale(.98); }
      [${ROOT_ATTR}="true"] .cfp-panel[data-open="true"] { opacity: 1; transform: translateY(0) scale(1); }
      [${ROOT_ATTR}="true"] .cfp-head { align-items: center; display: flex; gap: 10px; justify-content: space-between; padding: 14px 16px 10px; }
      [${ROOT_ATTR}="true"] .cfp-title { font-size: 14px; font-weight: 700; }
      [${ROOT_ATTR}="true"] .cfp-actions, [${ROOT_ATTR}="true"] .cfp-tabs { align-items: center; display: flex; gap: 4px; }
      [${ROOT_ATTR}="true"] button { color: inherit; font: inherit; }
      [${ROOT_ATTR}="true"] .cfp-icon, [${ROOT_ATTR}="true"] .cfp-tab, [${ROOT_ATTR}="true"] .cfp-nav {
        background: transparent; border: 0; border-radius: 9px; cursor: pointer; padding: 6px 8px;
      }
      [${ROOT_ATTR}="true"] .cfp-icon:hover, [${ROOT_ATTR}="true"] .cfp-tab:hover, [${ROOT_ATTR}="true"] .cfp-nav:hover { background: var(--cfp-row); }
      [${ROOT_ATTR}="true"] .cfp-tab[data-active="true"] { background: var(--cfp-row); color: var(--cfp-accent); font-weight: 650; }
      [${ROOT_ATTR}="true"] .cfp-body { display: flex; flex: 1; min-height: 0; flex-direction: column; padding: 0 10px 10px; }
      [${ROOT_ATTR}="true"] .cfp-scroll { flex: 1; min-height: 0; overflow: auto; padding: 4px 4px 8px; scrollbar-width: none; }
      [${ROOT_ATTR}="true"] .cfp-scroll::-webkit-scrollbar { display: none; }
      [${ROOT_ATTR}="true"] .cfp-row { background: var(--cfp-row); border-radius: 12px; margin: 5px 0; padding: 10px 11px; text-align: left; }
      [${ROOT_ATTR}="true"] .cfp-row button { background: none; border: 0; cursor: pointer; display: block; padding: 0; text-align: left; width: 100%; }
      [${ROOT_ATTR}="true"] .cfp-row-title { font-weight: 650; }
      [${ROOT_ATTR}="true"] .cfp-row-summary { color: var(--cfp-muted); font-size: 12px; margin-top: 3px; }
      [${ROOT_ATTR}="true"] .cfp-empty { align-items: center; color: var(--cfp-muted); display: flex; flex: 1; justify-content: center; min-height: 100px; text-align: center; }
      [${ROOT_ATTR}="true"] .cfp-outline-row { align-items: baseline; display: grid; gap: 7px; grid-template-columns: max-content minmax(0, 1fr); }
      [${ROOT_ATTR}="true"] .cfp-outline-row[data-level="2"] { padding-left: 12px; }
      [${ROOT_ATTR}="true"] .cfp-outline-index { color: var(--cfp-accent); font-variant-numeric: tabular-nums; }
      [${ROOT_ATTR}="true"] .cfp-outline-row button { color: inherit; overflow-wrap: anywhere; }
      [${ROOT_ATTR}="true"] .cfp-footer { align-items: center; border-top: 1px solid var(--cfp-border); display: flex; gap: 4px; justify-content: flex-end; padding: 8px 4px 0; }
      [${ROOT_ATTR}="true"] .cfp-resize { bottom: 3px; cursor: nwse-resize; height: 18px; position: absolute; right: 3px; width: 18px; }
      [${ROOT_ATTR}="true"] .cfp-resize::after { border-bottom: 2px solid var(--cfp-muted); border-right: 2px solid var(--cfp-muted); content: ""; height: 8px; position: absolute; right: 2px; bottom: 2px; width: 8px; }
      [${ROOT_ATTR}="true"][data-material="clear"] { --cfp-bg: rgba(255,255,255,.22); --cfp-panel-bg: rgba(255,255,255,.18); }
      [${ROOT_ATTR}="true"][data-material="liquid"] { --cfp-bg: rgba(142,188,255,.26); --cfp-panel-bg: rgba(142,188,255,.22); }
      [${ROOT_ATTR}="true"][data-material="crystal"] { --cfp-bg: rgba(210,231,255,.32); --cfp-panel-bg: rgba(210,231,255,.28); }
      [${ROOT_ATTR}="true"][data-material="matte"] { --cfp-bg: rgba(108,115,128,.72); --cfp-panel-bg: rgba(108,115,128,.68); }
      [${ROOT_ATTR}="true"] .codex-floating-outline-target { background: color-mix(in srgb, var(--cfp-accent) 18%, transparent); border-radius: 6px; }
      @media (prefers-reduced-motion: reduce) { [${ROOT_ATTR}="true"] .cfp-panel { transition: none; } }
    `;
    document.documentElement.appendChild(style);
  }

  function detectTheme() {
    const classes = document.documentElement.classList;
    return classes.contains("electron-dark") || classes.contains("theme-dark") ? "dark" : "light";
  }

  function defaultPosition() {
    return { left: Math.max(16, window.innerWidth - 180), top: Math.max(16, window.innerHeight - 90) };
  }

  function applyPosition() {
    const position = state.position || defaultPosition();
    state.position = {
      left: clamp(Number(position.left) || 0, 8, Math.max(8, window.innerWidth - 80)),
      top: clamp(Number(position.top) || 0, 8, Math.max(8, window.innerHeight - 58)),
    };
    if (state.root) {
      state.root.style.left = `${state.position.left}px`;
      state.root.style.top = `${state.position.top}px`;
    }
  }

  function applySize() {
    state.size.width = clamp(Number(state.size.width) || 360, MIN_WIDTH, MAX_WIDTH);
    state.size.height = clamp(Number(state.size.height) || 360, MIN_HEIGHT, MAX_HEIGHT);
    if (state.panel) {
      state.panel.style.width = `${state.size.width}px`;
      state.panel.style.height = `${state.size.height}px`;
    }
  }

  function installRoot() {
    if (state.root) return;
    state.root = document.createElement("div");
    state.root.setAttribute(ROOT_ATTR, "true");
    state.root.dataset.theme = detectTheme();
    state.root.dataset.material = state.material;
    state.root.innerHTML = `
      <div class="cfp-shell">
        <button class="cfp-capsule" type="button" aria-label="展开悬浮面板" data-action="toggle">
          <span class="cfp-glyph" aria-hidden="true"></span>
          <span class="cfp-capsule-label"></span>
          <span class="cfp-capsule-count"></span>
        </button>
        <section class="cfp-panel" data-open="false" aria-label="悬浮面板">
          <div class="cfp-head"><strong class="cfp-title">悬浮球</strong><div class="cfp-actions">
            <button class="cfp-icon" type="button" data-action="material" title="切换材质">◈</button>
            <button class="cfp-icon" type="button" data-action="manager" title="打开 Manager 设置">⚙</button>
            <button class="cfp-icon" type="button" data-action="close" title="收起">×</button>
          </div></div>
          <div class="cfp-tabs">
            <button class="cfp-tab" type="button" data-tab="next">Stepwise</button>
            <button class="cfp-tab" type="button" data-tab="outline">大纲</button>
          </div>
          <div class="cfp-body"></div>
          <span class="cfp-resize" data-action="resize" aria-hidden="true"></span>
        </section>
      </div>`;
    document.body.appendChild(state.root);
    state.capsule = state.root.querySelector(".cfp-capsule");
    state.panel = state.root.querySelector(".cfp-panel");
    state.root.addEventListener("click", onClick);
    state.capsule?.addEventListener("pointerdown", beginDrag);
    state.panel?.addEventListener("pointerdown", (event) => {
      if (event.target?.closest?.(".cfp-head")) beginDrag(event);
    });
    state.root.querySelector("[data-action='resize']")?.addEventListener("pointerdown", beginResize);
    applyPosition();
    applySize();
  }

  function onClick(event) {
    const target = event.target?.closest?.("[data-action], [data-tab], [data-outline-id], [data-anchor]");
    if (!target) return;
    const action = target.dataset.action;
    if (action === "toggle") return toggleOpen();
    if (action === "close") return setOpen(false);
    if (action === "material") return cycleMaterial();
    if (action === "manager") return openManager();
    if (action === "refresh-outline") return refreshOutline(true);
    if (target.dataset.tab) {
      state.activeTab = target.dataset.tab;
      render();
      if (state.activeTab === "outline") void refreshOutline(false);
      return;
    }
    if (target.dataset.outlineId) return jumpTo(target.dataset.outlineId);
    if (target.dataset.anchor) return jumpToAnchor(target.dataset.anchor);
    if (target.dataset.prompt) return fillComposer(target.dataset.prompt);
  }

  function setOpen(open) {
    state.open = open;
    if (open && state.activeTab === "outline") void refreshOutline(false);
    render();
  }

  function toggleOpen() {
    setOpen(!state.open);
  }

  function cycleMaterial() {
    const index = MATERIALS.indexOf(state.material);
    state.material = MATERIALS[(index + 1) % MATERIALS.length];
    localStorage.setItem(MATERIAL_KEY, state.material);
    if (state.root) state.root.dataset.material = state.material;
  }

  function render() {
    if (!isCurrent()) return;
    installRoot();
    state.root.dataset.theme = detectTheme();
    state.root.dataset.material = state.material;
    state.panel.dataset.open = String(state.open);
    state.capsule.querySelector(".cfp-glyph").textContent = statusGlyph();
    state.capsule.querySelector(".cfp-capsule-label").textContent = statusLabel();
    state.capsule.querySelector(".cfp-capsule-count").textContent = state.activeTab === "next" && prompts().length ? `· ${prompts().length}` : "";
    state.root.querySelectorAll("[data-tab]").forEach((tab) => { tab.dataset.active = String(tab.dataset.tab === state.activeTab); });
    state.panel.querySelector(".cfp-body").innerHTML = state.activeTab === "outline" ? outlineHtml() : nextHtml();
    applyPosition();
    applySize();
  }

  function nextHtml() {
    const values = prompts();
    if (!values.length) return `<div class="cfp-empty">${escapeHtml(statusLabel())}</div>`;
    return `<div class="cfp-scroll">${values.map((item) => `
      <div class="cfp-row"><button type="button" data-prompt="${escapeHtml(item.prompt)}">
        <div class="cfp-row-title">${escapeHtml(item.label || "下一步")}</div>
        <div class="cfp-row-summary">${escapeHtml(item.prompt)}</div>
      </button></div>`).join("")}</div>`;
  }

  function outlineHtml() {
    const rows = state.outline.map((item) => `
      <div class="cfp-row cfp-outline-row" data-level="${item.level}">
        <span class="cfp-outline-index">${escapeHtml(item.numberPrefix || "•")}</span>
        <button type="button" data-outline-id="${escapeHtml(item.id)}">${escapeHtml(item.title)}</button>
      </div>`).join("");
    const empty = state.outlineStatus === "pending" ? "正在整理大纲" : state.outlineError || "当前回答暂无可识别标题";
    return `<div class="cfp-scroll"><div class="cfp-row cfp-outline-row"><span class="cfp-outline-index">↥</span><button type="button" data-anchor="start">本轮开头</button></div>${rows || `<div class="cfp-empty">${escapeHtml(empty)}</div>`}<div class="cfp-row cfp-outline-row"><span class="cfp-outline-index">↧</span><button type="button" data-anchor="end">本轮结尾</button></div></div><div class="cfp-footer"><button class="cfp-nav" type="button" data-action="refresh-outline">刷新</button></div>`;
  }

  function latestAssistant() {
    const candidates = Array.from(document.querySelectorAll(
      `[data-message-author-role="assistant"], .group.flex.min-w-0.flex-col`
    )).filter((node) => visible(node) && !node.closest(`[${ROOT_ATTR}="true"]`));
    return candidates.at(-1) || null;
  }

  function assistantText(node) {
    const content = node?.querySelector?.("[class*='markdown'], [class*='prose'], article") || node;
    return normalizeText(content?.innerText || content?.textContent || "");
  }

  function localOutline(text) {
    const items = [];
    const seen = new Set();
    let inCode = false;
    for (const line of text.split(/\r?\n/)) {
      const value = line.trim();
      if (value.startsWith("```") || value.startsWith("~~~")) { inCode = !inCode; continue; }
      if (inCode || !value) continue;
      const match = value.match(/^(#{1,6})\s+(.+?)\s*#*$/) || value.match(/^((?:\d+(?:\.\d+)*|[一二三四五六七八九十]+)[、.．)])\s+(.+)$/);
      if (!match) continue;
      const title = normalizeText(match[2]).replace(/[:：]$/, "");
      if (title.length < 2 || title.length > 56) continue;
      const key = title.toLocaleLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      items.push({ level: match[1].startsWith("#") ? match[1].length : 1, title, numberPrefix: match[1].startsWith("#") ? "" : match[1] });
      if (items.length >= MAX_OUTLINE_ITEMS) break;
    }
    const minimum = Math.min(...items.map((item) => item.level), 1);
    return items.map((item, index) => ({ ...item, level: item.level - minimum + 1, id: `outline-${index + 1}-${hashText(item.title)}` }));
  }

  async function refreshOutline(force) {
    if (!isCurrent() || !state.open || state.activeTab !== "outline") return;
    const message = latestAssistant();
    const text = assistantText(message);
    const hash = hashText(text);
    if (!force && hash && hash === state.outlineHash && state.outlineStatus === "ready") return;
    state.latestMessage = message;
    state.outlineHash = hash;
    state.outlineStatus = "pending";
    state.outlineError = "";
    render();
    if (!message || !text) {
      state.outline = [];
      state.outlineStatus = "empty";
      render();
      return;
    }
    const response = await bridgeCall("/answer-outline/parse", { enabled: true, text, maxItems: MAX_OUTLINE_ITEMS });
    if (!isCurrent() || state.outlineHash !== hash) return;
    const parsed = Array.isArray(response?.items) ? response.items : [];
    state.outline = (parsed.length ? parsed : localOutline(text)).map((item, index) => ({
      ...item,
      id: item.id || `outline-${index + 1}-${hashText(item.title)}`,
    }));
    mapOutlineTargets(message);
    state.outlineStatus = state.outline.length ? "ready" : "empty";
    state.outlineError = response?.status === "failed" && !state.outline.length ? response.error || "Bridge 不可用" : "";
    render();
  }

  function mapOutlineTargets(message) {
    state.outlineTargets.clear();
    const root = message?.querySelector?.("[class*='markdown'], [class*='prose'], article") || message;
    const candidates = Array.from(root?.querySelectorAll?.("h1,h2,h3,h4,h5,h6,p,strong,b") || []);
    for (const item of state.outline) {
      const target = candidates.find((node) => normalizeText(node.innerText || node.textContent).replace(/^#+\s+/, "").replace(/^[\d一二三四五六七八九十]+[、.．)]\s*/, "") === item.title);
      if (target) {
        state.outlineTargets.set(item.id, target);
        target.setAttribute("data-codex-floating-outline-id", item.id);
      }
    }
  }

  function scrollContainer(element) {
    let node = element?.parentElement;
    while (node && node !== document.documentElement) {
      const style = getComputedStyle(node);
      if (/(auto|scroll|overlay)/.test(style.overflowY || style.overflow) && node.scrollHeight > node.clientHeight + 4) return node;
      node = node.parentElement;
    }
    return document.scrollingElement || document.documentElement;
  }

  function jumpTo(id) {
    const target = state.outlineTargets.get(id);
    if (!target) return;
    const container = scrollContainer(target);
    const targetRect = target.getBoundingClientRect();
    if (container === document.scrollingElement || container === document.documentElement || container === document.body) {
      window.scrollTo({ top: clamp(window.scrollY + targetRect.top - OUTLINE_TOP_OFFSET, 0, document.documentElement.scrollHeight), behavior: "smooth" });
    } else {
      const containerRect = container.getBoundingClientRect();
      container.scrollTo({ top: clamp(container.scrollTop + targetRect.top - containerRect.top - OUTLINE_TOP_OFFSET, 0, container.scrollHeight - container.clientHeight), behavior: "smooth" });
    }
    target.classList.add("codex-floating-outline-target");
    window.setTimeout(() => target.classList.remove("codex-floating-outline-target"), 900);
  }

  function jumpToAnchor(anchor) {
    const message = state.latestMessage || latestAssistant();
    if (!message) return;
    const target = anchor === "start" ? message : message.lastElementChild || message;
    if (anchor === "start") {
      const previous = message.previousElementSibling;
      if (previous?.getAttribute?.("data-message-author-role") === "user") previous.scrollIntoView({ behavior: "smooth", block: "start" });
    }
    target.scrollIntoView({ behavior: "smooth", block: anchor === "start" ? "start" : "end" });
  }

  function composer() {
    return document.querySelector("textarea, [contenteditable='true'].ProseMirror, [contenteditable='true']");
  }

  function fillComposer(prompt) {
    const target = composer();
    if (!target) return;
    if (target instanceof HTMLTextAreaElement) {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
      setter?.call(target, prompt);
    } else {
      target.textContent = prompt;
    }
    target.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: prompt }));
    target.focus();
  }

  function openManager() {
    bridgeCall("/manager/open-transient", { page: "settings", section: "stepwise" });
  }

  function beginDrag(event) {
    if (event.button !== 0 || event.target?.closest?.("button:not(.cfp-capsule), [data-action='resize']")) return;
    const rect = state.root.getBoundingClientRect();
    state.drag = { pointerId: event.pointerId, offsetX: event.clientX - rect.left, offsetY: event.clientY - rect.top };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function beginResize(event) {
    if (event.button !== 0) return;
    state.resize = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, width: state.size.width, height: state.size.height };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onPointerMove(event) {
    if (state.drag && event.pointerId === state.drag.pointerId) {
      state.position = { left: event.clientX - state.drag.offsetX, top: event.clientY - state.drag.offsetY };
      applyPosition();
    }
    if (state.resize && event.pointerId === state.resize.pointerId) {
      state.size = { width: state.resize.width + event.clientX - state.resize.startX, height: state.resize.height + event.clientY - state.resize.startY };
      applySize();
    }
  }

  function onPointerUp(event) {
    if (state.drag?.pointerId === event.pointerId) { state.drag = null; writeJson(POSITION_KEY, state.position); }
    if (state.resize?.pointerId === event.pointerId) { state.resize = null; writeJson(SIZE_KEY, state.size); }
  }

  function observe() {
    state.observer?.disconnect();
    state.observer = new MutationObserver((records) => {
      const touchesOutsidePanel = records.some((record) => !state.root?.contains(record.target));
      hideLegacyPanel();
      if (touchesOutsidePanel) render();
      if (state.open && state.activeTab === "outline") {
        window.clearTimeout(state.timer);
        state.timer = window.setTimeout(() => void refreshOutline(false), 180);
      }
    });
    state.observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  }

  function hideLegacyPanel() {
    document.querySelectorAll?.(`[${LEGACY_ROOT_ATTR}="true"]`).forEach((node) => {
      if (!node.classList.contains("cfp-legacy-hidden")) node.classList.add("cfp-legacy-hidden");
    });
  }

  function destroy() {
    state.destroyed = true;
    state.observer?.disconnect();
    state.root?.remove();
    document.getElementById(STYLE_ID)?.remove();
    document.querySelectorAll?.(".cfp-legacy-hidden").forEach((node) => node.classList.remove("cfp-legacy-hidden"));
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
    if (window[API_KEY]?.instanceId === instanceId) delete window[API_KEY];
  }

  async function start() {
    for (let attempt = 0; attempt < 40 && !state.destroyed; attempt += 1) {
      state.core = window[CORE_KEY]?.state || null;
      if (state.core?.settings) break;
      await new Promise((resolve) => window.setTimeout(resolve, 125));
    }
    if (state.destroyed || !state.core) return;
    if (!state.core.settings && typeof window[CORE_KEY]?.loadSettings === "function") {
      await window[CORE_KEY].loadSettings();
    }
    state.settings = state.core.settings || null;
    if (!runtimeEnabled()) return;
    if (!document.body) {
      document.addEventListener("DOMContentLoaded", () => void start(), { once: true });
      return;
    }
    installStyle();
    installRoot();
    hideLegacyPanel();
    observe();
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    render();
  }

  const instanceId = `floating-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  window[API_KEY] = { version: "1.0.0", instanceId, state, destroy, refreshOutline };
  void start();
})();
