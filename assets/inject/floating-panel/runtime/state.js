/*
 * Floating-panel runtime state.
 *
 * Rust composes this fragment into the single floating-panel IIFE before the
 * context, feature, core, outline, and public-entry fragments.
 */

  "use strict";

  /*
   * Stepwise is a self-contained runtime injected into ChatGPT's renderer.
   * It owns the floating shell, Stepwise suggestions, and Answer Outline;
   * the Manager only supplies settings and the page bridge supplies requests.
   *
   * The important invariants are:
   * - only one live instance, root, style element, and observer may exist;
   * - Stepwise and Outline can be enabled independently;
   * - passive page scrolling never changes the pinned answer context;
   * - asynchronous results must match the answer, request, feature epoch,
   *   and runtime generation that created them;
   * - every view or shell transition must settle, cancel, or time out cleanly.
   */

  // Runtime identity, DOM markers, storage keys, and stable UI dimensions.
  const API_KEY = "__codexStepwisePanel";
  const STYLE_ID = "codex-stepwise-panel-style";
  const CLEAR_FILTER_ID = "codex-stepwise-clear-distortion";
  const LIQUID_FILTER_ID = "codex-stepwise-liquid-distortion";
  const CRYSTAL_FILTER_ID = "codex-stepwise-crystal-distortion";
  const ROOT_ATTR = "data-codex-stepwise-root";
  const PAYLOAD_ATTR = "data-codex-stepwise-payload";
  const MARK_ATTR = "data-codex-stepwise-outline-id";
  const HIGHLIGHT_CLASS = "codex-stepwise-outline-target-flash";
  const SCRIPT_VERSION = "2.0.7";
  const PAGE_BRIDGE = "__codexSessionDeleteBridge";
  const CONVERSATION_TURN_SELECTOR = "div.contents[data-content-search-turn-key]";
  const POPOVER_ID = "codex-stepwise-popover";
  const POSITION_KEY = "codex-stepwise-float-position-v2";
  const WIDTH_KEY = "codex-stepwise-panel-width-v1";
  const HEIGHT_KEY = "codex-stepwise-panel-height-v1";
  const FONT_KEY = "codex-stepwise-font-v1";
  const FONT_OFFSET_KEY = "codex-stepwise-font-offset-v1";
  const LEGACY_MATERIAL_KEY = "codex-stepwise-material-v1";
  const PREVIOUS_MATERIAL_KEY = "codex-stepwise-material-v2";
  const MATERIAL_KEY = "codex-stepwise-material-v3";
  const MATERIAL_ORIGIN_KEY = "codex-stepwise-material-v3-origin";
  const MATERIAL_MIGRATION_KEY = "codex-stepwise-material-v3-migrated";
  const LABEL_ONLY_KEY = "codex-stepwise-label-only-v1";
  const PROMPT_CLICK_MODE_KEY = "codex-stepwise-prompt-click-mode-v1";
  const VIEW_ORDER_KEY = "codex-stepwise-view-order-v1";
  const PROMPT_CLICK_MODES = ["direct", "hybrid", "fill"];
  const DEFAULT_PROMPT_CLICK_MODE = "hybrid";
  const GENERATION_MODES = ["auto", "manual"];
  const MATERIAL_MODES = ["frosted", "clear", "liquid", "crystal", "matte"];
  const DEFAULT_MATERIAL = "frosted";
  const LEGACY_MATERIAL_MODES = Object.freeze({
    glass: "frosted",
    liquid: "clear",
    liquid2: "liquid",
    solid: "matte",
    opaque: "matte",
  });
  const LEGACY_OUTLINE_FONT_KEY = "codex-answer-outline-font";
  const LEGACY_OUTLINE_FONT_OFFSET_KEY = "codex-answer-outline-font-offset";
  const DIAGNOSTICS_KEY = "codex-stepwise-diagnostics-v1";
  const SCAN_DELAY_MS = 220;
  const STREAM_IDLE_MS = 1300;
  const NEW_ANSWER_EXPRESSION_MS = 700;
  const BRIDGE_TIMEOUT_MS = 26000;
  const SETTINGS_SYNC_INTERVAL_MS = 2000;
  const FLASH_MS = 1200;
  const COMPLETION_BEAM_MS = 1600;
  const MIN_OUTLINE_TEXT_LEN = 280;
  const MIN_OUTLINE_ITEMS = 2;
  const MAX_OUTLINE_ITEMS = 24;
  const MAX_OUTLINE_TITLE_LEN = 56;
  const MIN_OUTLINE_TITLE_LEN = 2;
  const OUTLINE_TARGET_TOP_OFFSET = 28;
  const OUTLINE_INDENT_STEP = 12;
  const OUTLINE_SCROLL_SETTLE_MS = 720;
  const OUTLINE_SCROLL_RECHECK_MS = 140;
  const OUTLINE_SEMANTIC_HEADING_SELECTOR = "h1,h2,h3,h4,h5,h6,[role='heading']";
  const OUTLINE_PSEUDO_HEADING_SELECTOR = "p,div,li,strong,b";
  const OUTLINE_TABLE_SELECTOR = [
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "[role='table']",
    "[role='row']",
    "[role='cell']",
    "[role='columnheader']",
    "[role='rowheader']",
  ].join(",");
  const OUTLINE_PSEUDO_MIN_SCORE = 24;
  const CHIP_WIDTH = 84;
  const CHIP_HEIGHT = 46;
  const CHIP_RADIUS = 23;
  const PANEL_WIDTH = 404;
  const PANEL_HEIGHT = 420;
  const SETTINGS_PANEL_HEIGHT = 376;
  const PANEL_MIN_WIDTH = 300;
  const PANEL_MAX_WIDTH = 640;
  const PANEL_MIN_HEIGHT = 340;
  const PANEL_MAX_HEIGHT = 720;
  const PANEL_RADIUS = 25;
  const PANEL_SAFE_MARGIN = 12;
  const RIGHT_EDGE_SNAP_DISTANCE = 36;
  const DEFAULT_FONT = 13;
  const MIN_FONT = 10;
  const MAX_FONT = 24;
  const HOST_FONT_SIZE_FALLBACK = 15;
  const HOST_FONT_SIZE_MIN = 12;
  const HOST_FONT_SIZE_MAX = 22;
  const ITEM_FONT_RATIO = 13 / 15;
  const CHROME_FONT_RATIO = 12 / 15;
  const ICON_FONT_RATIO = 16 / 15;
  const HOST_FONT_FAMILY_FALLBACK = '-apple-system, "system-ui", "Segoe UI", sans-serif';
  const MIN_MORPH_MS = 840;
  const MAX_MORPH_MS = 1450;
  const MIN_PHASE_MS = 420;
  const MIN_REVERSE_MS = 120;
  const MORPH_FALLBACK_BUFFER_MS = 180;
  const HORIZONTAL_PHASE = 0.5;
  const MORPH_EDGE_SPEED = 0.18;
  const UNFOLD_SAMPLES = 28;
  const VIEW_SLIDE_MS = 240;
  const VIEW_SLIDE_DISTANCE = 12;
  const VIEW_INDICATOR_MS = 220;
  const DEFAULT_VIEW_ORDER = ["next", "outline"];
  const EYE_MAX_X = 4;
  const EYE_MAX_Y = 3;
  const CURIOUS_EYE_MAX_X = 3;
  const CURIOUS_EYE_MAX_Y = 2.5;
  const MAX_TEXT_LENGTH = 12000;
  const DEFAULT_STEPWISE_ITEMS = 4;
  const MAX_STEPWISE_ITEMS = 6;
  const MAX_PROMPT_SUMMARY_LENGTH = 72;
  const MAX_DIAGNOSTICS = 80;
  const EDITABLE_SUBMIT_DELAY_MS = 120;
  const PROMPT_PREVIEW_SWITCH_MS = 320;
  const PROMPT_CLICK_DELAY_MS = 230;
  const SUBMIT_RETRY_DELAY_MS = 50;
  const SUBMIT_RETRY_LIMIT = 80;
  const FRIENDLY_BRIDGE_ERRORS = [
    {
      pattern: /回答生成中/i,
      title: "回答尚未完成，完成后再试",
      message: "",
    },
    {
      pattern: /未找到可用于生成的回答/i,
      title: "回答尚未完成，完成后再试",
      message: "",
    },
    {
      pattern: /\b429\b|too many pending|rate[_ -]?limit/i,
      title: "请求较多，稍后再试",
      message: "",
    },
    {
      pattern: /timeout|timed out|超时/i,
      title: "响应较慢，稍后再试",
      message: "",
    },
    {
      pattern: /\b401\b|\b403\b|unauthori[sz]ed|forbidden|api.?key|鉴权|认证/i,
      title: "连接异常，检查模型与配置",
      message: "",
    },
    {
      pattern: /econnrefused|failed to fetch|network|connection|连接失败|无法连接/i,
      title: "暂时无法连接，检查服务后重试",
      message: "",
    },
    {
      pattern: /\b5\d{2}\b|upstream/i,
      title: "服务暂时不可用，稍后重试",
      message: "",
    },
  ];
  const INSTANCE_ID = `${SCRIPT_VERSION}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  let codexAppActionsPromise = null;
  let settingsPromise = null;
  let startupPromise = null;
  let settingsRequestId = 0;
  let settingsSyncEpoch = 0;
  let pendingSettingsPatch = {};

  // Re-injection replaces stale instances instead of layering another UI on top.
  const previous = window[API_KEY];
  const previousRuntimeHealthy = previous?.state?.runtimeActive === true
    && previous?.state?.settingsLoaded === true
    && document.readyState !== "loading"
    && previous?.state?.root?.isConnected === true
    && previous?.state?.popover?.isConnected === true
    && Boolean(previous?.state?.observer)
    && document.querySelectorAll?.(`[${ROOT_ATTR}="true"]`).length === 1
    && document.querySelectorAll?.(`#${STYLE_ID}`).length === 1
    && document.getElementById(STYLE_ID)?.dataset.codexStepwiseStyleVersion === SCRIPT_VERSION;
  if (previous?.version === SCRIPT_VERSION
    && previous?.state?.destroyed !== true
    && previousRuntimeHealthy) {
    previous.syncSettings?.();
    previous.start?.();
    return;
  }
  if (previous && typeof previous.destroy === "function") previous.destroy();
  document.querySelectorAll?.(`[${ROOT_ATTR}="true"]`).forEach((node) => node.remove());
  document.getElementById(STYLE_ID)?.remove();

  const storage = {
    get(key) {
      try {
        return localStorage.getItem(key);
      } catch {
        return null;
      }
    },
    set(key, value) {
      try {
        localStorage.setItem(key, value);
      } catch {}
    },
    remove(key) {
      try {
        localStorage.removeItem(key);
      } catch {}
    },
  };

  function normalizePromptClickMode(value) {
    return PROMPT_CLICK_MODES.includes(value) ? value : DEFAULT_PROMPT_CLICK_MODE;
  }

  function readPromptClickMode() {
    const stored = storage.get(PROMPT_CLICK_MODE_KEY);
    if (PROMPT_CLICK_MODES.includes(stored)) return stored;
    storage.set(PROMPT_CLICK_MODE_KEY, DEFAULT_PROMPT_CLICK_MODE);
    return DEFAULT_PROMPT_CLICK_MODE;
  }

  function normalizeViewOrder(value) {
    const source = Array.isArray(value) ? value : [];
    const result = source.filter((tab, index) =>
      DEFAULT_VIEW_ORDER.includes(tab) && source.indexOf(tab) === index,
    );
    DEFAULT_VIEW_ORDER.forEach((tab) => {
      if (!result.includes(tab)) result.push(tab);
    });
    return result;
  }

  function readViewOrder() {
    try {
      return normalizeViewOrder(JSON.parse(storage.get(VIEW_ORDER_KEY) || "null"));
    } catch {
      return DEFAULT_VIEW_ORDER.slice();
    }
  }

  // All mutable runtime state lives here so cleanup can invalidate one generation.
  const state = {
    observer: null,
    themeObserver: null,
    typographyObserver: null,
    promptPreviewTimer: 0,
    promptClickTimer: 0,
    promptPreviewIndex: 0,
    timer: 0,
    expressionTimer: 0,
    keepAliveTimer: 0,
    flashTimer: 0,
    completionBeamTimer: 0,
    snapTimer: 0,
    materialAnimTimer: 0,
    viewAnimation: null,
    viewIndicatorFrame: 0,
    viewReorderCleanup: null,
    suppressViewTabClickUntil: 0,
    viewTransitioning: false,
    pendingTab: "",
    pendingRender: false,
    root: null,
    fab: null,
    popover: null,
    glass: null,
    rim: null,
    completionBeam: null,
    clearFilter: null,
    clearDisplacement: null,
    clearDistortion: null,
    liquidFilter: null,
    crystalFilter: null,
    displacementTexture: null,
    panel: null,
    contentFadeCleanup: null,
    open: false,
    morphAnimation: null,
    rimMorphAnimation: null,
    displacementMorphAnimation: null,
    panelMorphAnimation: null,
    fabMorphAnimation: null,
    morphTransition: null,
    morphGeneration: 0,
    layout: null,
    focusAfterMorph: "",
    activeTab: "next",
    returnTab: "next",
    viewOrder: readViewOrder(),
    position: null,
    width: readPanelWidth(),
    height: readPanelHeight(),
    hostTypography: fallbackHostTypography(),
    fontOffset: readFontOffset(),
    material: readMaterial(),
    labelOnly: storage.get(LABEL_ONLY_KEY) === "true",
    promptClickMode: readPromptClickMode(),
    drag: null,
    dragCleanup: null,
    resizeDrag: null,
    resizeCleanup: null,
    suppressFabClick: false,
    suppressHeadFaceClick: false,
    eyePointer: null,
    eyeRaf: 0,
    eyeCleanup: null,
    sourceCueAngle: null,
    sourceCueAnimation: 0,
    lastAssistantHash: "",
    lastAssistantAt: 0,
    currentHash: "",
    scanStatus: "idle",
    scanBusy: false,
    lastScanStatus: "",
    bridgeCache: new Map(),
    bridgeActiveKey: "",
    bridgePendingHash: "",
    bridgePendingRequestId: 0,
    bridgePendingMode: "auto",
    bridgeRequestSequence: 0,
    bridgeStatus: "idle",
    bridgeError: "",
    prompts: [],
    promptContext: null,
    outlineItems: [],
    outlineStatus: "idle",
    outlineError: "",
    outlineFingerprint: "",
    outlineSourceHash: "",
    outlineRefreshPromise: null,
    outlineMessage: null,
    outlineScrollCleanup: null,
    settings: null,
    settingsLoaded: false,
    settingsFingerprint: "",
    settingsSyncTimer: 0,
    settingsStatus: "",
    surpriseUntil: 0,
    fabExpression: "idle",
    theme: "dark",
    themeMode: "auto",
    pinnedThreadRoot: null,
    pinnedThreadAt: 0,
    pinnedPaneKey: "",
    pinnedSessionId: "",
    latestTurnAnchor: null,
    threadActivity: new WeakMap(),
    nodeKeySeq: 0,
    nodeKeys: new WeakMap(),
    activeContext: {
      paneRoot: null,
      paneKey: "",
      sessionId: "",
      assistantMessageId: "",
      generation: 0,
    },
    focusHandler: null,
    pointerHandler: null,
    selectionHandler: null,
    scrollHandler: null,
    keyHandler: null,
    scans: 0,
    runtimeGeneration: 0,
    runtimeActive: false,
    stepwiseEpoch: 0,
    outlineEpoch: 0,
    domReadyHandler: null,
    destroyed: false,
    diagnostics: readDiagnostics(),
  };

  // Runtime gates and feature epochs make stale callbacks harmless after re-injection or disablement.
  function isCurrentInstance() {
    return !state.destroyed && window[API_KEY]?.instanceId === INSTANCE_ID;
  }

  function isCurrentRuntime(generation = state.runtimeGeneration) {
    return isCurrentInstance()
      && state.runtimeActive
      && generation === state.runtimeGeneration;
  }

  function stepwiseEnabled(settings = state.settings) {
    return settings?.enabled === true;
  }

  function normalizeGenerationMode(value) {
    return value === "manual" ? "manual" : "auto";
  }

  function stepwiseGenerationMode(settings = state.settings) {
    return normalizeGenerationMode(settings?.generationMode);
  }

  function outlineEnabled(settings = state.settings) {
    return settings?.answerOutlineEnabled === true;
  }

  function runtimeEnabled(settings = state.settings) {
    return stepwiseEnabled(settings) || outlineEnabled(settings);
  }

  function enabledViewOrder() {
    return state.viewOrder.filter((tab) => tab === "next" ? stepwiseEnabled() : outlineEnabled());
  }

  function viewNavigationOrder() {
    return [...enabledViewOrder(), "settings"];
  }

  function persistViewOrder(order) {
    state.viewOrder = normalizeViewOrder(order);
    storage.set(VIEW_ORDER_KEY, JSON.stringify(state.viewOrder));
  }

  function configuredMaxPromptItems(settings = state.settings) {
    const value = Number(settings?.maxItems);
    if (!Number.isFinite(value)) return DEFAULT_STEPWISE_ITEMS;
    return clamp(Math.floor(value), 1, MAX_STEPWISE_ITEMS);
  }

  function normalizeActiveTab(tab = state.activeTab) {
    if (tab === "settings") return "settings";
    if (tab === "next" && stepwiseEnabled()) return "next";
    if (tab === "outline" && outlineEnabled()) return "outline";
    if (stepwiseEnabled()) return "next";
    if (outlineEnabled()) return "outline";
    return "next";
  }

  function resetStepwiseFeature() {
    state.stepwiseEpoch += 1;
    clearPromptInteractionTimers();
    state.promptPreviewIndex = 0;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = stepwiseGenerationMode();
    state.bridgeStatus = "idle";
    state.bridgeError = "";
    state.bridgeCache.clear();
    state.prompts = [];
    state.promptContext = null;
    state.currentHash = "";
    clearStepwisePayloadMarks();
  }

  function invalidateStepwiseRequest(status = stepwiseGenerationMode() === "manual" ? "manual-ready" : "idle") {
    state.stepwiseEpoch += 1;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = stepwiseGenerationMode();
    state.bridgeStatus = status;
    state.bridgeError = "";
  }

  function resetOutlineFeature() {
    state.outlineEpoch += 1;
    state.outlineScrollCleanup?.();
    state.outlineScrollCleanup = null;
    outlineClearMarks();
    state.outlineItems = [];
    state.outlineRefreshPromise = null;
    state.outlineMessage = null;
    state.outlineSourceHash = "";
    state.outlineFingerprint = "";
    state.outlineStatus = "idle";
    state.outlineError = "";
  }

  function applyRuntimeSettings(nextSettings) {
    const hadStepwise = stepwiseEnabled();
    const hadOutline = outlineEnabled();
    const previousGenerationMode = stepwiseGenerationMode();
    state.settings = nextSettings;
    state.settingsFingerprint = settingsFingerprint(nextSettings);
    if (hadStepwise && !stepwiseEnabled()) resetStepwiseFeature();
    if (hadOutline && !outlineEnabled()) resetOutlineFeature();
    if (hadStepwise && stepwiseEnabled() && previousGenerationMode !== stepwiseGenerationMode()) {
      invalidateStepwiseRequest();
      state.prompts = [];
      state.promptContext = null;
      state.promptPreviewIndex = 0;
      state.currentHash = "";
    }
    state.activeTab = normalizeActiveTab();
    return state.settings;
  }

  function settingsFingerprint(settings) {
    if (!settings || typeof settings !== "object") return "";
    return JSON.stringify(
      Object.keys(settings)
        .sort()
        .map((key) => [key, settings[key]]),
    );
  }

  // Shared text and numeric helpers keep DOM extraction and persisted values bounded.
  function normalizeText(value) {
    return String(value || "")
      .replace(/\u00a0/g, " ")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n{3,}/g, "\n\n")
      .replace(/[ \t]{2,}/g, " ")
      .trim();
  }

  function shortText(value, limit = MAX_TEXT_LENGTH) {
    const text = normalizeText(value);
    return text.length > limit ? text.slice(text.length - limit) : text;
  }

  function hashText(value) {
    const text = shortText(value, 4000);
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
