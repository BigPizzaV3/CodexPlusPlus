(() => {
  "use strict";

  const VERSION = "0.6.8";
  const SOURCE_HASH = window.__CODEX_TASKBOARD_SOURCE_HASH__;
  const SENTINEL_KEY = "__codexTaskboardInjection__";
  const DEFAULT_TASKBOARD_URL = "http://127.0.0.1:47823/?host=codex";
  const ENTRY_ID = "codex-taskboard-entry";
  const PAGE_ID = "codex-taskboard-page";
  const FRAME_ID = "codex-taskboard-frame";
  const NATIVE_THREAD_PANEL_ID = "codex-taskboard-native-thread-panel";
  const NATIVE_THREAD_PANEL_BODY_ID = "codex-taskboard-native-thread-panel-body";
  const DRAG_REGION_ID = "codex-taskboard-drag-region";
  const NO_DRAG_LEFT_ID = "codex-taskboard-no-drag-left";
  const NO_DRAG_RIGHT_ID = "codex-taskboard-no-drag-right";
  const STATUS_ID = "codex-taskboard-status";
  const STYLE_ID = "codex-taskboard-inject-style";
  const OWNED_ATTRIBUTE = "data-codex-taskboard-owned";
  const HIDDEN_ATTRIBUTE = "data-codex-taskboard-native-hidden";
  const HOST_ATTRIBUTE = "data-codex-taskboard-page-host";
  const NATIVE_SELECTED_ATTRIBUTE = "data-codex-taskboard-native-selected";
  const NATIVE_THREAD_HOST_ATTRIBUTE = "data-codex-taskboard-native-thread-host";
  const HOST_BINDING_NAME = "__codexTaskboardHostV1";
  const HOST_HEARTBEAT_NAME = "__codexTaskboardHostHeartbeatV1";
  const REATTACH_DELAY_MS = 160;
  const FRAME_READY_TIMEOUT_MS = 12_000;
  const HOST_REQUEST_TIMEOUT_MS = 12_000;
  const HOST_HEARTBEAT_MAX_AGE_MS = 8_000;
  const MACOS_TITLEBAR_SAFE_LEFT = 80;
  const FRAME_REFRESH_PARAM = "__codex_taskboard_refresh";
  const PLUGIN_LABELS = ["插件", "plugins"];
  const NATIVE_PAGE_LABELS = [
    "新建任务",
    "新对话",
    "new task",
    "new chat",
    "new conversation",
    "拉取请求",
    "pull requests",
    "站点",
    "sites",
    "已安排",
    "scheduled",
    "插件",
    "plugins",
  ];
  const PROJECT_SECTION_LABELS = ["projects", "项目"];
  const TASK_SECTION_LABELS = ["tasks", "任务", "chats", "对话"];
  const CODEX_ICONS = {
    chevronDown: {
      viewBox: "0 0 16 16",
      content: `<path d="M4.53 5.47a.75.75 0 0 0-1.06 1.06l4 4a.75.75 0 0 0 1.054.007l4-3.903a.75.75 0 0 0-1.048-1.073l-3.47 3.385L4.53 5.47Z"></path>`,
    },
    close: {
      viewBox: "0 0 16 16",
      content: `<path d="M2.96967 2.96967C3.26256 2.67678 3.73744 2.67678 4.03033 2.96967L8 6.939L11.9697 2.96967C12.2626 2.67678 12.7374 2.67678 13.0303 2.96967C13.3232 3.26256 13.3232 3.73744 13.0303 4.03033L9.061 8L13.0303 11.9697C13.2966 12.2359 13.3208 12.6526 13.1029 12.9462L13.0303 13.0303C12.7374 13.3232 12.2626 13.3232 11.9697 13.0303L8 9.061L4.03033 13.0303C3.73744 13.3232 3.26256 13.3232 2.96967 13.0303C2.67678 12.7374 2.67678 12.2626 2.96967 11.9697L6.939 8L2.96967 4.03033C2.7034 3.76406 2.6792 3.3474 2.89705 3.05379L2.96967 2.96967Z"></path>`,
    },
    expand: {
      viewBox: "0 0 16 16",
      content: `<path d="M6.2168 8.72266C6.50798 8.42824 6.98279 8.4257 7.27734 8.7168C7.57154 9.00799 7.57423 9.48287 7.2832 9.77734L4.59863 12.5H6.2998C6.71402 12.5 7.0498 12.8358 7.0498 13.25C7.04964 13.6641 6.71392 14 6.2998 14H2.75C2.55116 14 2.36036 13.9208 2.21973 13.7803C2.07915 13.6397 2.00008 13.4488 2 13.25V9.75C2 9.33579 2.33579 9 2.75 9C3.16421 9 3.5 9.33579 3.5 9.75V11.4775L6.2168 8.72266Z"></path><path d="M13.25 2C13.4488 2.00006 13.6397 2.07917 13.7803 2.21973C13.9208 2.36033 14 2.55119 14 2.75V6.25C14 6.66414 13.6641 6.99988 13.25 7C12.8358 7 12.5 6.66421 12.5 6.25V4.52246L9.7832 7.27734C9.49206 7.57173 9.01721 7.57419 8.72266 7.2832C8.42838 6.99201 8.42575 6.51716 8.7168 6.22266L11.4014 3.5H9.7002C9.28598 3.5 8.9502 3.16421 8.9502 2.75C8.95028 2.33586 9.28603 2 9.7002 2H13.25Z"></path>`,
    },
    openExternal: {
      viewBox: "0 0 16 16",
      content: `<path d="M6.413 1.313c.241 0 .472-.005.643.166.17.17.266.342.266.583a.817.817 0 0 1-.266.602c-.17.17-.402.166-.643.166h-1.43c-.635-.018-1.144.292-1.485.633-.34.34-.632 1.003-.632 1.485v6.06c0 .481.192 1.044.532 1.384.341.341.904.733 1.386.733h6.259c.482 0 1.144-.292 1.485-.633.341-.34.633-1.003.633-1.485V9.721c0-.24-.005-.472.166-.642.17-.17.301-.267.542-.267a.91.91 0 0 1 .643.267c.17.17.166.401.166.642v1.286a3.635 3.635 0 0 1-3.449 3.63c-.072.004-.113.004-.186.005-2.366.06-3.694.06-6.06 0-.072-.001-.113 0-.186-.004a3.636 3.636 0 0 1-3.444-3.444c-.004-.073-.003-.114-.005-.187-.06-2.365 0-3.693 0-6.059a3.635 3.635 0 0 1 3.636-3.635h1.429Z"></path><path d="M14.675 6.063a.75.75 0 1 1-1.5 0v-2.19l-4.148 4.5a.75.75 0 1 1-1.06-1.06l4.148-4.5h-2.19a.75.75 0 0 1 0-1.5h4a.75.75 0 0 1 .75.75v4Z"></path>`,
    },
    panel: {
      viewBox: "0 0 16 16",
      content: `<path fill-rule="evenodd" clip-rule="evenodd" d="M4.25 2C2.45508 2 1 3.45508 1 5.25V10.75C1 12.5449 2.45508 14 4.25 14H11.75C13.5449 14 15 12.5449 15 10.75V5.25C15 3.45508 13.5449 2 11.75 2H4.25ZM2.5 5.5C2.5 4.39543 3.39543 3.5 4.5 3.5H11.5C12.6046 3.5 13.5 4.39543 13.5 5.5V10.5C13.5 11.6046 12.6046 12.5 11.5 12.5H4.5C3.39543 12.5 2.5 11.6046 2.5 10.5V5.5Z"></path><rect x="10" y="5" width="1.5" height="6" rx=".75"></rect>`,
    },
  };

  const previous = window[SENTINEL_KEY];
  if (previous?.sourceHash === SOURCE_HASH && typeof previous.refresh === "function") {
    previous.refresh();
    return;
  }
  try {
    previous?.destroy?.();
  } catch (_) {}

  let entry = null;
  let page = null;
  let frame = null;
  let dragRegion = null;
  let noDragLeft = null;
  let noDragRight = null;
  let status = null;
  let frameOrigin = "";
  let frameReady = false;
  let frameReadyWaiters = new Set();
  let hostRequests = new Map();
  let hostRequestSequence = 0;
  let observer = null;
  let reattachTimer = null;
  let lastFocusedElement = null;
  let hostContextSnapshot = null;
  let mutedNativeSelections = new Map();
  let openGeneration = 0;
  let pendingThreadCreation = null;
  let lastNativeThreadId = "";
  let active = false;
  let destroyed = false;
  let nativeThreadPanel = null;
  let nativeThreadPanelBody = null;
  let nativeThreadPanelHost = null;
  let nativeThreadPanelHostStyle = "";
  let nativeThreadPanelSize = "compact";
  let nativeThreadPanelThreadId = "";
  let nativeThreadPanelWorkspacePath = "";
  let nativeThreadPanelThreadTitle = "";
  let nativeThreadPanelCanAttach = false;
  let nativeThreadPanelResizeObserver = null;
  let suppressNativeCloseUntil = 0;

  function normalizedLabel(value) {
    return String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
  }

  function normalizeThreadId(value) {
    return String(value || "").trim().replace(/^(?:local|cloud):/i, "");
  }

  function resolveTaskboardUrl() {
    const configured = typeof window.__CODEX_TASKBOARD_URL__ === "string"
      ? window.__CODEX_TASKBOARD_URL__.trim()
      : "";
    try {
      const url = new URL(configured || DEFAULT_TASKBOARD_URL);
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        throw new Error("Unsupported taskboard URL protocol");
      }
      if (!url.searchParams.has("host")) url.searchParams.set("host", "codex");
      return url;
    } catch (_) {
      return new URL(DEFAULT_TASKBOARD_URL);
    }
  }

  function isLocalTaskboardOrigin(origin) {
    try {
      const { protocol, hostname } = new URL(origin);
      return (protocol === "http:" || protocol === "https:")
        && (hostname === "127.0.0.1" || hostname === "localhost");
    } catch (_) {
      return false;
    }
  }

  function installStyles() {
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.setAttribute(OWNED_ATTRIBUTE, "true");
    style.textContent = `
      #${ENTRY_ID}[aria-current="page"] {
        background: var(--color-token-list-hover-background, color-mix(in srgb, currentColor 8%, transparent));
        color: var(--color-token-foreground, inherit);
      }
      #${ENTRY_ID}:focus-visible {
        outline: 2px solid var(--color-token-border, Highlight);
        outline-offset: 2px;
      }
      [${HOST_ATTRIBUTE}="true"] {
        position: relative !important;
        z-index: 31 !important;
        pointer-events: none !important;
      }
      [${HIDDEN_ATTRIBUTE}="true"] {
        visibility: hidden !important;
        pointer-events: none !important;
      }
      [${NATIVE_SELECTED_ATTRIBUTE}="true"] {
        background-color: transparent !important;
      }
      [${NATIVE_SELECTED_ATTRIBUTE}="true"] [class*="text-token-list-active-selection"] {
        color: var(--color-token-foreground, inherit) !important;
      }
      #${PAGE_ID} {
        position: absolute;
        top: 0;
        right: 0;
        bottom: 0;
        left: 0;
        z-index: 1;
        min-width: 0;
        min-height: 0;
        overflow: hidden;
        background: Canvas;
        color: CanvasText;
        pointer-events: auto;
      }
      #${PAGE_ID}[hidden] {
        display: none !important;
      }
      #${FRAME_ID} {
        display: block;
        width: 100%;
        height: 100%;
        border: 0;
        background: Canvas;
      }
      #${FRAME_ID}[hidden] {
        display: none !important;
      }
      #${DRAG_REGION_ID} {
        position: absolute;
        z-index: 2;
        background: transparent;
        pointer-events: none;
        -webkit-app-region: drag;
      }
      #${NO_DRAG_LEFT_ID},
      #${NO_DRAG_RIGHT_ID} {
        position: absolute;
        z-index: 2;
        background: transparent;
        pointer-events: none;
        -webkit-app-region: no-drag;
      }
      #${DRAG_REGION_ID}[hidden],
      #${NO_DRAG_LEFT_ID}[hidden],
      #${NO_DRAG_RIGHT_ID}[hidden] {
        display: none !important;
      }
      #${STATUS_ID} {
        position: absolute;
        inset: 0;
        display: grid;
        place-items: center;
        padding: 24px;
        color: var(--color-token-text-secondary, color-mix(in srgb, CanvasText 60%, transparent));
        font: 13px/1.5 system-ui, sans-serif;
        text-align: center;
      }
      #${STATUS_ID}[hidden] {
        display: none !important;
      }
      #${STATUS_ID} button {
        margin-top: 10px;
        border: 1px solid var(--color-token-border, color-mix(in srgb, CanvasText 16%, transparent));
        border-radius: 7px;
        padding: 5px 10px;
        background: var(--color-token-main-surface-secondary, Canvas);
        color: var(--color-token-foreground, CanvasText);
        cursor: pointer;
      }
      #${NATIVE_THREAD_PANEL_ID} {
        position: fixed;
        right: 12px;
        bottom: 12px;
        width: min(440px, calc(100vw - 24px));
        height: min(620px, calc(100vh - 24px));
        max-width: calc(100vw - 24px);
        max-height: calc(100vh - 24px);
        box-sizing: border-box;
        z-index: 8;
        display: grid;
        grid-template-rows: 38px minmax(0, 1fr);
        pointer-events: none;
        color: var(--color-token-foreground, CanvasText);
        font: 13px/1.4 system-ui, sans-serif;
      }
      #${NATIVE_THREAD_PANEL_ID}[hidden] {
        display: none !important;
      }
      #${NATIVE_THREAD_PANEL_ID}[data-size="maximized"] {
        inset: 12px;
        width: auto;
        height: auto;
        grid-template-rows: 40px minmax(0, 1fr);
      }
      #${NATIVE_THREAD_PANEL_ID} .codex-taskboard-native-thread-header {
        display: flex;
        align-items: center;
        gap: 6px;
        min-width: 0;
        min-height: 0;
        padding: 0 7px 0 11px;
        border: 1px solid var(--color-token-border, color-mix(in srgb, CanvasText 14%, transparent));
        border-bottom: 0;
        border-radius: 14px 14px 0 0;
        background: var(--color-token-main-surface-primary, Canvas);
        box-shadow: 0 10px 30px color-mix(in srgb, CanvasText 15%, transparent);
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      #${NATIVE_THREAD_PANEL_ID} .codex-taskboard-native-thread-title {
        min-width: 0;
        flex: 1;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        font-weight: 650;
        font-size: 13px;
      }
      #${NATIVE_THREAD_PANEL_ID} .codex-taskboard-native-thread-id {
        display: none;
        color: var(--color-token-text-secondary, color-mix(in srgb, CanvasText 58%, transparent));
        font-size: 12px;
        font-weight: 500;
      }
      #${NATIVE_THREAD_PANEL_ID} button {
        border: 1px solid transparent;
        border-radius: 8px;
        width: 28px;
        height: 28px;
        padding: 0;
        background: transparent;
        color: var(--color-token-text-secondary, color-mix(in srgb, CanvasText 68%, transparent));
        cursor: pointer;
        display: grid;
        place-items: center;
        flex: 0 0 auto;
        font: 16px/1 system-ui, sans-serif;
      }
      #${NATIVE_THREAD_PANEL_ID} button:hover {
        background: var(--color-token-list-hover-background, color-mix(in srgb, CanvasText 8%, transparent));
        color: var(--color-token-foreground, CanvasText);
      }
      #${NATIVE_THREAD_PANEL_ID} button svg {
        display: block;
        width: 16px;
        height: 16px;
      }
      #${NATIVE_THREAD_PANEL_BODY_ID} {
        min-width: 0;
        min-height: 0;
        border-radius: 0 0 14px 14px;
        pointer-events: none;
      }
      #${NATIVE_THREAD_PANEL_ID}[data-native-host="false"] #${NATIVE_THREAD_PANEL_BODY_ID} {
        display: grid;
        place-items: center;
        padding: 20px;
        border: 1px solid var(--color-token-border, color-mix(in srgb, CanvasText 14%, transparent));
        border-top: 0;
        background: var(--color-token-main-surface-primary, Canvas);
        box-shadow: 0 14px 42px color-mix(in srgb, CanvasText 16%, transparent);
        pointer-events: auto;
      }
      #${NATIVE_THREAD_PANEL_ID} .codex-taskboard-native-thread-fallback {
        max-width: 320px;
        display: grid;
        justify-items: center;
        gap: 12px;
        color: var(--color-token-text-secondary, color-mix(in srgb, CanvasText 62%, transparent));
        text-align: center;
      }
      #${NATIVE_THREAD_PANEL_ID} .codex-taskboard-native-thread-fallback p {
        margin: 0;
        font: 13px/1.55 system-ui, sans-serif;
      }
      #${NATIVE_THREAD_PANEL_ID} .codex-taskboard-native-thread-fallback button {
        width: auto;
        height: 32px;
        padding: 0 12px;
        border: 1px solid var(--color-token-border, color-mix(in srgb, CanvasText 16%, transparent));
        border-radius: 8px;
        background: var(--color-token-main-surface-secondary, Canvas);
        color: var(--color-token-foreground, CanvasText);
        font: 13px/1 system-ui, sans-serif;
      }
      [${NATIVE_THREAD_HOST_ATTRIBUTE}="true"] {
        visibility: visible !important;
        pointer-events: auto !important;
        margin: 0 !important;
        min-width: 0 !important;
        min-height: 0 !important;
        overflow: hidden !important;
        background: var(--color-token-main-surface-primary, Canvas) !important;
        border: 1px solid var(--color-token-border, color-mix(in srgb, CanvasText 14%, transparent)) !important;
        border-top: 0 !important;
        border-radius: 0 0 14px 14px !important;
        box-shadow: 0 14px 42px color-mix(in srgb, CanvasText 16%, transparent) !important;
      }
      @media (max-width: 720px) {
        #${NATIVE_THREAD_PANEL_ID} {
          right: 8px;
          bottom: 8px;
          width: calc(100vw - 16px);
          max-width: calc(100vw - 16px);
        }
        #${NATIVE_THREAD_PANEL_ID}[data-size="maximized"] {
          inset: 6px;
        }
      }
      @media (max-height: 680px) {
        #${NATIVE_THREAD_PANEL_ID} {
          bottom: 8px;
          height: calc(100vh - 16px);
          max-height: calc(100vh - 16px);
        }
        #${NATIVE_THREAD_PANEL_ID}[data-size="maximized"] {
          inset: 6px;
        }
      }
    `;
    (document.head || document.documentElement).appendChild(style);
  }

  function buttonMatches(button, labels) {
    if (!button) return false;
    const text = normalizedLabel(button.textContent || button.getAttribute("aria-label"));
    return labels.includes(text);
  }

  function findReferenceButton() {
    const scroll = document.querySelector("[data-app-action-sidebar-scroll]");
    if (!scroll) return null;
    const buttons = Array.from(scroll.querySelectorAll("button"));
    const plugin = buttons.find((button) => buttonMatches(button, PLUGIN_LABELS));
    if (plugin && plugin.parentElement) {
      const siblings = Array.from(plugin.parentElement.children).filter((child) => child.tagName === "BUTTON");
      if (siblings.length >= 3) return plugin;
    }

    const firstSection = scroll.querySelector("[data-app-action-sidebar-section]");
    const sectionTop = firstSection?.getBoundingClientRect().top ?? Number.POSITIVE_INFINITY;
    const groups = Array.from(scroll.querySelectorAll("div")).filter((element) => {
      const directButtons = Array.from(element.children).filter((child) => child.tagName === "BUTTON");
      return directButtons.length >= 3 && element.getBoundingClientRect().top < sectionTop;
    });
    const group = groups.sort((left, right) => right.children.length - left.children.length)[0];
    return Array.from(group?.children || []).filter((child) => child.tagName === "BUTTON").at(-1) || null;
  }

  function applyCodexIcon(svg, name) {
    const icon = CODEX_ICONS[name];
    if (!icon) return;
    svg.setAttribute("viewBox", icon.viewBox);
    svg.setAttribute("fill", "currentColor");
    svg.removeAttribute("stroke");
    svg.removeAttribute("stroke-width");
    svg.removeAttribute("stroke-linecap");
    svg.removeAttribute("stroke-linejoin");
    svg.style.fill = "currentColor";
    svg.style.stroke = "none";
    svg.innerHTML = icon.content;
  }

  function createCodexIcon(name) {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("width", "16");
    svg.setAttribute("height", "16");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("focusable", "false");
    applyCodexIcon(svg, name);
    return svg;
  }

  function setIconButton(button, name) {
    button.replaceChildren(createCodexIcon(name));
  }

  function replaceEntryIcon(button) {
    const icon = button.querySelector("svg");
    if (!icon) return;
    applyCodexIcon(icon, "panel");
  }

  function createEntry(reference) {
    const button = reference.cloneNode(true);
    button.id = ENTRY_ID;
    button.type = "button";
    button.removeAttribute("disabled");
    button.removeAttribute("aria-expanded");
    button.removeAttribute("aria-controls");
    button.removeAttribute("aria-describedby");
    button.removeAttribute("data-state");
    button.setAttribute("aria-label", "打开任务面板");
    button.setAttribute("title", "任务面板");
    button.setAttribute(OWNED_ATTRIBUTE, "true");
    button.querySelectorAll("[id]").forEach((node) => node.removeAttribute("id"));
    const label = button.querySelector(".text-fade-truncate")
      || Array.from(button.querySelectorAll("span")).find((node) => buttonMatches(node, PLUGIN_LABELS));
    if (label) label.textContent = "任务面板";
    else button.textContent = "任务面板";
    replaceEntryIcon(button);
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openTaskboard();
    });
    return button;
  }

  function syncEntryState() {
    if (!entry) return;
    if (active && entry.getAttribute("aria-current") !== "page") {
      entry.setAttribute("aria-current", "page");
    } else if (!active && entry.hasAttribute("aria-current")) {
      entry.removeAttribute("aria-current");
    }
  }

  function ensureEntry() {
    if (destroyed || !document.body) return;
    installStyles();
    const reference = findReferenceButton();
    if (!reference?.parentElement) return;
    if (!entry) entry = createEntry(reference);
    if (entry.parentElement !== reference.parentElement || entry.previousElementSibling !== reference) {
      reference.after(entry);
    }
    syncEntryState();
  }

  function findPageHost() {
    const direct = document.querySelector(".app-shell-main-content-frame");
    if (direct?.closest?.("[data-app-shell-main-content-layout]")) return direct;

    const viewport = document.querySelector("[data-app-shell-main-content-layout]");
    if (!viewport) return null;
    const viewportRect = viewport.getBoundingClientRect();
    const headerBottom = document.querySelector("main > header")?.getBoundingClientRect().bottom
      ?? viewportRect.top;
    return Array.from(viewport.children).find((candidate) => {
      const rect = candidate.getBoundingClientRect();
      return rect.width >= viewportRect.width * 0.8
        && rect.height >= viewportRect.height * 0.7
        && rect.top >= headerBottom - 1;
    }) || null;
  }

  function findPageMount() {
    const frameHost = findPageHost();
    const viewport = frameHost?.closest?.("[data-app-shell-main-content-layout]");
    const surface = viewport?.parentElement;
    if (!frameHost || !viewport || !surface || !surface.closest("main")) return null;
    return { frameHost, surface };
  }

  function muteNativeSelection() {
    if (!active) return;
    document.querySelectorAll('aside nav[role="navigation"] [aria-current]')
      .forEach((node) => {
        if (node === entry || node.closest(`#${ENTRY_ID}`)) return;
        if (!mutedNativeSelections.has(node)) {
          mutedNativeSelections.set(node, node.getAttribute("aria-current"));
        }
        node.removeAttribute("aria-current");
        node.setAttribute(NATIVE_SELECTED_ATTRIBUTE, "true");
      });
  }

  function restoreNativeSelection() {
    mutedNativeSelections.forEach((ariaCurrent, node) => {
      if (!node.isConnected) return;
      node.setAttribute("aria-current", ariaCurrent);
      node.removeAttribute(NATIVE_SELECTED_ATTRIBUTE);
    });
    mutedNativeSelections.clear();
    document.querySelectorAll(`[${NATIVE_SELECTED_ATTRIBUTE}="true"]`)
      .forEach((node) => node.removeAttribute(NATIVE_SELECTED_ATTRIBUTE));
  }

  function hideNativeHeader() {
    document.querySelectorAll('[data-testid="app-shell-header-context-menu-surface"]')
      .forEach((surface) => {
        Array.from(surface.children).forEach((child) => {
          if (child.getAttribute(OWNED_ATTRIBUTE) !== "true") {
            child.setAttribute(HIDDEN_ATTRIBUTE, "true");
          }
        });
      });
  }

  function currentTheme() {
    const root = document.documentElement;
    const explicit = String(root.dataset.theme || root.getAttribute("data-color-theme") || "").toLowerCase();
    if (explicit.includes("dark") || root.classList.contains("dark")) return "dark";
    if (explicit.includes("light") || root.classList.contains("light")) return "light";
    try {
      return window.getComputedStyle(root).colorScheme.includes("dark") ? "dark" : "light";
    } catch (_) {
      return "light";
    }
  }

  function threadIdFromLocation() {
    const source = `${window.location.pathname || ""}${window.location.search || ""}${window.location.hash || ""}`;
    const match = source.match(/(?:session|conversation|thread)(?:\/|=|:|-)([A-Za-z0-9_.-]+)/i)
      || source.match(/\/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})(?:[/?#]|$)/)
      || source.match(/\/([A-Za-z0-9_-]{24,})(?:[/?#]|$)/);
    return match ? decodeURIComponent(match[1]) : "";
  }

  function activeThreadRow() {
    const rows = Array.from(document.querySelectorAll("[data-app-action-sidebar-thread-id]"));
    return rows.find((row) => row.getAttribute("data-app-action-sidebar-thread-active") === "true")
      || rows.find((row) => ["page", "true"].includes(row.getAttribute("aria-current")))
      || null;
  }

  function readCodexProjects() {
    const seen = new Set();
    return Array.from(document.querySelectorAll("[data-app-action-sidebar-project-row]"))
      .flatMap((row) => {
        const id = row.getAttribute("data-app-action-sidebar-project-id")?.trim();
        const name = (
          row.getAttribute("data-app-action-sidebar-project-label")
          || row.getAttribute("aria-label")
          || ""
        ).trim();
        if (!id || !name || seen.has(id)) return [];
        seen.add(id);
        return [{ id, name }];
      });
  }

  function readVisibleThreadIds(scope) {
    const seen = new Set();
    const root = scope && typeof scope.querySelectorAll === "function" ? scope : document;
    return Array.from(root.querySelectorAll("[data-app-action-sidebar-thread-id]"))
      .flatMap((row) => {
        const threadId = normalizeThreadId(row.getAttribute("data-app-action-sidebar-thread-id"));
        if (!threadId || threadId.startsWith("client-new-thread:") || seen.has(threadId)) return [];
        seen.add(threadId);
        return [threadId];
      })
      .sort();
  }

  function findProjectsSection() {
    return Array.from(document.querySelectorAll("[data-app-action-sidebar-section-heading]"))
      .find((node) => PROJECT_SECTION_LABELS.includes(normalizedLabel(
        node.getAttribute("data-app-action-sidebar-section-heading") || node.textContent,
      )))
      ?.closest("[data-app-action-sidebar-section]") || null;
  }

  function findTasksSection() {
    return Array.from(document.querySelectorAll("[data-app-action-sidebar-section]"))
      .find((section) => {
        const heading = section.querySelector("[data-app-action-sidebar-section-heading]");
        const label = heading?.getAttribute("data-app-action-sidebar-section-heading")
          || heading?.textContent
          || section.textContent;
        return TASK_SECTION_LABELS.includes(normalizedLabel(label));
      }) || null;
  }

  async function captureHostContext() {
    let projects = readCodexProjects();
    let section = findProjectsSection();
    const sectionDeadline = Date.now() + 1_200;
    while (!section && Date.now() < sectionDeadline) {
      await new Promise((resolve) => window.setTimeout(resolve, 40));
      section = findProjectsSection();
    }
    const tasksSection = findTasksSection();
    const expandedSections = [section, tasksSection].filter((candidate) => (
      candidate?.getAttribute("data-app-action-sidebar-section-collapsed") === "true"
    ));
    expandedSections.forEach((candidate) => (
      candidate.querySelector("[data-app-action-sidebar-section-toggle]")?.click()
    ));
    if (expandedSections.length > 0) {
      const deadline = Date.now() + 1_200;
      do {
        await new Promise((resolve) => window.setTimeout(resolve, 40));
        projects = readCodexProjects();
      } while ((projects.length === 0 || !activeThreadRow()) && Date.now() < deadline);
    }
    const context = readHostContext(projects);
    expandedSections.forEach((candidate) => {
      if (candidate.isConnected && candidate.getAttribute("data-app-action-sidebar-section-collapsed") === "false") {
        candidate.querySelector("[data-app-action-sidebar-section-toggle]")?.click();
      }
    });
    return context;
  }

  function workspaceFromLocation() {
    try {
      const url = new URL(window.location.href);
      return url.searchParams.get("workspace") || url.searchParams.get("cwd") || "";
    } catch (_) {
      return "";
    }
  }

  function titlebarLeftInset() {
    if (!/Macintosh|Mac OS X/.test(navigator.userAgent)) return 0;
    if (nativeSidebarCollapsed()) return MACOS_TITLEBAR_SAFE_LEFT;
    const surfaceLeft = findPageMount()?.surface.getBoundingClientRect().left;
    if (!Number.isFinite(surfaceLeft)) return 0;
    return Math.max(0, Math.ceil(MACOS_TITLEBAR_SAFE_LEFT - surfaceLeft));
  }

  function nativeSidebarTrigger() {
    const triggers = Array.from(
      document.querySelectorAll('[data-app-shell-sidebar-trigger="true"]'),
    );
    return triggers.find((trigger) => getComputedStyle(trigger).visibility !== "hidden")
      || triggers[0]
      || null;
  }

  function nativeSidebarCollapsed() {
    const label = normalizedLabel(nativeSidebarTrigger()?.getAttribute("aria-label"));
    return label.startsWith("显示") || label.startsWith("show ");
  }

  function expandNativeSidebar() {
    const trigger = nativeSidebarTrigger();
    if (!trigger || !nativeSidebarCollapsed()) return;
    trigger.click();
    window.setTimeout(postHostContext, REATTACH_DELAY_MS);
  }

  function userIdFromName(name) {
    const slug = name.normalize("NFKD")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 96);
    if (slug) return slug;
    let hash = 2166136261;
    for (const character of name) {
      hash ^= character.codePointAt(0);
      hash = Math.imul(hash, 16777619);
    }
    return `codex-user-${(hash >>> 0).toString(36)}`;
  }

  function readCodexUser() {
    const avatar = Array.from(document.querySelectorAll("img"))
      .find((image) => image.src.includes("cdn.auth0.com/avatars/"));
    const profileButton = avatar?.closest("button")
      || Array.from(document.querySelectorAll('button[aria-haspopup="menu"]')).find((button) => (
        normalizedLabel(button.getAttribute("aria-label")).includes("profile")
        || normalizedLabel(button.getAttribute("aria-label")).includes("个人资料")
      ));
    const name = profileButton?.textContent?.replace(/\s+/g, " ").trim();
    if (!name) return null;
    const avatarUrl = avatar?.currentSrc || avatar?.src || null;
    return {
      type: "user",
      id: userIdFromName(name),
      name,
      avatarUrl,
    };
  }

  function readHostContext(projects = readCodexProjects()) {
    const row = activeThreadRow();
    const activeThreadId = normalizeThreadId(row?.getAttribute("data-app-action-sidebar-thread-id"));
    if (activeThreadId) lastNativeThreadId = activeThreadId;
    const threadId = activeThreadId || lastNativeThreadId || normalizeThreadId(threadIdFromLocation());
    const projectList = row?.closest?.("[data-app-action-sidebar-project-list-id]");
    const projectRow = row?.closest?.("[data-app-action-sidebar-project-id]")
      || document.querySelector('[data-app-action-sidebar-project-row][aria-current="page"]')
      || document.querySelector('[data-app-action-sidebar-project-row][data-app-action-sidebar-project-active="true"]');
    const projectId = projectList?.getAttribute("data-app-action-sidebar-project-list-id")
      || projectRow?.getAttribute("data-app-action-sidebar-project-id")
      || "";
    const workspacePath = workspaceFromLocation();
    const payload = {
      theme: currentTheme(),
      projects,
      user: readCodexUser() ?? undefined,
      visibleThreadIds: readVisibleThreadIds(projectList),
      titlebarLeftInset: titlebarLeftInset(),
      sidebarCollapsed: nativeSidebarCollapsed(),
    };
    if (workspacePath) payload.workspacePath = workspacePath;
    if (projectId) payload.projectId = projectId;
    if (threadId) payload.threadId = threadId;
    return payload;
  }

  function postToFrame(message) {
    if (!frame?.contentWindow || !frameOrigin) return;
    frame.contentWindow.postMessage(message, frameOrigin);
  }

  function dispatchHostMessage(message) {
    window.postMessage(message, window.location.origin);
  }

  function postHostContext() {
    if (!frame) return;
    const liveContext = readHostContext();
    const payload = hostContextSnapshot
      ? {
          ...hostContextSnapshot,
          ...liveContext,
          projects: liveContext.projects.length > 0
            ? liveContext.projects
            : hostContextSnapshot.projects,
        }
      : liveContext;
    postToFrame({ type: "taskboard:host-context", payload });
    postToFrame({ type: "taskboard:theme", theme: payload.theme });
  }

  function threadRowTitle(row) {
    return typeof row?.getAttribute === "function"
      ? row.getAttribute("data-app-action-sidebar-thread-title") || ""
      : "";
  }

  function findThreadRow(threadId, threadTitle = "") {
    const normalizedThreadId = normalizeThreadId(threadId);
    const rows = Array.from(document.querySelectorAll("[data-app-action-sidebar-thread-id]"));
    if (normalizedThreadId) {
      const exact = rows.find((row) => normalizeThreadId(row.getAttribute("data-app-action-sidebar-thread-id")) === normalizedThreadId);
      if (exact) return exact;
    }
    const normalizedThreadTitle = normalizedLabel(threadTitle);
    if (!normalizedThreadTitle) return null;
    return rows.find((row) => normalizedLabel(threadRowTitle(row)) === normalizedThreadTitle) || null;
  }

  function routeForThread(threadId) {
    return `/local/${encodeURIComponent(threadId)}`;
  }

  function deepLinkForThread(threadId) {
    const normalizedThreadId = normalizeThreadId(threadId);
    return normalizedThreadId ? `codex://threads/${encodeURIComponent(normalizedThreadId)}` : "";
  }

  function openThreadDeepLink(threadId) {
    const deepLink = deepLinkForThread(threadId);
    if (deepLink) window.location.assign(deepLink);
  }

  function normalizeWorkspacePath(value) {
    return typeof value === "string" ? value.trim() : "";
  }

  async function setActiveWorkspaceRoot(workspacePath) {
    const root = normalizeWorkspacePath(workspacePath);
    if (!root) return false;
    const bridge = window.electronBridge;
    if (!bridge || typeof bridge.sendMessageFromView !== "function") return false;
    await bridge.sendMessageFromView({
      type: "electron-set-active-workspace-root",
      root,
    });
    return true;
  }

  function nativeThreadIsActive(threadId, threadTitle = "") {
    const normalizedThreadId = normalizeThreadId(threadId);
    if (!normalizedThreadId) return false;
    const normalizedThreadTitle = normalizedLabel(threadTitle);
    const activeThreadId = normalizeThreadId(
      activeThreadRow()?.getAttribute("data-app-action-sidebar-thread-id"),
    );
    return activeThreadId === normalizedThreadId
      || normalizeThreadId(threadIdFromLocation()) === normalizedThreadId
      || (
        normalizedThreadTitle
        && normalizedLabel(threadRowTitle(activeThreadRow())) === normalizedThreadTitle
      );
  }

  async function waitForNativeThread(threadId, threadTitle = "", timeoutMs = 1_500) {
    const deadline = Date.now() + timeoutMs;
    do {
      if (nativeThreadIsActive(threadId, threadTitle)) return true;
      await new Promise((resolve) => window.setTimeout(resolve, 80));
    } while (Date.now() < deadline);
    return nativeThreadIsActive(threadId, threadTitle);
  }

  async function waitForThreadRow(threadId, threadTitle = "", timeoutMs = 800) {
    const deadline = Date.now() + timeoutMs;
    do {
      const row = findThreadRow(threadId, threadTitle);
      if (row?.isConnected) return row;
      await new Promise((resolve) => window.setTimeout(resolve, 80));
    } while (Date.now() < deadline);
    return findThreadRow(threadId, threadTitle);
  }

  async function navigateNativeThread(
    threadId,
    { closeCurrentTaskboard, workspacePath, threadTitle } = { closeCurrentTaskboard: false, workspacePath: "", threadTitle: "" },
  ) {
    if (typeof threadId !== "string" || !threadId.trim()) return false;
    const normalizedThreadId = normalizeThreadId(threadId);
    lastNativeThreadId = normalizedThreadId;
    const normalizedWorkspacePath = normalizeWorkspacePath(workspacePath);
    const normalizedThreadTitle = normalizedLabel(threadTitle);
    if (normalizedWorkspacePath) {
      try {
        await setActiveWorkspaceRoot(normalizedWorkspacePath);
      } catch (_) {}
    }
    const row = normalizedWorkspacePath
      ? await waitForThreadRow(normalizedThreadId, normalizedThreadTitle)
      : findThreadRow(normalizedThreadId, normalizedThreadTitle);
    if (closeCurrentTaskboard) closeTaskboard(false);

    if (row?.isConnected) {
      if (!closeCurrentTaskboard) suppressNativeCloseUntil = Date.now() + 1_000;
      row.click?.();
      return true;
    }

    try {
      if (!closeCurrentTaskboard) suppressNativeCloseUntil = Date.now() + 1_000;
      await dispatchHostMessage({
        type: "navigate-to-route",
        path: routeForThread(normalizedThreadId),
      });
    } catch (_) {
      return false;
    }
    return waitForNativeThread(normalizedThreadId, normalizedThreadTitle);
  }

  async function openThread(threadId, workspacePath = "", threadTitle = "") {
    const targetWorkspacePath = normalizeWorkspacePath(workspacePath) || nativeThreadPanelWorkspacePath;
    const targetThreadTitle = normalizedLabel(threadTitle) || nativeThreadPanelThreadTitle;
    const opened = await navigateNativeThread(threadId, {
      closeCurrentTaskboard: true,
      workspacePath: targetWorkspacePath,
      threadTitle: targetThreadTitle,
    });
    if (!opened) openThreadDeepLink(threadId);
  }

  function restoreNativeThreadHost() {
    nativeThreadPanelResizeObserver?.disconnect?.();
    nativeThreadPanelResizeObserver = null;
    if (!nativeThreadPanelHost) return;
    nativeThreadPanelHost.style.cssText = nativeThreadPanelHostStyle;
    nativeThreadPanelHost.removeAttribute(NATIVE_THREAD_HOST_ATTRIBUTE);
    nativeThreadPanelHost = null;
    nativeThreadPanelHostStyle = "";
  }

  function clearNativeThreadFallback() {
    if (nativeThreadPanel) nativeThreadPanel.dataset.nativeHost = "true";
    nativeThreadPanelBody?.replaceChildren();
  }

  function showNativeThreadFallback(threadId) {
    if (!nativeThreadPanel || !nativeThreadPanelBody) return;
    nativeThreadPanel.dataset.nativeHost = "false";
    nativeThreadPanelBody.replaceChildren();

    const fallback = document.createElement("div");
    fallback.className = "codex-taskboard-native-thread-fallback";

    const message = document.createElement("p");
    message.textContent = "当前 Codex 侧栏没有加载这个原对话，已停止挂载当前新对话页面。请完整打开后继续。";

    const action = document.createElement("button");
    action.type = "button";
    action.textContent = "完整打开";
    action.addEventListener("click", () => void openThread(threadId, "", nativeThreadPanelThreadTitle));

    fallback.append(message, action);
    nativeThreadPanelBody.append(fallback);
  }

  function mountNativeThreadPanel() {
    const mount = findPageMount();
    if (!mount || !nativeThreadPanel) return null;
    if (nativeThreadPanel.parentElement !== mount.surface) {
      mount.surface.appendChild(nativeThreadPanel);
    }
    return mount;
  }

  function syncNativeThreadPanelLayout() {
    if (
      !active
      || !nativeThreadPanel
      || nativeThreadPanel.hidden
      || !nativeThreadPanelBody
      || !nativeThreadPanelHost
      || !nativeThreadPanelHost.isConnected
    ) return;
    const surface = nativeThreadPanel.parentElement;
    if (!surface) return;
    const surfaceRect = surface.getBoundingClientRect();
    const bodyRect = nativeThreadPanelBody.getBoundingClientRect();
    if (bodyRect.width <= 0 || bodyRect.height <= 0) return;
    Object.assign(nativeThreadPanelHost.style, {
      position: "absolute",
      left: `${Math.max(0, bodyRect.left - surfaceRect.left)}px`,
      top: `${Math.max(0, bodyRect.top - surfaceRect.top)}px`,
      width: `${bodyRect.width}px`,
      height: `${bodyRect.height}px`,
      zIndex: "7",
      boxSizing: "border-box",
    });
    nativeThreadPanelHost.removeAttribute(HIDDEN_ATTRIBUTE);
    nativeThreadPanelHost.setAttribute(NATIVE_THREAD_HOST_ATTRIBUTE, "true");
  }

  function scheduleNativeThreadPanelAttach() {
    for (const delay of [80, 220, 500, 1_000]) {
      window.setTimeout(() => {
        if (nativeThreadPanelThreadId && nativeThreadPanelCanAttach) attachNativeThreadHost();
      }, delay);
    }
  }

  function attachNativeThreadHost() {
    if (!nativeThreadPanelCanAttach) return false;
    const mount = mountNativeThreadPanel();
    if (!mount || !nativeThreadPanel || nativeThreadPanel.hidden) return false;
    if (nativeThreadPanelHost !== mount.frameHost) {
      restoreNativeThreadHost();
      nativeThreadPanelHost = mount.frameHost;
      nativeThreadPanelHostStyle = nativeThreadPanelHost.getAttribute("style") || "";
      if (typeof ResizeObserver === "function") {
        nativeThreadPanelResizeObserver = new ResizeObserver(syncNativeThreadPanelLayout);
        nativeThreadPanelResizeObserver.observe(nativeThreadPanel);
      }
    }
    clearNativeThreadFallback();
    syncNativeThreadPanelLayout();
    return true;
  }

  function closeNativeThreadPanel({ remountTaskboard = true } = {}) {
    nativeThreadPanelThreadId = "";
    nativeThreadPanelWorkspacePath = "";
    nativeThreadPanelThreadTitle = "";
    nativeThreadPanelCanAttach = false;
    if (nativeThreadPanel) nativeThreadPanel.hidden = true;
    nativeThreadPanelBody?.replaceChildren();
    restoreNativeThreadHost();
    if (remountTaskboard && active) mountActivePage();
  }

  function updateNativeThreadPanelTitle(title, threadId) {
    if (!nativeThreadPanel) return;
    const titleNode = nativeThreadPanel.querySelector(".codex-taskboard-native-thread-title");
    const idNode = nativeThreadPanel.querySelector(".codex-taskboard-native-thread-id");
    if (titleNode) {
      titleNode.textContent = "继续对话";
      titleNode.title = title || normalizeThreadId(threadId) || "继续对话";
    }
    if (idNode) idNode.textContent = normalizeThreadId(threadId);
  }

  function toggleNativeThreadPanelSize() {
    nativeThreadPanelSize = nativeThreadPanelSize === "maximized" ? "compact" : "maximized";
    if (nativeThreadPanel) {
      nativeThreadPanel.dataset.size = nativeThreadPanelSize;
      const button = nativeThreadPanel.querySelector("[data-codex-taskboard-native-thread-size]");
      if (button) {
        setIconButton(button, nativeThreadPanelSize === "maximized" ? "chevronDown" : "expand");
        button.setAttribute("aria-label", nativeThreadPanelSize === "maximized" ? "缩小对话浮窗" : "放大对话浮窗");
      }
    }
    requestAnimationFrame(syncNativeThreadPanelLayout);
  }

  function createNativeThreadPanel() {
    const panel = document.createElement("section");
    panel.id = NATIVE_THREAD_PANEL_ID;
    panel.hidden = true;
    panel.dataset.size = nativeThreadPanelSize;
    panel.dataset.nativeHost = "true";
    panel.setAttribute(OWNED_ATTRIBUTE, "true");
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-label", "继续 Codex 对话");

    const header = document.createElement("div");
    header.className = "codex-taskboard-native-thread-header";

    const title = document.createElement("div");
    title.className = "codex-taskboard-native-thread-title";
    title.textContent = "继续对话";

    const id = document.createElement("span");
    id.className = "codex-taskboard-native-thread-id";

    const full = document.createElement("button");
    full.type = "button";
    setIconButton(full, "openExternal");
    full.title = "跳转到完整 Codex 对话";
    full.setAttribute("aria-label", "完整打开 Codex 对话");
    full.addEventListener("click", () => {
      const threadId = nativeThreadPanelThreadId;
      if (threadId) void openThread(threadId);
    });

    const size = document.createElement("button");
    size.type = "button";
    size.dataset.codexTaskboardNativeThreadSize = "true";
    setIconButton(size, nativeThreadPanelSize === "maximized" ? "chevronDown" : "expand");
    size.title = "放大或缩小浮窗";
    size.setAttribute("aria-label", nativeThreadPanelSize === "maximized" ? "缩小对话浮窗" : "放大对话浮窗");
    size.addEventListener("click", toggleNativeThreadPanelSize);

    const close = document.createElement("button");
    close.type = "button";
    setIconButton(close, "close");
    close.title = "关闭对话浮窗";
    close.setAttribute("aria-label", "关闭对话浮窗");
    close.addEventListener("click", () => closeNativeThreadPanel());

    header.append(title, id, full, size, close);

    const body = document.createElement("div");
    body.id = NATIVE_THREAD_PANEL_BODY_ID;
    panel.append(header, body);
    nativeThreadPanel = panel;
    nativeThreadPanelBody = body;
    return panel;
  }

  async function showNativeThreadPanel(payload) {
    const threadId = normalizeThreadId(payload?.threadId);
    if (!threadId) return;
    const title = typeof payload?.title === "string" ? payload.title.trim() : "";
    const workspacePath = normalizeWorkspacePath(payload?.workspacePath);
    const threadTitle = typeof payload?.threadTitle === "string" ? payload.threadTitle.trim() : "";
    const inferredThreadTitle = title.replace(/^[A-Z][A-Z0-9_-]*-\d+\s+/, "").trim();
    nativeThreadPanelThreadId = threadId;
    nativeThreadPanelWorkspacePath = workspacePath;
    nativeThreadPanelThreadTitle = threadTitle || inferredThreadTitle;
    nativeThreadPanelCanAttach = false;
    if (!active) openTaskboard();
    if (!nativeThreadPanel) createNativeThreadPanel();
    nativeThreadPanel.hidden = false;
    nativeThreadPanel.dataset.size = nativeThreadPanelSize;
    nativeThreadPanel.dataset.nativeHost = "true";
    updateNativeThreadPanelTitle(title, threadId);
    clearNativeThreadFallback();
    mountActivePage();
    mountNativeThreadPanel();
    const opened = await navigateNativeThread(threadId, {
      closeCurrentTaskboard: false,
      workspacePath,
      threadTitle: threadTitle || title,
    });
    if (!opened) {
      restoreNativeThreadHost();
      nativeThreadPanelCanAttach = false;
      showNativeThreadFallback(threadId);
      return;
    }
    nativeThreadPanelCanAttach = true;
    attachNativeThreadHost();
    scheduleNativeThreadPanelAttach();
  }

  function projectRowById(projectId) {
    if (typeof projectId !== "string" || !projectId.trim()) return null;
    return Array.from(document.querySelectorAll("[data-app-action-sidebar-project-row]"))
      .find((row) => row.getAttribute("data-app-action-sidebar-project-id") === projectId.trim()) || null;
  }

  function projectRowByLabel(label) {
    if (typeof label !== "string" || !label.trim()) return null;
    const expected = normalizedLabel(label);
    return Array.from(document.querySelectorAll("[data-app-action-sidebar-project-row]"))
      .find((row) => normalizedLabel(row.getAttribute("data-app-action-sidebar-project-label")) === expected) || null;
  }

  async function ensureProjectRows() {
    let section = findProjectsSection();
    const deadline = Date.now() + 1_200;
    while (!section && Date.now() < deadline) {
      await new Promise((resolve) => window.setTimeout(resolve, 40));
      section = findProjectsSection();
    }
    if (section?.getAttribute("data-app-action-sidebar-section-collapsed") === "true") {
      section.querySelector("[data-app-action-sidebar-section-toggle]")?.click();
    }
    while (readCodexProjects().length === 0 && Date.now() < deadline) {
      await new Promise((resolve) => window.setTimeout(resolve, 40));
    }
  }

  async function waitForPreparedComposer(identifier, skillPath) {
    const deadline = Date.now() + 8_000;
    while (Date.now() < deadline) {
      const editor = document.querySelector('[data-codex-composer="true"][contenteditable="true"]');
      if (editor && editor.getClientRects().length > 0) {
        const containsIdentifier = normalizedLabel(editor.textContent).includes(normalizedLabel(identifier));
        const skillMention = Array.from(editor.querySelectorAll("[skill-mention-name]"))
          .find((mention) => (
            mention.getAttribute("skill-mention-name") === "manage-taskboard"
            && mention.getAttribute("skill-mention-path") === skillPath
          ));
        if (containsIdentifier && skillMention) return editor;
      }
      await new Promise((resolve) => window.setTimeout(resolve, 80));
    }
    throw new Error("Codex 对话输入框没有生成 manage-taskboard Skill 引用");
  }

  async function createThreadForTask(payload) {
    const taskId = typeof payload?.taskId === "string" ? payload.taskId.trim() : "";
    const identifier = typeof payload?.identifier === "string" ? payload.identifier.trim() : "";
    const instruction = typeof payload?.instruction === "string" ? payload.instruction.trim() : "";
    const skillName = typeof payload?.skillName === "string" ? payload.skillName.trim() : "";
    const skillDisplayName = typeof payload?.skillDisplayName === "string"
      ? payload.skillDisplayName.trim()
      : "";
    const skillPath = typeof payload?.skillPath === "string" ? payload.skillPath.trim() : "";
    const workspacePath = typeof payload?.workspacePath === "string"
      ? payload.workspacePath.trim()
      : "";
    if (
      !taskId
      || !identifier
      || !instruction
      || !skillName
      || !skillDisplayName
      || !skillPath
      || pendingThreadCreation
    ) return;
    pendingThreadCreation = taskId;
    try {
      const bridge = window.electronBridge;
      if (!bridge || typeof bridge.sendMessageFromView !== "function") {
        throw new Error("当前 Codex 版本没有提供原生对话导航能力");
      }

      if (workspacePath) {
        await bridge.sendMessageFromView({
          type: "electron-set-active-workspace-root",
          root: workspacePath,
        });
      } else {
        await ensureProjectRows();
        const snapshotProjectId = hostContextSnapshot?.projectId || "";
        const requestedProjectId = typeof payload.codexProjectId === "string"
          ? payload.codexProjectId.trim()
          : "";
        const row = projectRowByLabel(payload.workspaceLabel)
          || projectRowById(requestedProjectId)
          || projectRowById(snapshotProjectId)
          || projectRowByLabel(payload.projectName);
        if (row?.getAttribute("data-app-action-sidebar-project-collapsed") === "true") {
          row.click?.();
          await new Promise((resolve) => window.setTimeout(resolve, 120));
        }
        const selectProject = row?.querySelector("[data-app-action-sidebar-select-project]");
        selectProject?.click?.();
        if (selectProject) await new Promise((resolve) => window.setTimeout(resolve, 120));
      }

      closeTaskboard(false);
      await dispatchHostMessage({
        type: "navigate-to-route",
        path: "/",
        state: {
          focusComposerNonce: Date.now(),
        },
      });
      await requestHostTaskComposerPrefill({
        instruction,
        skillDisplayName,
        skillName,
        skillPath,
      });
      await waitForPreparedComposer(identifier, skillPath);
      postToFrame({ type: "taskboard:thread-prepared", payload: { taskId } });
    } catch (error) {
      postToFrame({
        type: "taskboard:thread-create-error",
        payload: { taskId, error: error instanceof Error ? error.message : "无法创建 Codex 对话" },
      });
    } finally {
      pendingThreadCreation = null;
    }
  }

  function buildAutomationHostPayload(payload) {
    return {
      requestId: payload.requestId,
      operation: payload.operation,
      taskboardProjectId: payload.taskboardProjectId,
      codexProjectId: payload.codexProjectId,
      projectName: payload.projectName,
      workspacePath: payload.workspacePath,
      skillPath: payload.skillPath,
      ...(payload.automationId === undefined ? {} : { automationId: payload.automationId }),
      enabledByUser: payload.enabledByUser,
      quotaAware: payload.quotaAware,
      intervalMinutes: payload.intervalMinutes,
      model: payload.model,
      reasoningEffort: payload.reasoningEffort,
    };
  }

  async function handleAutomationRequest(payload) {
    const requestId = typeof payload?.requestId === "string" ? payload.requestId : "";
    if (!requestId) return;
    if (!isLocalTaskboardOrigin(frameOrigin)) {
      postToFrame({
        type: "taskboard:automation-response",
        payload: { requestId, ok: false, error: "仅本地任务面板可用" },
      });
      return;
    }
    try {
      const response = await requestHost(
        "automation",
        buildAutomationHostPayload(payload),
      );
      postToFrame({
        type: "taskboard:automation-response",
        payload: response.error
          ? { requestId, ok: false, error: response.error }
          : {
              requestId,
              ok: true,
              item: response.item,
              items: response.items,
              quota: response.quota,
              policy: response.policy,
            },
      });
    } catch (error) {
      postToFrame({
        type: "taskboard:automation-response",
        payload: {
          requestId,
          ok: false,
          error: error instanceof Error ? error.message : "Codex 自动任务操作失败",
        },
      });
    }
  }

  function onFrameMessage(event) {
    if (!frame || event.source !== frame.contentWindow || event.origin !== frameOrigin) return;
    const message = event.data;
    if (!message || typeof message !== "object") return;
    if (message.type === "taskboard:ready") {
      frameReady = true;
      frameReadyWaiters.forEach(({ resolve, timer }) => {
        window.clearTimeout(timer);
        resolve();
      });
      frameReadyWaiters.clear();
      if (active) showFrame();
      postHostContext();
      return;
    }
    if (message.type === "taskboard:drag-region") {
      updateDragRegion(message.payload);
      return;
    }
    if (message.type === "taskboard:open-thread") {
      void openThread(message.payload?.threadId, message.payload?.workspacePath, message.payload?.threadTitle);
      return;
    }
    if (message.type === "taskboard:show-thread-panel") {
      void showNativeThreadPanel(message.payload);
      return;
    }
    if (message.type === "taskboard:expand-sidebar") {
      expandNativeSidebar();
      return;
    }
    if (message.type === "taskboard:automation-request") {
      void handleAutomationRequest(message.payload);
      return;
    }
    if (message.type === "taskboard:create-thread") void createThreadForTask(message.payload);
  }

  function updateDragRegion(payload) {
    if (!dragRegion || !noDragLeft || !noDragRight) return;
    const [x, y, width, height] = [payload?.x, payload?.y, payload?.width, payload?.height];
    if (![x, y, width, height].every((value) => Number.isFinite(value)) || width <= 0 || height <= 0) {
      dragRegion.hidden = true;
      noDragLeft.hidden = true;
      noDragRight.hidden = true;
      return;
    }
    const left = Math.max(0, x);
    const right = left + width;
    dragRegion.style.left = `${left}px`;
    dragRegion.style.top = `${Math.max(0, y)}px`;
    dragRegion.style.width = `${width}px`;
    dragRegion.style.height = `${height}px`;
    noDragLeft.style.left = "0";
    noDragLeft.style.top = `${Math.max(0, y)}px`;
    noDragLeft.style.width = `${left}px`;
    noDragLeft.style.height = `${height}px`;
    noDragRight.style.left = `${right}px`;
    noDragRight.style.top = `${Math.max(0, y)}px`;
    noDragRight.style.right = "0";
    noDragRight.style.height = `${height}px`;
    dragRegion.hidden = false;
    noDragLeft.hidden = left <= 0;
    noDragRight.hidden = right >= page.clientWidth;
  }

  function createPage() {
    const section = document.createElement("section");
    section.id = PAGE_ID;
    section.hidden = true;
    section.setAttribute(OWNED_ATTRIBUTE, "true");
    section.setAttribute("role", "region");
    section.setAttribute("aria-label", "任务面板");

    status = document.createElement("div");
    status.id = STATUS_ID;
    status.setAttribute("role", "status");
    status.setAttribute("aria-live", "polite");
    section.appendChild(status);

    dragRegion = document.createElement("div");
    dragRegion.id = DRAG_REGION_ID;
    dragRegion.hidden = true;
    dragRegion.setAttribute(OWNED_ATTRIBUTE, "true");
    dragRegion.setAttribute("aria-hidden", "true");
    section.appendChild(dragRegion);

    noDragLeft = document.createElement("div");
    noDragLeft.id = NO_DRAG_LEFT_ID;
    noDragLeft.hidden = true;
    noDragLeft.setAttribute(OWNED_ATTRIBUTE, "true");
    noDragLeft.setAttribute("aria-hidden", "true");
    section.appendChild(noDragLeft);

    noDragRight = document.createElement("div");
    noDragRight.id = NO_DRAG_RIGHT_ID;
    noDragRight.hidden = true;
    noDragRight.setAttribute(OWNED_ATTRIBUTE, "true");
    noDragRight.setAttribute("aria-hidden", "true");
    section.appendChild(noDragRight);
    return section;
  }

  function showLoading() {
    if (!status) return;
    status.replaceChildren(document.createTextNode("正在启动任务面板…"));
    status.hidden = false;
    if (frame) frame.hidden = true;
  }

  function showFrame() {
    if (status) status.hidden = true;
    if (frame) {
      frame.hidden = false;
      frame.focus?.();
    }
  }

  function showLoadError(message) {
    if (!status) return;
    const content = document.createElement("div");
    const text = document.createElement("div");
    text.textContent = message;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "重新启动";
    retry.addEventListener("click", openTaskboard, { once: true });
    content.append(text, retry);
    status.replaceChildren(content);
    status.hidden = false;
    if (frame) frame.hidden = true;
  }

  function cancelFrameReadyWaiters(error) {
    frameReadyWaiters.forEach(({ reject, timer }) => {
      window.clearTimeout(timer);
      reject(error);
    });
    frameReadyWaiters.clear();
  }

  function waitForFrameReady() {
    if (frameReady) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const waiter = {
        resolve,
        reject,
        timer: window.setTimeout(() => {
          frameReadyWaiters.delete(waiter);
          reject(new Error("任务面板页面加载超时"));
        }, FRAME_READY_TIMEOUT_MS),
      };
      frameReadyWaiters.add(waiter);
    });
  }

  function loadTaskboardFrame(cacheBust = false) {
    cancelFrameReadyWaiters(new Error("任务面板正在重新加载"));
    frame?.remove();
    frame = null;
    frameReady = false;
    if (dragRegion) dragRegion.hidden = true;
    if (noDragLeft) noDragLeft.hidden = true;
    if (noDragRight) noDragRight.hidden = true;

    const taskboardUrl = resolveTaskboardUrl();
    if (cacheBust) {
      taskboardUrl.searchParams.set(FRAME_REFRESH_PARAM, Date.now().toString(36));
    }
    frameOrigin = taskboardUrl.origin;
    const nextFrame = document.createElement("iframe");
    nextFrame.id = FRAME_ID;
    nextFrame.hidden = true;
    nextFrame.src = taskboardUrl.href;
    nextFrame.title = "任务面板";
    nextFrame.referrerPolicy = "no-referrer";
    nextFrame.setAttribute("allow", "clipboard-read; clipboard-write");
    nextFrame.addEventListener("load", postHostContext);
    frame = nextFrame;
    page.appendChild(nextFrame);
  }

  function reloadFrame() {
    if (!frame) return false;
    const generation = ++openGeneration;
    if (active) showLoading();
    loadTaskboardFrame(true);
    if (active) {
      void waitForFrameReady()
        .then(() => {
          if (!active || generation !== openGeneration) return;
          showFrame();
          postHostContext();
        })
        .catch((error) => {
          if (!active || generation !== openGeneration) return;
          showLoadError(error.message);
        });
    }
    return true;
  }

  function managedTaskboardOrigin() {
    const configured = typeof window.__CODEX_TASKBOARD_MANAGED_ORIGIN__ === "string"
      ? window.__CODEX_TASKBOARD_MANAGED_ORIGIN__.trim()
      : "";
    try {
      return new URL(configured || DEFAULT_TASKBOARD_URL).origin;
    } catch (_) {
      return new URL(DEFAULT_TASKBOARD_URL).origin;
    }
  }

  function hasLiveHostBinding() {
    const heartbeat = Number(window[HOST_HEARTBEAT_NAME]);
    return typeof window[HOST_BINDING_NAME] === "function"
      && Number.isFinite(heartbeat)
      && Date.now() - heartbeat <= HOST_HEARTBEAT_MAX_AGE_MS;
  }

  function requestHost(action, payload = {}) {
    const binding = window[HOST_BINDING_NAME];
    if (!hasLiveHostBinding()) {
      return Promise.reject(new Error("Taskboard 启动器未运行，无法操作 Codex 对话输入框"));
    }

    const id = `${Date.now().toString(36)}-${(++hostRequestSequence).toString(36)}`;
    return new Promise((resolve, reject) => {
      const timeout = window.setTimeout(() => {
        hostRequests.delete(id);
        reject(new Error("任务面板启动器没有响应"));
      }, HOST_REQUEST_TIMEOUT_MS);
      hostRequests.set(id, { resolve, reject, timeout });
      try {
        binding(JSON.stringify({ ...payload, id, action }));
      } catch (error) {
        window.clearTimeout(timeout);
        hostRequests.delete(id);
        reject(error);
      }
    });
  }

  function requestHostEnsure(taskboardUrl) {
    if (taskboardUrl.origin !== managedTaskboardOrigin() || !hasLiveHostBinding()) {
      return Promise.resolve({ managed: false, restarted: false });
    }
    return requestHost("ensure");
  }

  function requestHostTaskComposerPrefill({
    instruction,
    skillDisplayName,
    skillName,
    skillPath,
  }) {
    return requestHost("prefill-task-composer", {
      instruction,
      skillDisplayName,
      skillName,
      skillPath,
    });
  }

  function frameMatchesTaskboardUrl(taskboardUrl) {
    if (!frame) return false;
    try {
      const loadedUrl = new URL(frame.getAttribute("src") || frame.src);
      loadedUrl.searchParams.delete(FRAME_REFRESH_PARAM);
      const expectedUrl = new URL(taskboardUrl.href);
      expectedUrl.searchParams.delete(FRAME_REFRESH_PARAM);
      return loadedUrl.href === expectedUrl.href;
    } catch (_) {
      return false;
    }
  }

  function onHostResponse(response) {
    if (!response || typeof response !== "object" || typeof response.id !== "string") return;
    const pending = hostRequests.get(response.id);
    if (!pending) return;
    window.clearTimeout(pending.timeout);
    hostRequests.delete(response.id);
    if (response.ok) pending.resolve(response);
    else pending.reject(new Error(response.error || "任务面板服务启动失败"));
  }

  async function prepareTaskboard(generation) {
    const taskboardUrl = resolveTaskboardUrl();
    const canReuseFrame = Boolean(
      frameReady
      && frame?.isConnected
      && frameMatchesTaskboardUrl(taskboardUrl),
    );
    if (canReuseFrame) showFrame();
    else showLoading();

    try {
      const [result, context] = await Promise.all([
        requestHostEnsure(taskboardUrl),
        captureHostContext(),
      ]);
      if (!active || generation !== openGeneration) return;
      hostContextSnapshot = context;
      if (!frameReady || result.restarted || !frameMatchesTaskboardUrl(taskboardUrl)) {
        showLoading();
        loadTaskboardFrame();
        await waitForFrameReady();
      }
      if (!active || generation !== openGeneration) return;
      showFrame();
      postHostContext();
    } catch (error) {
      if (!active || generation !== openGeneration) return;
      const bindingAvailable = hasLiveHostBinding();
      showLoadError(bindingAvailable
        ? error.message
        : "任务面板服务未就绪。请保持 Taskboard 启动器运行后重试。");
    }
  }

  function restoreNativeContent() {
    document.querySelectorAll(`[${HIDDEN_ATTRIBUTE}="true"]`)
      .forEach((node) => node.removeAttribute(HIDDEN_ATTRIBUTE));
    document.querySelectorAll(`[${HOST_ATTRIBUTE}="true"]`)
      .forEach((node) => node.removeAttribute(HOST_ATTRIBUTE));
  }

  function mountActivePage() {
    if (!active) return;
    if (!page) page = createPage();
    const mount = findPageMount();
    if (!mount) return;
    const { surface } = mount;

    if (page.parentElement !== surface) {
      restoreNativeContent();
      surface.appendChild(page);
    }
    surface.setAttribute(HOST_ATTRIBUTE, "true");
    Array.from(surface.children).forEach((child) => {
      if (child === nativeThreadPanelHost) {
        child.removeAttribute(HIDDEN_ATTRIBUTE);
        child.setAttribute(NATIVE_THREAD_HOST_ATTRIBUTE, "true");
        return;
      }
      if (child !== page && child.getAttribute(OWNED_ATTRIBUTE) !== "true") {
        child.setAttribute(HIDDEN_ATTRIBUTE, "true");
      }
    });
    hideNativeHeader();
    muteNativeSelection();
    page.hidden = false;
    syncNativeThreadPanelLayout();
    document.documentElement.setAttribute("data-codex-taskboard-open", "true");
  }

  function closeTaskboard(restoreFocus = true) {
    if (!active && page?.hidden !== false) return;
    openGeneration += 1;
    active = false;
    closeNativeThreadPanel({ remountTaskboard: false });
    if (page) page.hidden = true;
    restoreNativeContent();
    restoreNativeSelection();
    document.documentElement.removeAttribute("data-codex-taskboard-open");
    syncEntryState();
    if (restoreFocus) lastFocusedElement?.focus?.();
    lastFocusedElement = null;
    hostContextSnapshot = null;
  }

  function openTaskboard() {
    if (destroyed) return;
    if (!active) {
      lastFocusedElement = document.activeElement;
      hostContextSnapshot = null;
    }
    const generation = ++openGeneration;
    active = true;
    ensureEntry();
    mountActivePage();
    syncEntryState();
    void prepareTaskboard(generation);
  }

  function isNativePageNavigation(target) {
    const clickable = target?.closest?.("button,a,[role='button'],[data-app-action-sidebar-thread-id]");
    if (!clickable || clickable === entry || clickable.closest(`#${ENTRY_ID}`)) return false;
    if (!clickable.closest("aside nav[role='navigation']")) return false;
    if (clickable.hasAttribute("data-app-action-sidebar-section-toggle")) return false;
    if (buttonMatches(clickable, NATIVE_PAGE_LABELS)) return true;
    return Boolean(clickable.closest(
      "[data-app-action-sidebar-thread-id],"
      + "[data-app-action-sidebar-project-row],"
      + "[data-app-action-sidebar-project-id]",
    ));
  }

  function onDocumentClick(event) {
    const threadRow = event.target?.closest?.("[data-app-action-sidebar-thread-id]");
    const clickedThreadId = normalizeThreadId(threadRow?.getAttribute?.("data-app-action-sidebar-thread-id"));
    if (clickedThreadId) lastNativeThreadId = clickedThreadId;
    if (clickedThreadId && clickedThreadId === normalizeThreadId(nativeThreadPanelThreadId)) return;
    if (Date.now() < suppressNativeCloseUntil) return;
    if (!active || !isNativePageNavigation(event.target)) return;
    closeTaskboard(false);
  }

  function scheduleRefresh() {
    if (destroyed || reattachTimer !== null) return;
    reattachTimer = window.setTimeout(() => {
      reattachTimer = null;
      ensureEntry();
      mountActivePage();
      postHostContext();
    }, REATTACH_DELAY_MS);
  }

  function refresh() {
    ensureEntry();
    mountActivePage();
    postHostContext();
  }

  function mount() {
    document.removeEventListener("DOMContentLoaded", mount);
    if (destroyed || observer || !document.documentElement) return;
    ensureEntry();
    observer = new MutationObserver(scheduleRefresh);
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: [
        "class",
        "data-theme",
        "data-color-theme",
        "data-app-action-sidebar-project-label",
        "data-app-action-sidebar-thread-active",
        "aria-label",
        "aria-current",
      ],
    });
  }

  function destroy() {
    if (destroyed) return;
    destroyed = true;
    if (reattachTimer !== null) window.clearTimeout(reattachTimer);
    reattachTimer = null;
    observer?.disconnect();
    observer = null;
    cancelFrameReadyWaiters(new Error("任务面板已关闭"));
    hostRequests.forEach(({ reject, timeout }) => {
      window.clearTimeout(timeout);
      reject(new Error("任务面板已关闭"));
    });
    hostRequests.clear();
    pendingThreadCreation = null;
    document.removeEventListener("DOMContentLoaded", mount);
    document.removeEventListener("click", onDocumentClick, true);
    window.removeEventListener("message", onFrameMessage);
    window.removeEventListener("popstate", onNativeRouteChange);
    window.removeEventListener("hashchange", onNativeRouteChange);
    window.removeEventListener("resize", scheduleRefresh);
    closeTaskboard(false);
    document.querySelectorAll(`[${OWNED_ATTRIBUTE}="true"]`).forEach((node) => node.remove());
    entry = null;
    page = null;
    frame = null;
    dragRegion = null;
    noDragLeft = null;
    noDragRight = null;
    status = null;
    frameOrigin = "";
    if (window[SENTINEL_KEY] === api) delete window[SENTINEL_KEY];
  }

  function onNativeRouteChange() {
    if (nativeThreadPanelThreadId) {
      suppressNativeCloseUntil = Date.now() + 1_000;
      if (nativeThreadPanelCanAttach) scheduleNativeThreadPanelAttach();
      return;
    }
    if (active) closeTaskboard(false);
  }

  const api = {
    version: VERSION,
    sourceHash: SOURCE_HASH,
    refresh,
    reloadFrame,
    open: openTaskboard,
    close: closeTaskboard,
    destroy,
    hostResponse: onHostResponse,
  };
  window[SENTINEL_KEY] = api;

  window.addEventListener("message", onFrameMessage);
  window.addEventListener("popstate", onNativeRouteChange);
  window.addEventListener("hashchange", onNativeRouteChange);
  window.addEventListener("resize", scheduleRefresh);
  document.addEventListener("click", onDocumentClick, true);
  if (document.documentElement) mount();
  else document.addEventListener("DOMContentLoaded", mount, { once: true });
})();
