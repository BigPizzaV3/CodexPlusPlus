(() => {
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
  const SCRIPT_VERSION = "2.0.0";
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
  const VIEW_SLIDE_MS = 180;
  const VIEW_SLIDE_DISTANCE = 12;
  const VIEW_INDICATOR_MS = 150;
  const VIEW_ORDER = ["next", "outline", "settings"];
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
    && document.querySelectorAll?.(`#${STYLE_ID}`).length === 1;
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

  function roundPixel(value) {
    return Math.round(Number(value) * 100) / 100;
  }

  function clampPanelWidth(value) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return PANEL_WIDTH;
    return Math.round(clamp(parsed, PANEL_MIN_WIDTH, PANEL_MAX_WIDTH));
  }

  function readPanelWidth() {
    const raw = storage.get(WIDTH_KEY);
    return raw == null || raw === "" ? PANEL_WIDTH : clampPanelWidth(raw);
  }

  function panelHeightCap() {
    const viewportCap = Math.max(
      PANEL_MIN_HEIGHT,
      Math.floor((window.innerHeight || PANEL_MAX_HEIGHT) - PANEL_SAFE_MARGIN * 2)
    );
    return Math.min(PANEL_MAX_HEIGHT, viewportCap);
  }

  function clampPanelHeight(value) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return Math.min(PANEL_HEIGHT, panelHeightCap());
    return Math.round(clamp(parsed, PANEL_MIN_HEIGHT, panelHeightCap()));
  }

  function readPanelHeight() {
    const raw = storage.get(HEIGHT_KEY);
    return raw == null || raw === "" ? clampPanelHeight(PANEL_HEIGHT) : clampPanelHeight(raw);
  }

  function clampFontSize(value) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return DEFAULT_FONT;
    return Math.round(clamp(parsed, MIN_FONT, MAX_FONT));
  }

  function clampFontOffset(value, baseItemFontSize = DEFAULT_FONT) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return 0;
    const parsedBase = Number(baseItemFontSize);
    const base = Number.isFinite(parsedBase) ? parsedBase : DEFAULT_FONT;
    return roundPixel(clamp(parsed, MIN_FONT - base, MAX_FONT - base));
  }

  function readFontOffset() {
    const storedOffset = storage.get(FONT_OFFSET_KEY);
    if (storedOffset != null && storedOffset !== "" && Number.isFinite(Number(storedOffset))) {
      return clampFontOffset(storedOffset);
    }

    const legacyStepwiseFont = storage.get(FONT_KEY);
    if (legacyStepwiseFont != null && legacyStepwiseFont !== "") {
      const migrated = clampFontOffset(clampFontSize(legacyStepwiseFont) - DEFAULT_FONT);
      storage.set(FONT_OFFSET_KEY, String(migrated));
      return migrated;
    }

    const outlineOffset = storage.get(LEGACY_OUTLINE_FONT_OFFSET_KEY);
    if (outlineOffset != null && outlineOffset !== "" && Number.isFinite(Number(outlineOffset))) {
      const migrated = clampFontOffset(outlineOffset);
      storage.set(FONT_OFFSET_KEY, String(migrated));
      return migrated;
    }

    const outlineFont = storage.get(LEGACY_OUTLINE_FONT_KEY);
    const migrated = outlineFont == null || outlineFont === ""
      ? 0
      : clampFontOffset(clampFontSize(outlineFont) - DEFAULT_FONT);
    storage.set(FONT_OFFSET_KEY, String(migrated));
    return migrated;
  }

  // Typography follows the host composer while persisting only the user's relative offset.
  function fallbackHostTypography() {
    const hostFontSize = HOST_FONT_SIZE_FALLBACK;
    return {
      source: "fallback",
      fontFamily: HOST_FONT_FAMILY_FALLBACK,
      fontWeight: 400,
      labelWeight: 500,
      hostFontSize,
      baseItemFontSize: roundPixel(hostFontSize * ITEM_FONT_RATIO),
      chromeFontSize: roundPixel(hostFontSize * CHROME_FONT_RATIO),
      iconFontSize: roundPixel(hostFontSize * ICON_FONT_RATIO),
    };
  }

  function hostTypographySource() {
    const trigger = visibleTypographyNode("[data-codex-intelligence-trigger]");
    if (trigger) return { element: trigger, source: "model-trigger" };
    const composer = visibleTypographyNode(
      '[data-codex-composer] .ProseMirror, [data-codex-composer] [contenteditable="true"], .ProseMirror, [contenteditable="true"]'
    );
    if (composer) return { element: composer, source: "composer" };
    const textarea = visibleTypographyNode("textarea");
    if (textarea) return { element: textarea, source: "textarea" };
    if (document.body) return { element: document.body, source: "body" };
    return { element: document.documentElement, source: "document" };
  }

  function readHostTypography() {
    const { element, source } = hostTypographySource();
    if (!(element instanceof Element)) return fallbackHostTypography();
    const computed = getComputedStyle(element);
    const parsedSize = Number.parseFloat(computed.fontSize);
    const parsedWeight = Number.parseInt(computed.fontWeight, 10);
    const hostFontSize = clamp(
      Number.isFinite(parsedSize) ? parsedSize : HOST_FONT_SIZE_FALLBACK,
      HOST_FONT_SIZE_MIN,
      HOST_FONT_SIZE_MAX
    );
    const fontWeight = Number.isFinite(parsedWeight) ? parsedWeight : 400;
    return {
      source,
      fontFamily: computed.fontFamily || HOST_FONT_FAMILY_FALLBACK,
      fontWeight,
      labelWeight: clamp(fontWeight + 100, 500, 700),
      hostFontSize: roundPixel(hostFontSize),
      baseItemFontSize: roundPixel(hostFontSize * ITEM_FONT_RATIO),
      chromeFontSize: roundPixel(hostFontSize * CHROME_FONT_RATIO),
      iconFontSize: roundPixel(hostFontSize * ICON_FONT_RATIO),
    };
  }

  function typographyFingerprint(value) {
    return [
      value.source,
      value.fontFamily,
      value.fontWeight,
      value.hostFontSize,
      value.baseItemFontSize,
    ].join("|");
  }

  function effectiveFontSize(typography = state.hostTypography) {
    return clampFontSize(typography.baseItemFontSize + state.fontOffset);
  }

  function persistFontPreference() {
    storage.set(FONT_OFFSET_KEY, String(state.fontOffset));
    storage.set(FONT_KEY, String(effectiveFontSize()));
  }

  function setPixelVariable(element, property, value) {
    if (!(element instanceof HTMLElement)) return;
    const next = `${roundPixel(value)}px`;
    if (element.style.getPropertyValue(property) !== next) {
      element.style.setProperty(property, next);
    }
  }

  function applyTypographyVariables() {
    if (!state.root) return;
    state.root.style.setProperty("--csw-font-family", state.hostTypography.fontFamily);
    state.root.style.setProperty("--csw-font-weight", String(state.hostTypography.fontWeight));
    state.root.style.setProperty("--csw-label-weight", String(state.hostTypography.labelWeight));
    setPixelVariable(state.root, "--csw-item-font", effectiveFontSize());
    setPixelVariable(state.root, "--csw-chrome-font", state.hostTypography.chromeFontSize);
    setPixelVariable(state.root, "--csw-icon-font", state.hostTypography.iconFontSize);
  }

  function installTypographyObserver() {
    if (state.typographyObserver || !document.documentElement) return;
    state.typographyObserver = new MutationObserver(() => syncHostTypography());
    const options = {
      attributes: true,
      attributeFilter: ["class", "style", "data-theme", "data-appearance", "data-color-mode"],
    };
    state.typographyObserver.observe(document.documentElement, options);
    if (document.body) state.typographyObserver.observe(document.body, options);
  }

  function writeFontSize(value) {
    const parsed = Number(value);
    const requested = clampFontSize(Number.isFinite(parsed) ? parsed : effectiveFontSize());
    const baseItemFontSize = state.hostTypography.baseItemFontSize;
    state.fontOffset = clampFontOffset(requested - baseItemFontSize, baseItemFontSize);
    persistFontPreference();
    applyTypographyVariables();
  }

  function bumpFontSize(delta) {
    writeFontSize(effectiveFontSize() + delta);
    if (state.open) renderFloat({ preserveMorph: true });
  }

  function fontSizeLabel() {
    return `${effectiveFontSize()}px`;
  }

  // Material v3 migrates legacy names once, then preserves explicit user choices.
  function normalizeMaterial(value) {
    if (MATERIAL_MODES.includes(value)) return value;
    return {
      glass: "frosted",
      solid: "matte",
      opaque: "matte",
    }[value] || DEFAULT_MATERIAL;
  }

  function migrateLegacyMaterial(value) {
    return LEGACY_MATERIAL_MODES[value] || DEFAULT_MATERIAL;
  }

  function migrateMaterialStorageV3() {
    const previous = storage.get(PREVIOUS_MATERIAL_KEY);
    const legacy = storage.get(LEGACY_MATERIAL_KEY);
    const previousIsUserChoice = MATERIAL_MODES.includes(previous)
      && (legacy === null || previous !== migrateLegacyMaterial(legacy));
    return previousIsUserChoice
      ? { material: previous, origin: "user" }
      : { material: DEFAULT_MATERIAL, origin: "default" };
  }

  function materialLabel(value = state.material) {
    return {
      frosted: "磨砂",
      clear: "通透",
      liquid: "液态",
      crystal: "冰晶",
      matte: "哑光",
    }[normalizeMaterial(value)];
  }

  function nextMaterial(value = state.material) {
    const index = MATERIAL_MODES.indexOf(normalizeMaterial(value));
    return MATERIAL_MODES[(index + 1) % MATERIAL_MODES.length];
  }

  function readMaterial() {
    const stored = storage.get(MATERIAL_KEY);
    if (MATERIAL_MODES.includes(stored)) return stored;
    if (storage.get(MATERIAL_MIGRATION_KEY) === "true") {
      storage.set(MATERIAL_KEY, DEFAULT_MATERIAL);
      storage.set(MATERIAL_ORIGIN_KEY, "default");
      return DEFAULT_MATERIAL;
    }
    const migrated = migrateMaterialStorageV3();
    storage.set(MATERIAL_KEY, migrated.material);
    storage.set(MATERIAL_ORIGIN_KEY, migrated.origin);
    storage.set(MATERIAL_MIGRATION_KEY, "true");
    return migrated.material;
  }

  function materialButtonLabel() {
    return `外观：${materialLabel()}；切换为${materialLabel(nextMaterial())}`;
  }

  function materialValueLabel() {
    return materialLabel();
  }

  function applyMaterial(options = {}) {
    const mode = normalizeMaterial(state.material);
    const animate = options.animate !== false;
    state.material = mode;
    state.root?.setAttribute("data-material", mode);
    state.popover?.setAttribute("data-material", mode);
    if (state.materialAnimTimer) window.clearTimeout(state.materialAnimTimer);
    state.materialAnimTimer = 0;
    if (animate) {
      state.popover?.setAttribute("data-material-animating", "true");
      state.materialAnimTimer = window.setTimeout(() => {
        state.popover?.removeAttribute("data-material-animating");
        state.materialAnimTimer = 0;
      }, 260);
    } else {
      state.popover?.removeAttribute("data-material-animating");
    }
    const button = state.panel?.querySelector("[data-action='material']");
    if (button) {
      button.dataset.material = mode;
      button.removeAttribute("aria-pressed");
      button.setAttribute("aria-label", materialButtonLabel());
      button.setAttribute("title", materialButtonLabel());
      const value = button.querySelector("[data-material-value]");
      if (value) value.textContent = materialValueLabel();
    }
    if (state.popover?.hasAttribute("data-csw-hot-hover") === true) {
      updateMaterialDistortion(state.open, true);
    } else {
      resetGlassPointer();
    }
  }

  function writeMaterial(value) {
    state.material = normalizeMaterial(value);
    storage.set(MATERIAL_KEY, state.material);
    storage.set(MATERIAL_ORIGIN_KEY, "user");
    storage.set(MATERIAL_MIGRATION_KEY, "true");
    applyMaterial();
    return state.material;
  }

  function toggleMaterial(event) {
    event?.preventDefault();
    event?.stopPropagation();
    return writeMaterial(nextMaterial());
  }

  function toggleLabelOnly(event) {
    event?.preventDefault();
    event?.stopPropagation();
    state.labelOnly = !state.labelOnly;
    storage.set(LABEL_ONLY_KEY, String(state.labelOnly));
    if (state.open) renderFloat({ preserveMorph: true });
    return state.labelOnly;
  }

  // Diagnostics remain local and compact so injection problems can be inspected without logging chat text.
  function rectSummary(node) {
    const rect = visibleRect(node);
    if (!rect) return null;
    return {
      left: Math.round(rect.left),
      top: Math.round(rect.top),
      right: Math.round(rect.right),
      bottom: Math.round(rect.bottom),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  }

  function readDiagnostics() {
    try {
      const parsed = JSON.parse(sessionStorage.getItem(DIAGNOSTICS_KEY) || "[]");
      return Array.isArray(parsed) ? parsed.slice(-MAX_DIAGNOSTICS) : [];
    } catch {
      return [];
    }
  }

  function writeDiagnostics() {
    try {
      sessionStorage.setItem(DIAGNOSTICS_KEY, JSON.stringify(state.diagnostics.slice(-MAX_DIAGNOSTICS)));
    } catch {}
  }

  function pushDiagnostic(event, details = {}) {
    state.diagnostics.push({
      at: new Date().toISOString(),
      instanceId: INSTANCE_ID,
      event,
      details,
    });
    if (state.diagnostics.length > MAX_DIAGNOSTICS) {
      state.diagnostics.splice(0, state.diagnostics.length - MAX_DIAGNOSTICS);
    }
    writeDiagnostics();
  }

  function visibleRect(node) {
    if (!(node instanceof Element)) return null;
    const rect = node.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    return rect;
  }

  function visibleElement(node) {
    const rect = visibleRect(node);
    return Boolean(rect && rect.width > 20 && rect.height > 10 && rect.bottom > 0 && rect.top < window.innerHeight);
  }

  function parseRgb(color) {
    const match = String(color || "").match(/rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?/i);
    if (!match) return null;
    return {
      r: Number(match[1]),
      g: Number(match[2]),
      b: Number(match[3]),
      a: match[4] === undefined ? 1 : Number(match[4]),
    };
  }

  function luminance(rgb) {
    if (!rgb) return 0;
    return 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
  }

  // Theme and typography adapters observe ChatGPT without taking ownership of its settings.
  function detectCodexTheme() {
    const rootClass = document.documentElement.classList;
    if (rootClass.contains("electron-dark") || rootClass.contains("theme-dark")) return "dark";
    if (rootClass.contains("electron-light") || rootClass.contains("theme-light")) return "light";

    const bodyClass = document.body?.classList;
    if (bodyClass?.contains("electron-dark") || bodyClass?.contains("theme-dark")) return "dark";
    if (bodyClass?.contains("electron-light") || bodyClass?.contains("theme-light")) return "light";

    const explicitTokens = [
      document.documentElement.getAttribute("data-theme"),
      document.documentElement.getAttribute("color-scheme"),
      document.body?.getAttribute("data-theme"),
      getComputedStyle(document.documentElement).colorScheme,
    ].join(" ");
    if (/\bdark\b/i.test(explicitTokens)) return "dark";
    if (/\blight\b/i.test(explicitTokens)) return "light";

    const candidates = [
      document.querySelector(".thread-scroll-container"),
      document.querySelector("main"),
      document.body,
      document.documentElement,
    ].filter(Boolean);
    for (const node of candidates) {
      const color = getComputedStyle(node).backgroundColor;
      const rgb = parseRgb(color);
      if (rgb && rgb.a > 0.05 && luminance(rgb) > 5) return luminance(rgb) < 128 ? "dark" : "light";
    }
    return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }

  function syncTheme() {
    localStorage.removeItem("codex-stepwise-theme-mode-v1");
    state.themeMode = "auto";
    state.theme = detectCodexTheme();
    state.root?.setAttribute("data-theme", state.theme);
    state.root?.setAttribute("data-theme-mode", state.themeMode);
    syncHostTypography();
  }

  function visibleTypographyNode(selector) {
    return Array.from(document.querySelectorAll(selector)).find((node) => node.getClientRects().length > 0) || null;
  }

  function syncHostTypography(force = false) {
    if (!state.root) return;
    const next = readHostTypography();
    const changed = force
      || typographyFingerprint(next) !== typographyFingerprint(state.hostTypography);
    if (changed) state.hostTypography = next;
    applyTypographyVariables();
    if (changed || force) persistFontPreference();
  }

  function appActionModuleCandidates() {
    const candidates = new Set();
    const add = (value) => {
      if (!value) return;
      try {
        const url = new URL(value, location.href);
        if (/\/assets\/rpc-[^/]+\.js$/.test(url.pathname)) candidates.add(`.${url.pathname}`);
      } catch {}
    };

    document.querySelectorAll("script[src],link[href]").forEach((node) => {
      add(node.getAttribute("src") || node.getAttribute("href"));
    });
    const resources = performance.getEntriesByType?.("resource") || [];
    resources.forEach((entry) => add(entry.name));
    return Array.from(candidates);
  }

  async function getCodexAppActions() {
    if (!codexAppActionsPromise) {
      codexAppActionsPromise = (async () => {
        const errors = [];
        for (const candidate of appActionModuleCandidates()) {
          try {
            const module = await import(candidate);
            const appActions = module?.n?.appActions || module?.appServices?.appActions;
            if (typeof appActions?.runInPrimaryWindow === "function") return appActions;
            errors.push(`${candidate}: missing appActions`);
          } catch (error) {
            errors.push(`${candidate}: ${error.message}`);
          }
        }
        throw new Error(`Codex app actions unavailable (${errors.join("; ")})`);
      })();
    }

    try {
      return await codexAppActionsPromise;
    } catch (error) {
      codexAppActionsPromise = null;
      throw error;
    }
  }

  async function setCodexThemeMode(theme) {
    if (theme !== "light" && theme !== "dark") return;
    const appActions = await getCodexAppActions();
    await appActions.runInPrimaryWindow({
      action: { type: "app.appearance.set_mode", mode: theme },
    });
  }

  function toggleCodexTheme() {
    const nextTheme = detectCodexTheme() === "dark" ? "light" : "dark";
    setCodexThemeMode(nextTheme)
      .then(() => {
        const before = `${state.themeMode}:${state.theme}`;
        syncTheme();
        if (state.open && before !== `${state.themeMode}:${state.theme}`) renderFloat();
      })
      .catch((error) => {
        console.warn("[Codex++ Stepwise] Failed to switch Codex theme", error);
      });
  }

  function themeLabel() {
    return state.theme === "dark" ? "主题：深色；切换到浅色主题" : "主题：浅色；切换到深色主题";
  }

  function iconSvg(name) {
    const common = `fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"`;
    if (name === "next") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M5 7.5h8.5M5 12h11M5 16.5h7"/><path ${common} d="m15.5 7.5 3 2.5-3 2.5"/></svg>`;
    }
    if (name === "outline") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M8 6h11M8 12h8M8 18h6"/><circle fill="currentColor" cx="4.5" cy="6" r="1.2"/><circle fill="currentColor" cx="4.5" cy="12" r="1.2"/><circle fill="currentColor" cx="4.5" cy="18" r="1.2"/></svg>`;
    }
    if (name === "settings") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M12 8.8a3.2 3.2 0 1 0 0 6.4 3.2 3.2 0 0 0 0-6.4Z"/><path ${common} d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.04.04a2 2 0 0 1-2.83 2.83l-.04-.04a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 0 1-4 0v-.06a1.7 1.7 0 0 0-1.03-1.56 1.7 1.7 0 0 0-1.88.34l-.04.04a2 2 0 1 1-2.83-2.83l.04-.04A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 0 1 0-4h.06A1.7 1.7 0 0 0 4.6 8.96a1.7 1.7 0 0 0-.34-1.88l-.04-.04A2 2 0 1 1 7.05 4.2l.04.04a1.7 1.7 0 0 0 1.88.34H9A1.7 1.7 0 0 0 10 3.06V3a2 2 0 0 1 4 0v.06a1.7 1.7 0 0 0 1.03 1.56h.03a1.7 1.7 0 0 0 1.88-.34l.04-.04a2 2 0 1 1 2.83 2.83l-.04.04a1.7 1.7 0 0 0-.34 1.88v.03A1.7 1.7 0 0 0 20.94 10H21a2 2 0 0 1 0 4h-.06A1.7 1.7 0 0 0 19.4 15Z"/></svg>`;
    }
    if (name === "open-config") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M3.5 6h7M14.5 6h6M3.5 12h3M10.5 12h10M3.5 18h9M16.5 18h4"/><path ${common} d="M12.5 3.8v4.4M8.5 9.8v4.4M14.5 15.8v4.4"/></svg>`;
    }
    if (name === "moon") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path fill="currentColor" d="M20.1 14.8A8.2 8.2 0 0 1 9.2 3.9a.9.9 0 0 0-1.1-1.1 9.8 9.8 0 1 0 13.1 13.1.9.9 0 0 0-1.1-1.1Z"/></svg>`;
    }
    if (name === "sun") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><circle ${common} cx="12" cy="12" r="4.3"/><path ${common} d="M12 2.6v2.2M12 19.2v2.2M2.6 12h2.2M19.2 12h2.2M5.35 5.35 6.9 6.9M17.1 17.1l1.55 1.55M18.65 5.35 17.1 6.9M6.9 17.1l-1.55 1.55"/></svg>`;
    }
    if (name === "refresh") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M20 11a8 8 0 0 0-14.1-5.2L4 8"/><path ${common} d="M4 4v4h4"/><path ${common} d="M4 13a8 8 0 0 0 14.1 5.2L20 16"/><path ${common} d="M20 20v-4h-4"/></svg>`;
    }
    if (name === "connection") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="m8.2 15.8-1.4 1.4a3.4 3.4 0 0 1-4.8-4.8l3.2-3.2A3.4 3.4 0 0 1 10 9"/><path ${common} d="m15.8 8.2 1.4-1.4a3.4 3.4 0 0 1 4.8 4.8l-3.2 3.2A3.4 3.4 0 0 1 14 15"/><path ${common} d="m8.5 15.5 7-7"/></svg>`;
    }
    if (name === "turn-start") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M5 5h14M12 19V8m-4 4 4-4 4 4"/></svg>`;
    }
    if (name === "turn-end") {
      return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M5 19h14M12 5v11m-4-4 4 4 4-4"/></svg>`;
    }
    return `<svg aria-hidden="true" viewBox="0 0 24 24"><path ${common} d="M6 6l12 12M18 6 6 18"/></svg>`;
  }

  function themeIcon() {
    return state.theme === "dark" ? iconSvg("sun") : iconSvg("moon");
  }

  function installThemeObserver() {
    if (state.themeObserver) return;

    let frame = 0;
    const update = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const before = `${state.themeMode}:${state.theme}`;
        syncTheme();
        if (state.open && before !== `${state.themeMode}:${state.theme}`) renderFloat();
      });
    };

    state.themeObserver = new MutationObserver(update);
    [document.documentElement, document.body].filter(Boolean).forEach((node) => {
      state.themeObserver.observe(node, {
        attributes: true,
        attributeFilter: ["class", "style", "data-theme", "color-scheme"],
      });
    });
  }

  function stripOwnUi(clone) {
    clone.querySelectorAll?.(`[${ROOT_ATTR}], [${PAYLOAD_ATTR}]`).forEach((item) => item.remove());
    return clone;
  }

  function elementText(node) {
    if (!(node instanceof Element)) return normalizeText(node?.textContent || "");
    return normalizeText(stripOwnUi(node.cloneNode(true)).textContent || "");
  }

  function directText(node) {
    if (!(node instanceof Element)) return "";
    const clone = stripOwnUi(node.cloneNode(true));
    clone.querySelectorAll?.("button,[role='button'],svg").forEach((item) => item.remove());
    return normalizeText(clone.textContent || "");
  }

  // One stylesheet owns the shell, materials, views, responsive layout, and reduced-motion states.
  function installStyle() {
    if (document.getElementById(STYLE_ID)) return;

    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      [${ROOT_ATTR}="true"] {
        --csw-surface-opaque: var(--color-background-elevated-primary-opaque, var(--color-token-dropdown-background, var(--main-surface-primary, #FAFAFA)));
        --csw-text: var(--color-token-text-primary, var(--color-token-foreground, var(--text-primary, #202020)));
        --csw-muted: var(--color-token-text-tertiary, var(--color-token-description-foreground, #6F6F6F));
        --csw-faint: color-mix(in srgb, var(--csw-text) 34%, transparent);
        --csw-accent: var(--color-token-charts-blue, #4D8DFF);
        --csw-danger: #dc5d67;
        --csw-ready: var(--csw-accent);
        --csw-hover: var(--color-token-list-hover-background, color-mix(in srgb, var(--csw-text) 6%, transparent));
        --csw-divider: color-mix(in srgb, var(--csw-text) 9%, transparent);
        --csw-glass-x: 28%;
        --csw-glass-y: 22%;
        --csw-glass-strength: 0;
        --csw-glass-rim-width: 140%;
        --csw-glass-rim-height: 120%;
        --csw-glass-px: 0px;
        --csw-glass-py: 0px;
        --csw-glass-angle: -40deg;
        --csw-glass-edge: rgba(108, 128, 152, 0.4);
        --csw-glass-edge-hi: rgba(168, 190, 214, 0.7);
        --csw-hover-core: 0.13;
        --csw-hover-mid: 0.04;
        --csw-hover-layer-opacity: 0.85;
        --csw-hover-rim-gain: 10%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 190, 210, 230;
        --csw-frost-noise: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='180' height='180' viewBox='0 0 180 180'%3E%3Cfilter id='n' color-interpolation-filters='sRGB'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.008' numOctaves='2' seed='92' stitchTiles='stitch'/%3E%3CfeGaussianBlur stdDeviation='2'/%3E%3CfeComponentTransfer%3E%3CfeFuncA type='table' tableValues='0 .055'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)' opacity='.55'/%3E%3C/svg%3E");
        --csw-eye-x: 0px;
        --csw-eye-y: 0px;
        --csw-curious-eye-x: 0px;
        --csw-curious-eye-y: 0px;
        --csw-panel-width: ${PANEL_WIDTH}px;
        --csw-panel-height: ${PANEL_HEIGHT}px;
        --csw-item-font: ${DEFAULT_FONT}px;
        --csw-chrome-font: 12px;
        --csw-icon-font: 16px;
        color: var(--csw-text);
        font-family: var(--csw-font-family, -apple-system, system-ui, "Segoe UI", sans-serif);
        font-size: var(--csw-item-font);
        font-weight: var(--csw-font-weight, 400);
        line-height: 1.4;
        inset: 0;
        letter-spacing: 0;
        pointer-events: none;
        position: fixed;
        z-index: 2147483000;
      }

      [${ROOT_ATTR}="true"][data-hidden="true"] {
        display: none !important;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] {
        --csw-surface-opaque: var(--color-background-elevated-primary-opaque, var(--color-token-dropdown-background, #2B2B2B));
        --csw-text: var(--color-token-text-primary, var(--color-token-foreground, #F3F3F3));
        --csw-muted: var(--color-token-text-tertiary, var(--color-token-description-foreground, #AAAAAA));
        --csw-faint: color-mix(in srgb, var(--csw-text) 32%, transparent);
        --csw-accent: var(--color-token-charts-blue, #4D8DFF);
        --csw-danger: #ff7f89;
        --csw-ready: var(--csw-accent);
        --csw-hover: var(--color-token-list-hover-background, rgba(255, 255, 255, 0.078));
        --csw-divider: rgba(255, 255, 255, 0.09);
        --csw-glass-strength: 0;
        --csw-glass-edge: rgba(132, 154, 180, 0.36);
        --csw-glass-edge-hi: rgba(178, 200, 224, 0.62);
        color: var(--csw-text);
      }

      [${PAYLOAD_ATTR}="true"],
      [${PAYLOAD_ATTR}="block"] {
        display: none !important;
      }

      .csw-popover {
        height: var(--csw-panel-height);
        isolation: isolate;
        pointer-events: none;
        position: fixed;
        width: var(--csw-panel-width);
      }

      .csw-material-layer {
        inset: 0;
        isolation: isolate;
        pointer-events: none;
        position: absolute;
        z-index: 0;
      }

      .csw-material-layer,
      .csw-material-layer * {
        pointer-events: none;
      }

      .csw-glass {
        -webkit-backdrop-filter: blur(18px) saturate(165%) contrast(1.04) brightness(1.04);
        backdrop-filter: blur(18px) saturate(165%) contrast(1.04) brightness(1.04);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 68%, transparent);
        background-image: linear-gradient(160deg, rgba(255, 255, 255, 0.11) 0%, rgba(255, 255, 255, 0.032) 48%, rgba(150, 170, 195, 0.032) 100%);
        border: 0;
        border-radius: ${CHIP_RADIUS}px;
        box-shadow: none;
        box-sizing: border-box;
        height: ${CHIP_HEIGHT}px;
        left: var(--csw-chip-left, ${Math.max(0, (PANEL_WIDTH - CHIP_WIDTH) / 2)}px);
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        top: 0;
        transition: background-color 0.2s ease, box-shadow 0.2s ease, backdrop-filter 0.2s ease, -webkit-backdrop-filter 0.2s ease;
        width: ${CHIP_WIDTH}px;
        z-index: 1;
      }

      .csw-rim {
        background: var(--csw-glass-edge);
        border-radius: ${CHIP_RADIUS}px;
        box-sizing: border-box;
        height: ${CHIP_HEIGHT}px;
        left: var(--csw-chip-left, ${Math.max(0, (PANEL_WIDTH - CHIP_WIDTH) / 2)}px);
        -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        -webkit-mask-composite: xor;
        mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        mask-composite: exclude;
        padding: 1px;
        pointer-events: none;
        position: absolute;
        top: 0;
        width: ${CHIP_WIDTH}px;
        z-index: 4;
      }

      .csw-popover[data-csw-hot-hover] .csw-rim {
        background: conic-gradient(
          from calc(var(--csw-glass-angle, -40deg) + 90deg),
          color-mix(in srgb, var(--csw-glass-edge-hi) var(--csw-hover-rim-gain, 0%), var(--csw-glass-edge)) 0deg,
          var(--csw-glass-edge) 64deg,
          var(--csw-glass-edge) 296deg,
          color-mix(in srgb, var(--csw-glass-edge-hi) var(--csw-hover-rim-gain, 0%), var(--csw-glass-edge)) 360deg
        );
      }

      .csw-popover[data-morphing="true"] .csw-glass,
      .csw-popover[data-morphing="true"] .csw-rim,
      .csw-popover[data-morphing="true"] .csw-displacement-texture {
        will-change: left, top, width, height, border-radius;
      }

      .csw-popover[data-snap-right="true"] {
        transition: left 180ms cubic-bezier(.22, .72, 0, 1), top 180ms cubic-bezier(.22, .72, 0, 1);
      }

      .csw-popover[data-snap-right="true"] .csw-fab,
      .csw-popover[data-snap-right="true"] .csw-glass,
      .csw-popover[data-snap-right="true"] .csw-rim,
      .csw-popover[data-snap-right="true"] .csw-displacement-texture {
        transition-property: left, top;
        transition-duration: 180ms;
        transition-timing-function: cubic-bezier(.22, .72, 0, 1);
      }

      .csw-completion-beam {
        box-sizing: border-box;
        color: var(--csw-text);
        -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        -webkit-mask-composite: xor;
        mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        mask-composite: exclude;
        opacity: 0;
        overflow: hidden;
        padding: 1px;
        pointer-events: none;
        position: absolute;
        z-index: 5;
      }

      .csw-completion-beam::before {
        background: conic-gradient(
          from 0deg,
          transparent 0deg,
          transparent 302deg,
          color-mix(in srgb, currentColor 22%, transparent) 320deg,
          color-mix(in srgb, currentColor 72%, transparent) 338deg,
          transparent 360deg
        );
        content: "";
        inset: -170%;
        opacity: 0;
        position: absolute;
        transform: rotate(-64deg);
        transform-origin: center;
        will-change: opacity, transform;
      }

      .csw-popover[data-morphing="false"][data-completion-beam="true"] .csw-completion-beam {
        opacity: 1;
      }

      .csw-popover[data-morphing="false"][data-completion-beam="true"] .csw-completion-beam::before {
        animation: csw-completion-beam-sweep ${COMPLETION_BEAM_MS}ms cubic-bezier(.22, .78, .18, 1) 1 both;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-glass {
        -webkit-backdrop-filter: blur(18px) saturate(165%) contrast(1.05) brightness(1.08);
        backdrop-filter: blur(18px) saturate(165%) contrast(1.05) brightness(1.08);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 60%, transparent);
        background-image: linear-gradient(160deg, rgba(160, 185, 215, 0.055) 0%, rgba(255, 255, 255, 0.012) 48%, rgba(30, 45, 70, 0.045) 100%);
      }

      .csw-popover[data-material="matte"] {
        --csw-hover-core: 0.07;
        --csw-hover-mid: 0.022;
        --csw-hover-layer-opacity: 0.75;
        --csw-hover-rim-gain: 6%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 205, 215, 225;
      }

      .csw-popover[data-material="frosted"] {
        --csw-hover-core: 0.13;
        --csw-hover-mid: 0.04;
        --csw-hover-layer-opacity: 0.85;
        --csw-hover-rim-gain: 10%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 190, 210, 230;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="frosted"] {
        --csw-hover-core: 0.11;
        --csw-hover-mid: 0.034;
      }

      .csw-popover[data-material="clear"] {
        --csw-hover-core: 0.12;
        --csw-hover-mid: 0.038;
        --csw-hover-layer-opacity: 0.65;
        --csw-hover-rim-gain: 8%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 174, 214, 255;
      }

      .csw-popover[data-material="liquid"] {
        --csw-hover-core: 0.15;
        --csw-hover-mid: 0.045;
        --csw-hover-layer-opacity: 0.72;
        --csw-hover-rim-gain: 9%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 145, 205, 255;
      }

      .csw-popover[data-material="crystal"] {
        --csw-hover-core: 0.14;
        --csw-hover-mid: 0.045;
        --csw-hover-layer-opacity: 0.68;
        --csw-hover-rim-gain: 9%;
        --csw-hover-core-color: 242, 252, 255;
        --csw-hover-mid-color: 122, 199, 255;
      }

      .csw-popover[data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 18%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
        background-repeat: no-repeat;
        background-size: cover;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 24%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
      }

      .csw-popover[data-open="true"] {
        --csw-glass-rim-width: 92%;
        --csw-glass-rim-height: 72%;
      }

      .csw-popover[data-open="false"][data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 12%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
        background-repeat: no-repeat;
        background-size: cover;
        box-shadow: none;
        filter: none !important;
        isolation: auto;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-open="false"][data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 18%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
        box-shadow: none;
      }

      .csw-popover[data-material="matte"] .csw-glass {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background-color: var(--csw-surface-opaque);
        background-image: none;
      }

      .csw-popover[data-material="clear"] .csw-glass {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background-color: transparent;
        background-image: none;
        isolation: isolate;
      }

      .csw-clear-texture {
        -webkit-backdrop-filter: none;
        backdrop-filter: url(#${CLEAR_FILTER_ID});
        background-color: transparent;
        background-image: none;
        border-radius: inherit;
        display: none;
        filter: none;
        inset: -12px;
        opacity: 1;
        pointer-events: none;
        position: absolute;
        transform: translateZ(0);
        will-change: backdrop-filter;
        z-index: 0;
      }

      .csw-popover[data-material="clear"] .csw-clear-texture {
        display: block;
      }

      .csw-clear-distortion {
        -webkit-backdrop-filter: none;
        -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        -webkit-mask-composite: xor;
        backdrop-filter: none;
        background: transparent;
        border-radius: inherit;
        box-sizing: border-box;
        display: none;
        filter: none;
        inset: 1px;
        mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        mask-composite: exclude;
        opacity: 1;
        padding: 1px;
        pointer-events: none;
        position: absolute;
        transform: translate3d(var(--csw-glass-px, 0px), var(--csw-glass-py, 0px), 0);
        transition: opacity 0.18s ease, transform 0.14s cubic-bezier(0.23, 1, 0.32, 1);
        will-change: transform;
        z-index: 0;
      }

      .csw-popover[data-material="clear"] .csw-clear-distortion {
        display: block;
      }

      .csw-popover[data-material="liquid"] .csw-glass,
      .csw-popover[data-material="crystal"] .csw-glass {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background-color: rgba(255, 255, 255, 0);
        background-image: none;
        isolation: isolate;
      }

      .csw-popover[data-material="liquid"] .csw-glass::before,
      .csw-popover[data-material="crystal"] .csw-glass::before {
        box-shadow: none;
        mix-blend-mode: screen;
        transform: translate3d(var(--csw-glass-px, 0px), var(--csw-glass-py, 0px), 0);
        z-index: 1;
      }

      .csw-displacement-texture {
        border-radius: ${CHIP_RADIUS}px;
        display: none;
        height: ${CHIP_HEIGHT}px;
        isolation: isolate;
        left: var(--csw-chip-left, ${Math.max(0, (PANEL_WIDTH - CHIP_WIDTH) / 2)}px);
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        top: 0;
        width: ${CHIP_WIDTH}px;
        z-index: 0;
      }

      .csw-displacement-texture::before {
        border-radius: inherit;
        content: "";
        inset: 0;
        pointer-events: none;
        position: absolute;
      }

      .csw-popover[data-material="liquid"] .csw-displacement-texture,
      .csw-popover[data-material="crystal"] .csw-displacement-texture {
        display: block;
      }

      .csw-popover[data-material="crystal"] .csw-displacement-texture {
        overflow: visible;
      }

      .csw-popover[data-material="liquid"] .csw-displacement-texture::before {
        -webkit-backdrop-filter: url(#${LIQUID_FILTER_ID}) blur(0.6px) saturate(112%) contrast(1.02);
        backdrop-filter: url(#${LIQUID_FILTER_ID}) blur(0.6px) saturate(112%) contrast(1.02);
        background-color: rgba(255, 255, 255, 0.24);
        -webkit-filter: none;
        filter: none;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="liquid"] .csw-displacement-texture::before {
        background-color: rgba(20, 24, 30, 0.34);
      }

      .csw-popover[data-material="crystal"] .csw-displacement-texture::before {
        -webkit-backdrop-filter: blur(7px);
        backdrop-filter: blur(7px);
        background-color: rgba(255, 255, 255, 0);
        border-radius: 0;
        clip-path: inset(48px round ${PANEL_RADIUS}px);
        inset: -48px;
        -webkit-filter: url(#${CRYSTAL_FILTER_ID});
        filter: url(#${CRYSTAL_FILTER_ID});
      }

      .csw-popover[data-material-animating="true"] .csw-glass {
        transition-duration: 260ms;
      }

      .csw-glass::before {
        background: radial-gradient(
          var(--csw-glass-rim-width, 140%) var(--csw-glass-rim-height, 120%) at var(--csw-glass-x, 28%) var(--csw-glass-y, 22%),
          rgba(var(--csw-hover-core-color, 255, 255, 255), calc(var(--csw-hover-core, 0.13) * var(--csw-glass-strength, 0))) 0%,
          rgba(var(--csw-hover-mid-color, 190, 210, 230), calc(var(--csw-hover-mid, 0.04) * var(--csw-glass-strength, 0))) 22%,
          transparent 50%
        );
        border-radius: inherit;
        content: "";
        inset: 0;
        mix-blend-mode: screen;
        opacity: var(--csw-hover-layer-opacity, 0.85);
        pointer-events: none;
        position: absolute;
        transform: translate3d(var(--csw-glass-px, 0px), var(--csw-glass-py, 0px), 0);
        transition: transform 0.14s cubic-bezier(0.23, 1, 0.32, 1);
        will-change: transform;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-glass {
        box-shadow: none;
      }

      .csw-popover[data-morphing="true"] .csw-glass {
        cursor: pointer;
        pointer-events: auto;
        transition: none;
      }

      .csw-popover[data-resizing="true"],
      .csw-popover[data-resizing="true"] .csw-glass,
      .csw-popover[data-resizing="true"] .csw-rim,
      .csw-popover[data-resizing="true"] .csw-panel {
        transition: none !important;
      }

      .csw-fab {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 999px;
        box-sizing: border-box;
        color: var(--csw-text);
        cursor: grab;
        display: flex;
        height: ${CHIP_HEIGHT}px;
        justify-content: center;
        padding: 0;
        pointer-events: auto;
        position: absolute;
        user-select: none;
        width: ${CHIP_WIDTH}px;
        z-index: 3;
      }

      .csw-fab[data-expression="hidden"] {
        display: none;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-fab {
        opacity: 0;
        pointer-events: none;
        visibility: hidden;
      }

      .csw-popover[data-morphing="true"] .csw-fab {
        opacity: 1;
        pointer-events: none;
        transform: none;
        visibility: visible;
      }

      .csw-fab:active {
        cursor: grabbing;
        transform: scale(0.96);
      }

      .csw-fab:focus-visible {
        outline: 2px solid color-mix(in srgb, var(--csw-accent) 76%, transparent);
        outline-offset: 4px;
      }

      .csw-fab-face {
        align-items: center;
        display: flex;
        gap: 16px;
        height: 27px;
        justify-content: center;
        position: relative;
        width: 52px;
        z-index: 1;
      }

      .csw-status-stage {
        align-items: center;
        display: flex;
        height: 28px;
        justify-content: center;
        position: relative;
        width: 58px;
        z-index: 1;
      }

      .csw-source-track {
        height: var(--csw-source-track-height, ${CHIP_HEIGHT}px);
        left: 50%;
        pointer-events: none;
        position: absolute;
        top: 50%;
        transform: translate(-50%, -50%);
        width: ${CHIP_WIDTH}px;
        z-index: 2;
      }

      .csw-fab-eye {
        background: currentColor;
        border-radius: 999px;
        display: block;
        height: 14px;
        position: relative;
        transform: translate3d(var(--csw-eye-x, 0px), var(--csw-eye-y, 0px), 0);
        transform-origin: center;
        transition: background 170ms ease, border-color 170ms ease, height 170ms ease, transform 170ms ease, width 170ms ease;
        will-change: transform;
        width: 8px;
      }

      .csw-fab-happy-arc {
        display: none;
        height: 100%;
        overflow: visible;
        width: 100%;
      }

      .csw-fab-happy-arc path {
        fill: none;
        stroke: currentColor;
        stroke-linecap: round;
        stroke-linejoin: round;
        stroke-width: 2.6;
        vector-effect: non-scaling-stroke;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="idle"] .csw-fab-eye {
        animation: csw-face-blink 4.8s infinite;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="answering"] .csw-fab-eye {
        height: 13px;
        transition-duration: 70ms;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="surprise"] .csw-fab-eye {
        animation: csw-face-star 1.25s ease-in-out infinite;
        background: currentColor;
        border: 0;
        border-radius: 0;
        clip-path: polygon(50% 0, 61% 36%, 100% 50%, 61% 64%, 50% 100%, 39% 64%, 0 50%, 39% 36%);
        height: 18px;
        width: 18px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="generating"] .csw-fab-eye {
        animation: csw-face-generate-bob .92s cubic-bezier(.45, 0, .2, 1) infinite;
        animation-delay: 0s;
        height: 14px;
        width: 8px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-eye {
        animation: csw-face-happy-lift 1.8s ease-in-out infinite;
        background: transparent;
        border: 0;
        border-radius: 0;
        height: 12px;
        width: 18px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-happy-arc {
        display: block;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-eye::before,
      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-eye::after {
        content: none;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="empty"] .csw-fab-eye {
        animation: csw-face-calm-breathe 3.6s ease-in-out infinite;
        height: 3px;
        width: 16px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye {
        animation: csw-face-error-breathe 3.8s ease-in-out infinite;
        background: transparent;
        color: var(--csw-text);
        height: 14px;
        width: 14px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::before,
      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::after {
        background: currentColor;
        border-radius: 999px;
        content: "";
        height: 2.5px;
        left: 0;
        position: absolute;
        top: 5.75px;
        width: 14px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::before {
        transform: rotate(45deg);
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::after {
        transform: rotate(-45deg);
      }

      :is(.csw-fab, .csw-head-face)[data-expression="curious"] .csw-fab-eye {
        animation: none;
        background: transparent;
        border: 3px solid currentColor;
        border-radius: 50%;
        clip-path: none;
        height: 17px;
        width: 17px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="curious"] .csw-fab-eye::before {
        content: none;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="curious"] .csw-fab-eye::after {
        animation: none;
        background: currentColor;
        border-radius: 50%;
        content: "";
        height: 5px;
        left: 50%;
        position: absolute;
        top: 50%;
        transform: translate(-50%, -50%) translate3d(var(--csw-curious-eye-x, 0px), var(--csw-curious-eye-y, 0px), 0);
        transition: transform 90ms cubic-bezier(.2, .8, .2, 1);
        width: 5px;
      }

      .csw-fab-badge {
        display: none;
      }

      .csw-fab[data-count="0"] .csw-fab-badge {
        display: none;
      }

      .csw-fab:not([data-expression="ready"]) .csw-fab-badge {
        display: none;
      }

      .csw-panel {
        -webkit-backdrop-filter: none !important;
        -webkit-filter: none !important;
        backdrop-filter: none !important;
        border-radius: ${PANEL_RADIUS}px;
        box-sizing: border-box;
        container-name: csw-panel;
        container-type: inline-size;
        display: flex;
        filter: none !important;
        flex-direction: column;
        height: 100%;
        opacity: 0;
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        inset: 0;
        visibility: hidden;
        will-change: clip-path;
        z-index: 2;
      }

      .csw-panel *,
      .csw-panel *::before,
      .csw-panel *::after {
        -webkit-backdrop-filter: none !important;
        -webkit-filter: none !important;
        backdrop-filter: none !important;
        filter: none !important;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-panel {
        opacity: 1;
        pointer-events: auto;
        visibility: visible;
      }

      .csw-popover[data-morphing="true"] .csw-panel {
        opacity: 1;
        pointer-events: none;
        visibility: visible;
      }

      .csw-head {
        align-items: center;
        cursor: grab;
        display: grid;
        flex: 0 0 auto;
        grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
        min-height: 48px;
        padding: 8px 10px;
        touch-action: none;
        user-select: none;
      }

      .csw-head[data-dragging="true"] {
        cursor: grabbing;
      }

      .csw-head-side {
        align-items: center;
        cursor: default;
        display: flex;
        min-width: 0;
        opacity: 0;
        pointer-events: auto;
        transform: translateY(-2px) scale(.98);
        transition:
          opacity .15s cubic-bezier(.23, 1, .32, 1),
          transform .15s cubic-bezier(.23, 1, .32, 1);
        will-change: opacity, transform;
      }

      .csw-head-side .csw-icon {
        pointer-events: none;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:hover .csw-head-side,
      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:has(:focus-visible) .csw-head-side,
      .csw-head[data-dragging="true"] .csw-head-side {
        opacity: 1;
        transform: translateY(0) scale(1);
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:hover .csw-head-side .csw-icon,
      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:has(:focus-visible) .csw-head-side .csw-icon {
        pointer-events: auto;
      }

      .csw-head-left {
        justify-content: flex-start;
      }

      .csw-head-right {
        align-items: center;
        display: flex;
        gap: 2px;
        justify-content: flex-end;
      }

      .csw-head-face {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 999px;
        color: var(--csw-text);
        cursor: grab;
        display: flex;
        height: 32px;
        justify-content: center;
        padding: 0;
        position: relative;
        touch-action: none;
        transition:
          background-color 140ms ease,
          transform 140ms cubic-bezier(.23, 1, .32, 1);
        user-select: none;
        width: ${CHIP_WIDTH}px;
      }

      .csw-head-face:hover {
        background: color-mix(in srgb, var(--csw-text) 4%, transparent);
      }

      .csw-head-face:active {
        background: color-mix(in srgb, var(--csw-text) 6%, transparent);
        transform: scale(.97);
      }

      .csw-source-dot {
        background: color-mix(in srgb, var(--csw-text) 72%, transparent);
        border-radius: 999px;
        box-shadow: none;
        height: 4px;
        left: var(--csw-source-x, 50%);
        opacity: .72;
        pointer-events: none;
        position: absolute;
        top: var(--csw-source-y, 50%);
        transform: translate(-50%, -50%);
        transition: opacity .15s ease;
        width: 4px;
      }

      .csw-source-dot[data-direction="single"] { opacity: 0; }

      :is(.csw-fab, .csw-head-face)[data-expression="generating"] .csw-source-track {
        opacity: 0;
      }

      .csw-head[data-dragging="true"] .csw-head-face {
        cursor: grabbing;
      }

      .csw-head-face:focus-visible {
        background: color-mix(in srgb, var(--csw-text) 5%, transparent);
      }

      .csw-popover[data-morphing="true"] .csw-head-face {
        opacity: 0;
        visibility: hidden;
      }

      .csw-tabs {
        align-items: center;
        display: flex;
        gap: 2px;
      }

      .csw-view-tabs {
        background: color-mix(in srgb, var(--csw-text) 3.5%, transparent);
        border-radius: 10px;
        isolation: isolate;
        padding: 2px;
        position: relative;
      }

      .csw-view-indicator {
        background: color-mix(in srgb, var(--csw-surface-opaque) 74%, transparent);
        border-radius: 8px;
        height: 28px;
        left: 2px;
        opacity: 0;
        pointer-events: none;
        position: absolute;
        top: 2px;
        transform: translate3d(0, 0, 0);
        transition:
          transform ${VIEW_INDICATOR_MS}ms cubic-bezier(.23, 1, .32, 1),
          opacity 110ms cubic-bezier(.23, 1, .32, 1);
        width: 28px;
        will-change: opacity, transform;
        z-index: 0;
      }

      .csw-icon {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 8px;
        color: var(--csw-muted);
        cursor: pointer;
        display: inline-flex;
        font: inherit;
        font-size: var(--csw-chrome-font);
        font-weight: var(--csw-label-weight, 500);
        height: 28px;
        justify-content: center;
        padding: 0;
        transition: background-color 140ms ease-out, color 140ms ease-out, transform 140ms cubic-bezier(.23, 1, .32, 1);
        width: 28px;
      }

      .csw-icon:active {
        transform: scale(.94);
      }

      .csw-view-tabs .csw-icon {
        position: relative;
        transform: scale(1);
        transition:
          color ${VIEW_INDICATOR_MS}ms cubic-bezier(.23, 1, .32, 1),
          transform ${VIEW_INDICATOR_MS}ms cubic-bezier(.23, 1, .32, 1);
        z-index: 1;
      }

      .csw-view-tabs .csw-icon:active {
        transform: scale(.9);
      }

      .csw-icon[data-active="true"],
      .csw-icon:hover {
        background: var(--csw-hover);
        color: var(--csw-text);
      }

      .csw-view-tabs .csw-icon[data-active="true"] {
        background: transparent;
        box-shadow: none;
        transform: scale(1);
      }

      .csw-icon:disabled {
        cursor: not-allowed;
        opacity: .42;
      }

      .csw-icon svg {
        display: block;
        height: var(--csw-icon-font);
        transform-origin: center;
        width: var(--csw-icon-font);
      }

      .csw-icon[data-view="next"] svg {
        transform: scale(1.08);
      }

      .csw-icon[data-action="refresh"] svg {
        transform: scale(.95);
      }

      .csw-icon[data-view="settings"] svg {
        transform: scale(.86);
      }

      .csw-body {
        display: flex;
        flex-direction: column;
        flex: 1 1 auto;
        min-height: 0;
        overflow: auto;
        overflow-anchor: none;
        padding: 2px 16px 14px;
        position: relative;
        scrollbar-color: color-mix(in srgb, var(--csw-text) 18%, transparent) transparent;
        scrollbar-gutter: stable;
        scrollbar-width: thin;
      }

      .csw-mouth-stage {
        display: flex;
        flex: 1 1 auto;
        flex-direction: column;
        min-height: 100%;
        transform-origin: 50% 0;
        will-change: opacity, transform;
      }

      .csw-body[data-view-transition="true"] {
        overflow: hidden;
      }

      .csw-view-transition-layer {
        inset: 0;
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        z-index: 2;
      }

      .csw-view-transition-copy {
        left: 16px;
        margin: 0;
        pointer-events: none;
        position: absolute;
        right: 16px;
      }

      .csw-mouth-stage[data-mouth-stage="settings"] {
        height: 100%;
      }

      .csw-body[data-view-body="next"] {
        overflow: auto;
      }

      .csw-popover[data-content-fade="true"] .csw-body[data-view-body="next"],
      .csw-popover[data-content-fade="true"] .csw-body[data-view-body="outline"] {
        -webkit-mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - var(--csw-content-fade-size, 24px))),
          transparent 100%
        );
        mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - var(--csw-content-fade-size, 24px))),
          transparent 100%
        );
        -webkit-mask-repeat: no-repeat;
        mask-repeat: no-repeat;
      }

      .csw-mouth-stage[data-mouth-stage="next"] {
        height: auto;
        min-height: 100%;
      }

      .csw-next-layout {
        display: grid;
        flex: 1 1 auto;
        gap: 16px;
        grid-template-rows: max-content minmax(clamp(168px, 28vh, 240px), auto);
        height: auto;
        min-height: 100%;
        width: 100%;
      }

      .csw-next-layout::after {
        content: "";
        height: 8px;
      }

      .csw-list {
        display: grid;
        align-content: start;
        align-self: start;
        flex: 0 0 auto;
        gap: 6px;
        grid-auto-rows: max-content;
        height: max-content;
        min-height: max-content;
        overflow: visible;
        padding: 4px 2px 6px;
        width: 100%;
      }

      .csw-row {
        align-items: start;
        appearance: none;
        background: transparent;
        border: 0;
        border-top: 0;
        border-radius: 13px;
        color: inherit;
        cursor: pointer;
        display: grid;
        gap: 12px;
        grid-template-columns: minmax(0, 1fr) 18px;
        isolation: isolate;
        box-sizing: border-box;
        min-height: 64px;
        min-width: 0;
        overflow: hidden;
        padding: 13px 10px;
        position: relative;
        text-align: left;
        transition: background 140ms ease-out, color 140ms ease-out, transform 90ms ease-out;
        width: 100%;
      }

      .csw-row:active {
        transform: scale(.985);
      }

      .csw-row::before {
        background: var(--csw-row-surface);
        border-radius: inherit;
        content: "";
        inset: 0;
        opacity: 0;
        pointer-events: none;
        position: absolute;
        transition: background 180ms ease-out, opacity 160ms ease-out;
        z-index: -1;
      }

      .csw-popover[data-material="frosted"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-surface-opaque) 28%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 8.5%, transparent);
      }

      .csw-popover[data-material="clear"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-text) 3.5%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 8%, transparent);
      }

      .csw-popover[data-material="liquid"],
      .csw-popover[data-material="crystal"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-text) 5%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 9%, transparent);
      }

      .csw-popover[data-material="matte"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-surface-opaque) 82%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 7%, transparent);
      }

      .csw-row:first-child {
        border-top: 0;
      }

      .csw-row:hover,
      .csw-row:focus-visible,
      .csw-row:focus-within {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-text) 12%, transparent);
        outline-offset: -1px;
      }

      .csw-row:hover::before,
      .csw-row:focus-visible::before,
      .csw-row:focus-within::before {
        opacity: 1;
      }

      .csw-row[data-preview-active="true"] {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-accent) 34%, transparent);
        outline-offset: -1px;
      }

      .csw-row[data-preview-active="true"]::before {
        background: var(--csw-row-selected);
        opacity: 1;
      }

      .csw-row:active {
        transform: scale(.992);
      }

      .csw-row-copy {
        display: block;
        min-width: 0;
        overflow: hidden;
      }

      .csw-row-label {
        color: var(--csw-text);
        display: block;
        font-size: var(--csw-item-font);
        font-weight: var(--csw-label-weight, 600);
        line-height: 1.3;
        margin-bottom: 3px;
        overflow-wrap: anywhere;
      }

      .csw-row-prompt {
        color: var(--csw-muted);
        display: -webkit-box;
        font-size: max(10px, calc(var(--csw-item-font) - 1px));
        line-height: 1.46;
        max-height: 2.92em;
        overflow: hidden;
        overflow-wrap: anywhere;
        white-space: normal;
        -webkit-box-orient: vertical;
        -webkit-line-clamp: 2;
      }

      .csw-list[data-label-only="true"] .csw-row-prompt {
        display: none;
      }

      .csw-list[data-label-only="true"] .csw-row {
        align-items: center;
        min-height: 44px;
        padding-block: 10px;
      }

      .csw-list[data-label-only="true"] .csw-row-label {
        margin-bottom: 0;
      }

      .csw-list[data-label-only="true"] .csw-row-arrow {
        align-self: center;
      }

      .csw-row-arrow {
        color: var(--csw-faint);
        font-size: 17px;
        line-height: 1;
        text-align: center;
        transition: color 160ms ease, transform 160ms ease;
      }

      .csw-row:hover .csw-row-arrow,
      .csw-row:focus-visible .csw-row-arrow,
      .csw-row[data-preview-active="true"] .csw-row-arrow {
        color: var(--csw-accent);
        transform: translateX(2px);
      }

      .csw-prompt-preview {
        --csw-prompt-edge-fade-size: clamp(36px, 16%, 64px);
        background: transparent;
        border: 0;
        border-radius: 20px;
        box-shadow: none;
        isolation: isolate;
        min-height: 0;
        overflow: hidden;
        position: relative;
      }

      .csw-prompt-preview::before {
        -webkit-mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 calc(100% - var(--csw-prompt-edge-fade-size)),
          transparent 100%
        );
        -webkit-mask-repeat: no-repeat;
        background:
          linear-gradient(180deg, color-mix(in srgb, var(--csw-text) 4.5%, transparent), transparent),
          color-mix(in srgb, var(--csw-surface-opaque) 22%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 7%, transparent);
        border-radius: inherit;
        box-sizing: border-box;
        content: "";
        inset: 0;
        mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 calc(100% - var(--csw-prompt-edge-fade-size)),
          transparent 100%
        );
        mask-repeat: no-repeat;
        pointer-events: none;
        position: absolute;
        z-index: 0;
      }

      .csw-prompt-preview-scroll {
        -webkit-mask-image: none;
        height: 100%;
        mask-image: none;
        overflow: auto;
        overscroll-behavior: contain;
        padding: 16px 18px 28px;
        position: relative;
        scrollbar-color: color-mix(in srgb, var(--csw-text) 20%, transparent) transparent;
        scrollbar-width: thin;
        z-index: 1;
      }

      .csw-prompt-preview[data-scroll-fade="true"] .csw-prompt-preview-scroll {
        -webkit-mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - 18px)),
          transparent 100%
        );
        mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - 18px)),
          transparent 100%
        );
        -webkit-mask-repeat: no-repeat;
        mask-repeat: no-repeat;
      }

      .csw-prompt-preview-content {
        opacity: 1;
        transform: translateY(0);
        transition:
          opacity 120ms ease-out,
          transform 150ms cubic-bezier(.22, .8, .2, 1);
      }

      .csw-prompt-preview[data-switching="true"] .csw-prompt-preview-content {
        opacity: 0;
        transform: translateY(4px);
      }

      .csw-prompt-preview-kicker {
        color: var(--csw-accent);
        display: block;
        font-size: max(9px, calc(var(--csw-item-font) - 3px));
        font-weight: var(--csw-label-weight, 600);
        letter-spacing: .045em;
        line-height: 1.2;
        margin-bottom: 7px;
      }

      .csw-prompt-preview-title {
        color: var(--csw-text);
        display: block;
        font-size: clamp(12px, calc(var(--csw-item-font) + 1px), 25px);
        font-weight: var(--csw-label-weight, 600);
        letter-spacing: -.012em;
        line-height: 1.35;
        margin-bottom: 9px;
      }

      .csw-prompt-preview-body {
        color: var(--csw-muted);
        display: block;
        font-size: var(--csw-item-font);
        line-height: 1.65;
        overflow-wrap: anywhere;
        white-space: pre-wrap;
      }

      .csw-empty {
        align-items: center;
        background: transparent;
        border: 0;
        border-top: 0;
        border-radius: 0;
        color: var(--csw-muted);
        display: grid;
        flex: 1 1 auto;
        align-content: center;
        justify-items: center;
        min-height: 0;
        min-width: 0;
        max-width: 100%;
        padding: 24px 12px;
        text-align: center;
      }

      .csw-empty-title {
        color: var(--csw-text);
        font-size: clamp(12px, calc(var(--csw-item-font) + 1px), 25px);
        font-weight: 720;
        line-height: 1.25;
        max-width: 100%;
        min-width: 0;
        overflow-wrap: anywhere;
      }

      .csw-empty[data-state="manual"] .csw-empty-title {
        color: var(--csw-muted);
        font-size: clamp(11px, var(--csw-item-font), 20px);
        font-weight: 500;
        letter-spacing: 0.01em;
      }

      .csw-progress {
        align-items: center;
        color: var(--csw-muted);
        display: flex;
        flex: 1 1 auto;
        gap: clamp(10px, calc(var(--csw-item-font) - 1px), 18px);
        justify-content: center;
        min-height: 0;
        min-width: 0;
        max-width: 100%;
        padding: 16px 5px;
      }

      .csw-progress-ring {
        animation: csw-progress-spin .82s linear infinite;
        border: 2px solid color-mix(in srgb, var(--csw-text) 11%, transparent);
        border-radius: 999px;
        border-top-color: var(--csw-accent);
        flex: 0 0 auto;
        height: clamp(18px, calc(var(--csw-item-font) + 7px), 31px);
        width: clamp(18px, calc(var(--csw-item-font) + 7px), 31px);
      }

      .csw-progress-copy {
        display: grid;
        gap: 2px;
        min-width: 0;
        max-width: 100%;
      }

      .csw-progress-title {
        animation: csw-progress-text-shimmer 1.8s linear infinite;
        background-image: linear-gradient(
          90deg,
          color-mix(in srgb, var(--csw-text) 62%, var(--csw-muted)) 34%,
          var(--csw-text) 50%,
          color-mix(in srgb, var(--csw-text) 62%, var(--csw-muted)) 66%
        );
        background-position: 100% 50%;
        background-size: 220% 100%;
        -webkit-background-clip: text;
        background-clip: text;
        color: transparent;
        font-size: clamp(11px, var(--csw-item-font), 24px);
        font-weight: var(--csw-label-weight, 600);
        line-height: 1.25;
        overflow-wrap: anywhere;
        -webkit-text-fill-color: transparent;
      }

      .csw-outline-list {
        display: grid;
        align-content: start;
        flex: 0 0 auto;
        grid-auto-rows: max-content;
        min-height: max-content;
        width: 100%;
      }

      .csw-outline-view {
        display: flex;
        flex: 1 1 auto;
        flex-direction: column;
        min-height: 100%;
      }

      .csw-outline-toolbar {
        align-items: center;
        display: flex;
        flex: 0 0 auto;
        gap: 4px;
        justify-content: flex-end;
        margin-top: auto;
        opacity: 0;
        padding: 6px 0 0;
        pointer-events: none;
        position: sticky;
        bottom: 0;
        transform: translateY(4px);
        transition: opacity 150ms ease-out, transform 180ms ease-out;
        z-index: 2;
      }

      .csw-popover[data-open="true"]:hover .csw-outline-toolbar,
      .csw-popover[data-open="true"]:focus-within .csw-outline-toolbar {
        opacity: 1;
        pointer-events: auto;
        transform: translateY(0);
      }

      .csw-outline-nav-button {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 8px;
        color: var(--csw-muted);
        cursor: pointer;
        display: inline-flex;
        height: 26px;
        justify-content: center;
        padding: 0;
        transition: background 160ms ease-out, color 160ms ease-out, transform 90ms ease-out;
        width: 26px;
      }

      .csw-outline-nav-button svg {
        display: block;
        height: 13px;
        width: 13px;
      }

      .csw-outline-nav-button:hover,
      .csw-outline-nav-button:focus-visible {
        background: color-mix(in srgb, var(--csw-text) 8%, transparent);
        color: var(--csw-text);
        outline: none;
      }

      .csw-outline-nav-button:active {
        transform: scale(.94);
      }

      .csw-outline-row {
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 13px;
        box-sizing: border-box;
        color: var(--csw-text);
        cursor: pointer;
        display: block;
        isolation: isolate;
        min-height: 38px;
        padding: 8px 12px 8px calc(12px + var(--csw-outline-indent, 0px));
        position: relative;
        text-align: left;
        transition: color 140ms ease-out, transform 90ms ease-out;
        width: 100%;
      }

      .csw-outline-row::before {
        background: var(--csw-row-surface);
        border-radius: inherit;
        content: "";
        inset: 0;
        opacity: 0;
        pointer-events: none;
        position: absolute;
        transition: background 180ms ease-out, opacity 160ms ease-out;
        z-index: -1;
      }

      .csw-outline-row:first-child {
        border-top: 0;
      }

      .csw-outline-row:hover,
      .csw-outline-row:focus-visible {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-text) 12%, transparent);
        outline-offset: -1px;
      }

      .csw-outline-row:hover::before,
      .csw-outline-row:focus-visible::before {
        opacity: 1;
      }

      .csw-outline-row[data-active="true"] {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-accent) 34%, transparent);
        outline-offset: -1px;
      }

      .csw-outline-row[data-active="true"]::before {
        background: var(--csw-row-selected);
        opacity: 1;
      }

      .csw-outline-row:active {
        transform: scale(.992);
      }

      .csw-outline-row[data-outline-id] {
        align-items: center;
        column-gap: 8px;
        display: grid;
        grid-template-columns: 16px max-content minmax(0, 1fr);
        padding-left: calc(12px + var(--csw-outline-indent, 0px));
      }

      .csw-outline-heading-marker {
        align-items: center;
        display: inline-flex;
        height: 16px;
        justify-content: center;
        position: relative;
        width: 16px;
        z-index: 1;
      }

      .csw-outline-heading-marker::before {
        background: var(--csw-accent);
        border-radius: 999px;
        content: "";
        height: 5px;
        opacity: 0;
        width: 5px;
      }

      .csw-outline-row[data-level="0"] .csw-outline-heading-marker::before {
        opacity: 1;
      }

      .csw-outline-prefix {
        align-self: start;
        font-variant-numeric: tabular-nums;
        min-width: 0;
        white-space: nowrap;
      }

      .csw-outline-label {
        min-width: 0;
        overflow-wrap: anywhere;
      }

      .csw-outline-row[data-numbered="false"][data-level="0"] .csw-outline-prefix {
        display: none;
      }

      .csw-outline-row[data-numbered="false"][data-level="0"] .csw-outline-label {
        grid-column: 2 / -1;
      }

      .csw-outline-text,
      .csw-outline-prefix,
      .csw-outline-label {
        font-size: var(--csw-item-font);
        line-height: 1.35;
        position: relative;
        z-index: 1;
      }

      .${HIGHLIGHT_CLASS} {
        outline: 2px solid color-mix(in srgb, var(--csw-accent) 70%, transparent) !important;
        outline-offset: 4px !important;
        border-radius: 6px !important;
        transition: outline-color 0.2s ease;
      }

      .csw-resize-handle {
        appearance: none;
        -webkit-appearance: none;
        background: none;
        border: 0;
        bottom: 0;
        box-shadow: none;
        color: transparent;
        cursor: nwse-resize;
        display: none;
        font-size: 0;
        height: 28px;
        line-height: 0;
        outline: 0;
        padding: 0;
        pointer-events: auto;
        position: absolute;
        touch-action: none;
        user-select: none;
        width: 28px;
        z-index: 5;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-resize-handle {
        display: block;
      }

      .csw-popover[data-view="settings"] .csw-resize-handle {
        display: none !important;
      }

      .csw-resize-handle[data-corner="bl"] {
        cursor: nesw-resize;
        left: 0;
      }

      .csw-resize-handle[data-corner="br"] {
        right: 0;
      }

      .csw-settings {
        display: grid;
        height: 100%;
        min-height: 0;
        padding-top: 4px;
      }

      .csw-settings-surface {
        -webkit-backdrop-filter: blur(18px) saturate(145%);
        backdrop-filter: blur(18px) saturate(145%);
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 2%, transparent) 0%,
            transparent 16%,
            transparent 82%,
            color-mix(in srgb, #fff 7%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 82%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 6%, transparent);
        border-radius: 22px;
        box-shadow: none;
        display: grid;
        font-size: var(--csw-chrome-font);
        grid-template-rows: minmax(0, 1fr) auto;
        min-height: 0;
        overflow: hidden;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-settings-surface {
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 8%, transparent) 0%,
            transparent 18%,
            transparent 82%,
            color-mix(in srgb, #fff 3%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 76%, transparent);
        border-color: color-mix(in srgb, #fff 6%, transparent);
        box-shadow: none;
      }

      .csw-popover[data-material="clear"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background: transparent;
        border-color: color-mix(in srgb, var(--csw-glass-edge-hi) 24%, var(--csw-glass-edge));
        box-shadow: none;
      }

      .csw-popover[data-material="liquid"] .csw-settings-surface,
      .csw-popover[data-material="crystal"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background: rgba(255, 255, 255, 0.2);
        border-color: rgba(255, 255, 255, 0.14);
        box-shadow: none;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="liquid"] .csw-settings-surface,
      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="crystal"] .csw-settings-surface {
        background: rgba(20, 24, 30, 0.28);
        border-color: rgba(255, 255, 255, 0.11);
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="clear"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background: transparent;
        border-color: color-mix(in srgb, rgba(205, 228, 255, 0.5) 20%, var(--csw-glass-edge));
        box-shadow: none;
      }

      .csw-popover[data-material="matte"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 1.5%, transparent) 0%,
            transparent 18%,
            transparent 82%,
            color-mix(in srgb, #fff 5%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 98%, transparent);
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="matte"] .csw-settings-surface {
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 8%, transparent) 0%,
            transparent 18%,
            transparent 82%,
            color-mix(in srgb, #fff 3%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 96%, #000 2%);
      }

      .csw-settings-hero {
        align-items: center;
        display: grid;
        gap: 18px;
        grid-template-columns: minmax(0, 1fr) minmax(230px, 238px);
        min-height: 0;
        padding: 18px 18px 14px;
      }

      .csw-model-pane {
        align-self: center;
        display: flex;
        flex-direction: column;
        justify-content: center;
        min-width: 0;
        padding: 8px 6px;
      }

      .csw-metric-label,
      .csw-control-label {
        color: var(--csw-muted);
        font-size: 11px;
        font-weight: var(--csw-label-weight, 500);
        letter-spacing: .015em;
      }

      .csw-metric-label {
        font-synthesis: none;
        font-weight: 400;
      }

      .csw-model-value {
        color: var(--csw-text);
        font-size: 34px;
        font-weight: 580;
        letter-spacing: -.035em;
        line-height: 1.08;
        margin-top: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .csw-settings-surface[data-loading="true"] .csw-model-value {
        color: var(--csw-muted);
        font-size: 24px;
        letter-spacing: -.02em;
      }

      .csw-runtime-line {
        align-items: center;
        color: var(--csw-muted);
        display: flex;
        font-size: 12px;
        gap: 7px;
        margin-top: 10px;
        min-width: 0;
      }

      .csw-runtime-copy {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .csw-runtime-dot {
        background: var(--csw-faint);
        border-radius: 999px;
        flex: 0 0 auto;
        height: 7px;
        width: 7px;
      }

      .csw-runtime-dot[data-tone="busy"] {
        animation: csw-status-breathe 1.2s ease-in-out infinite;
        background: var(--csw-accent);
      }

      .csw-runtime-dot[data-tone="ready"] {
        background: var(--csw-ready);
      }

      .csw-runtime-dot[data-tone="error"] {
        background: var(--csw-danger);
      }

      .csw-runtime-grid {
        align-items: center;
        display: grid;
        gap: 14px;
        grid-template-columns: minmax(0, max-content) minmax(0, 1fr);
        min-width: 0;
        padding: 0;
        width: 100%;
      }

      .csw-click-mode,
      .csw-generation-mode {
        align-items: baseline;
        display: inline-flex;
        gap: 6px;
        max-width: 100%;
        min-width: 0;
        position: relative;
      }

      .csw-generation-mode {
        white-space: nowrap;
      }

      .csw-metric {
        align-items: baseline;
        display: inline-flex;
        gap: 6px;
        min-width: 0;
        padding: 0;
      }

      .csw-metric-value,
      .csw-metric-action {
        color: color-mix(in srgb, var(--csw-text) 76%, transparent);
        font-size: 11px;
        font-synthesis: none;
        font-variant-numeric: tabular-nums;
        font-weight: 400;
        letter-spacing: .005em;
        line-height: 1.25;
        min-width: 0;
        overflow: visible;
        text-overflow: clip;
      }

      .csw-metric-value,
      .csw-generation-mode .csw-metric-action {
        white-space: nowrap;
      }

      .csw-click-mode .csw-metric-action {
        overflow-wrap: anywhere;
        white-space: normal;
      }

      .csw-metric-value[data-enabled="true"],
      .csw-metric-action {
        color: var(--csw-text);
      }

      button.csw-metric-action {
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 0;
        box-shadow: none;
        color: var(--csw-text);
        cursor: pointer;
        font-family: inherit;
        font-size: 11px;
        font-weight: 400;
        margin: 0;
        max-width: 100%;
        min-width: 0;
        outline: none;
        padding: 0;
        text-align: left;
      }

      button.csw-metric-action:hover,
      button.csw-metric-action:focus-visible {
        background: transparent;
        color: color-mix(in srgb, var(--csw-text) 88%, var(--csw-accent) 12%);
      }

      button.csw-metric-action:active {
        color: color-mix(in srgb, var(--csw-text) 78%, var(--csw-accent) 22%);
      }

      button.csw-metric-action:disabled {
        cursor: not-allowed;
        opacity: .34;
      }

      .csw-control-deck {
        align-self: center;
        background: transparent;
        border: 0;
        border-radius: 0;
        box-shadow: none;
        display: grid;
        gap: 10px;
        grid-auto-rows: 32px;
        justify-self: end;
        min-width: 0;
        overflow: visible;
        padding: 0;
        width: 238px;
      }

      .csw-settings-footer {
        align-items: center;
        background: transparent;
        border: 0;
        border-radius: 0;
        border-top: 1px solid color-mix(in srgb, var(--csw-text) 8%, transparent);
        box-shadow: none;
        display: grid;
        gap: 12px;
        grid-template-columns: minmax(0, 1fr) auto;
        margin: 0 18px 12px;
        min-height: 50px;
        overflow: visible;
        padding: 9px 2px 0;
      }

      .csw-control-group {
        align-items: center;
        display: grid;
        gap: 10px;
        grid-template-columns: 76px minmax(0, 152px);
        min-width: 0;
        padding: 0;
      }

      .csw-control-group + .csw-control-group {
        border-top: 0;
      }

      .csw-control-label {
        align-self: center;
        font-size: 12px;
        line-height: 1;
        text-align: left;
      }

      .csw-control-row,
      .csw-stepper {
        align-items: center;
        box-sizing: border-box;
        justify-self: end;
        min-width: 0;
        width: 152px;
      }

      .csw-control-row {
        background: color-mix(in srgb, var(--csw-text) 2%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 7%, transparent);
        border-radius: 11px;
        box-shadow: none;
        display: grid;
        gap: 0;
        grid-template-columns: minmax(0, 1fr);
        height: 32px;
        overflow: hidden;
      }

      .csw-control-button,
      .csw-step-button,
      .csw-command-button {
        appearance: none;
        background: transparent;
        border: 0;
        color: var(--csw-text);
        cursor: pointer;
        font: inherit;
      }

      .csw-control-button {
        align-items: center;
        border-radius: 0;
        display: flex;
        font-size: 13px;
        font-weight: var(--csw-label-weight, 500);
        gap: 5px;
        height: 30px;
        justify-content: center;
        line-height: 30px;
        max-width: none;
        overflow: hidden;
        padding: 0 8px;
        text-overflow: ellipsis;
        white-space: nowrap;
        width: 100%;
      }

      .csw-stepper {
        display: grid;
        background: color-mix(in srgb, var(--csw-text) 2%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 7%, transparent);
        border-radius: 11px;
        box-shadow: none;
        grid-template-columns: 30px minmax(0, 1fr) 30px;
        height: 32px;
        overflow: hidden;
      }

      .csw-step-button {
        align-items: center;
        border-radius: 0;
        color: var(--csw-muted);
        display: flex;
        font-size: 18px;
        height: 30px;
        justify-content: center;
        line-height: 1;
        min-width: 30px;
        padding: 0;
      }

      .csw-step-value {
        align-items: center;
        border-left: 1px solid var(--csw-divider);
        border-right: 1px solid var(--csw-divider);
        color: var(--csw-text);
        display: flex;
        font-size: 13px;
        font-variant-numeric: tabular-nums;
        font-weight: 620;
        justify-content: center;
        min-width: 0;
        text-align: center;
      }

      .csw-control-button:hover,
      .csw-step-button:hover,
      .csw-command-button:hover {
        background: var(--csw-hover);
      }

      .csw-command-deck {
        align-items: center;
        display: flex;
        flex: 0 0 auto;
        gap: 3px;
        justify-self: end;
        min-width: 0;
        padding-left: 0;
      }

      .csw-command-button {
        align-items: center;
        border-radius: 7px;
        color: var(--csw-muted);
        display: flex;
        gap: 5px;
        justify-content: center;
        height: 30px;
        margin: 0;
        min-width: 0;
        padding: 0 7px;
      }

      .csw-command-button:hover {
        color: var(--csw-text);
      }

      .csw-command-button:disabled,
      .csw-step-button:disabled {
        cursor: not-allowed;
        opacity: .32;
      }

      .csw-step-button:disabled:hover {
        background: transparent;
      }

      .csw-command-icon {
        align-items: center;
        display: flex;
        height: 17px;
        justify-content: center;
        width: 17px;
      }

      .csw-command-icon svg {
        height: 16px;
        width: 16px;
      }

      .csw-command-icon[data-busy="true"] svg {
        animation: csw-progress-spin .9s linear infinite;
      }

      .csw-command-label {
        font-size: 12px;
        line-height: 1;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .csw-settings-notice {
        align-items: center;
        color: var(--csw-muted);
        display: flex;
        font-size: 11px;
        grid-column: 1 / -1;
        line-height: 1.4;
        max-width: 100%;
        min-height: 22px;
        min-width: 0;
        overflow-wrap: anywhere;
        padding: 2px 2px 0;
        white-space: normal;
      }

      .csw-settings-notice[data-tone="warn"] {
        color: color-mix(in srgb, var(--csw-danger) 78%, var(--csw-muted));
      }

      .csw-icon:focus-visible,
      .csw-head-face:focus-visible,
      .csw-control-button:focus-visible,
      .csw-metric-action:focus-visible,
      .csw-step-button:focus-visible,
      .csw-command-button:focus-visible,
      .csw-row:focus-visible {
        outline: 2px solid color-mix(in srgb, var(--csw-accent) 72%, transparent);
        outline-offset: 2px;
      }

      @container csw-panel (max-width: 440px) {
        .csw-list {
          padding-inline: 0;
        }

        .csw-row {
          gap: 8px;
          grid-template-columns: minmax(0, 1fr) 16px;
          padding: 11px 8px;
        }

        .csw-row-arrow {
          font-size: 16px;
        }

        .csw-prompt-preview-scroll {
          padding: 14px 14px 26px;
        }

        .csw-settings-hero {
          align-content: start;
          gap: 10px;
          grid-template-columns: 1fr;
          grid-template-rows: auto auto;
          overflow-x: hidden;
          overflow-y: auto;
          overscroll-behavior: contain;
          padding: 12px 14px 10px;
          scrollbar-color: color-mix(in srgb, var(--csw-text) 16%, transparent) transparent;
          scrollbar-width: thin;
        }

        .csw-model-pane {
          align-self: start;
          min-height: 72px;
          padding: 3px 4px 1px;
        }

        .csw-model-value {
          font-size: 27px;
        }

        .csw-settings-surface[data-loading="true"] .csw-model-value {
          font-size: 20px;
        }

        .csw-runtime-line {
          font-size: 11px;
          margin-top: 6px;
        }

        .csw-control-deck {
          gap: 8px;
          justify-self: stretch;
          min-height: 0;
          width: 100%;
        }

        .csw-control-group {
          gap: 8px;
          grid-template-columns: minmax(76px, auto) minmax(0, 1fr);
        }

        .csw-control-label {
          font-size: 11px;
        }

        .csw-control-row,
        .csw-stepper {
          width: 100%;
        }

        .csw-settings-footer {
          column-gap: 8px;
          display: grid;
          grid-template-columns: minmax(max-content, 1fr) auto;
          min-height: 0;
          padding: 8px 0 0;
          row-gap: 6px;
        }

        .csw-runtime-grid {
          gap: 12px;
          grid-template-columns: minmax(0, max-content) minmax(0, 1fr);
          justify-content: flex-start;
          min-height: 30px;
          width: 100%;
        }

        .csw-command-deck {
          gap: 2px;
          justify-content: flex-end;
        }

        .csw-command-button {
          flex: 0 0 30px;
          height: 30px;
          padding: 0;
          width: 30px;
        }

        .csw-command-label {
          display: none;
        }

        .csw-settings-notice {
          grid-column: 1 / -1;
          min-width: 0;
          padding-top: 0;
        }
      }

      @container csw-panel (max-width: 404px) {
        .csw-settings-footer {
          margin: 0 14px 10px;
        }
      }

      @container csw-panel (max-width: 360px) {
        .csw-runtime-grid {
          gap: 6px;
          grid-template-columns: minmax(0, 1fr);
        }

        .csw-generation-mode,
        .csw-click-mode {
          width: 100%;
        }
      }

      @container csw-panel (max-width: 320px) {
        .csw-row {
          gap: 6px;
          grid-template-columns: minmax(0, 1fr) 14px;
          padding-inline: 6px;
        }

        .csw-prompt-preview {
          border-radius: 16px;
        }

        .csw-settings-footer {
          column-gap: 6px;
          margin: 0 12px 10px;
        }

        .csw-runtime-grid {
          gap: 8px;
        }

        .csw-metric {
          gap: 5px;
          white-space: nowrap;
        }

        .csw-command-deck {
          gap: 0;
        }

        .csw-command-button {
          flex: 0 0 28px;
          height: 28px;
          width: 28px;
        }
      }

      @keyframes csw-face-blink {
        0%, 45%, 49%, 100% { transform: scaleY(1); }
        47% { transform: scaleY(0.12); }
      }

      @keyframes csw-face-star {
        0%, 100% { transform: scale(.9) rotate(0deg); }
        50% { transform: scale(1.08) rotate(8deg); }
      }

      @keyframes csw-face-generate-bob {
        0%, 100% { transform: translate3d(0, 2px, 0) scaleY(.9); }
        50% { transform: translate3d(0, -4px, 0) scaleY(1.06); }
      }

      @keyframes csw-face-happy-lift {
        0%, 100% { transform: translate3d(0, 1px, 0) scale(.97); }
        50% { transform: translate3d(0, -1px, 0) scale(1.03); }
      }

      @keyframes csw-face-calm-breathe {
        0%, 100% { opacity: .74; transform: scaleX(.94); }
        50% { opacity: 1; transform: scaleX(1); }
      }

      @keyframes csw-face-error-breathe {
        0%, 100% { opacity: .78; transform: scale(.96); }
        50% { opacity: 1; transform: scale(1); }
      }

      @keyframes csw-status-breathe {
        0%, 100% { opacity: .45; transform: scale(.86); }
        50% { opacity: 1; transform: scale(1); }
      }

      @keyframes csw-progress-spin {
        to { transform: rotate(360deg); }
      }

      @keyframes csw-progress-text-shimmer {
        to { background-position: -100% 50%; }
      }

      @media (prefers-reduced-motion: reduce) {
        .csw-progress-title {
          animation: none !important;
          background: none;
          color: var(--csw-text);
          -webkit-text-fill-color: currentColor;
        }

        .csw-view-indicator,
        .csw-view-tabs .csw-icon,
        .csw-outline-toolbar {
          transition: none !important;
        }
      }

      @media (hover: none), (pointer: coarse) {
        .csw-outline-toolbar {
          opacity: 1;
          pointer-events: auto;
          transform: none;
        }
      }

      @media (max-width: 520px) {
        .csw-head {
          padding-left: 14px;
          padding-right: 14px;
        }

        .csw-body {
          padding-left: 13px;
          padding-right: 13px;
        }

      }

      .csw-body,
      .csw-prompt-preview-scroll,
      .csw-settings-hero {
        scrollbar-color: transparent transparent;
        scrollbar-gutter: auto;
        scrollbar-width: none;
      }

      .csw-body::-webkit-scrollbar,
      .csw-prompt-preview-scroll::-webkit-scrollbar,
      .csw-settings-hero::-webkit-scrollbar {
        display: none;
        height: 0;
        width: 0;
      }

      .csw-popover[data-material="clear"],
      .csw-popover[data-material="clear"] *,
      .csw-popover[data-material="clear"]::before,
      .csw-popover[data-material="clear"]::after,
      .csw-popover[data-material="clear"] *::before,
      .csw-popover[data-material="clear"] *::after {
        text-shadow: none !important;
      }

      .csw-popover[data-material="matte"] .csw-prompt-preview::before {
        background:
          linear-gradient(180deg, color-mix(in srgb, var(--csw-text) 2.5%, transparent), transparent),
          color-mix(in srgb, var(--csw-surface-opaque) 14%, transparent);
        border-color: color-mix(in srgb, var(--csw-text) 4.5%, transparent);
      }

      .csw-popover[data-material] .csw-settings-surface {
        -webkit-backdrop-filter: none !important;
        backdrop-filter: none !important;
        background: transparent !important;
        border: 0 !important;
        border-radius: 0 !important;
        box-shadow: none !important;
      }

      @media (prefers-reduced-motion: reduce) {
        .csw-completion-beam,
        .csw-completion-beam::before {
          animation: none !important;
          opacity: 0 !important;
        }

        :is(.csw-fab, .csw-head-face) .csw-fab-eye,
        :is(.csw-fab, .csw-head-face) .csw-fab-eye::before,
        :is(.csw-fab, .csw-head-face) .csw-fab-eye::after {
          animation: none !important;
        }

        [${ROOT_ATTR}="true"] *,
        [${ROOT_ATTR}="true"] *::before,
        [${ROOT_ATTR}="true"] *::after {
          animation-duration: 1ms !important;
          animation-iteration-count: 1 !important;
          transition-duration: 1ms !important;
        }
      }

      @keyframes csw-completion-beam-sweep {
        0% {
          opacity: 0;
          transform: rotate(-64deg);
        }
        12% {
          opacity: .34;
        }
        72% {
          opacity: .52;
        }
        100% {
          opacity: 0;
          transform: rotate(296deg);
        }
      }

    `;
    document.head.appendChild(style);
  }

  // Derived expressions turn backend, parser, and page states into one calm user-facing status.
  function expressionError() {
    const settings = state.settings;
    const configurationMissing = settings?.enabled === true
      && (!settings.baseUrlConfigured || !settings.model || !settings.apiKeyConfigured);
    return configurationMissing
      || state.bridgeStatus === "failed"
      || (state.bridgeStatus === "disabled" && Boolean(state.bridgeError))
      || state.scanStatus === "manual-refresh-no-assistant";
  }

  function stepwiseWaitingForManualRefresh(settings = state.settings) {
    return stepwiseEnabled(settings)
      && stepwiseGenerationMode(settings) === "manual"
      && state.bridgeStatus !== "pending"
      && state.bridgeStatus !== "ok"
      && !state.prompts.length
      && !expressionError();
  }

  function resolveStepwiseExpression(now = Date.now()) {
    if (!stepwiseEnabled()) return "hidden";
    if (state.bridgeStatus === "pending") return "generating";
    if (expressionError()) return "error";
    if (stepwiseGenerationMode() === "manual") {
      if (state.bridgeStatus === "disabled") return "hidden";
      if (state.prompts.length) return "ready";
      if (state.bridgeStatus === "ok") return "empty";
      return "idle";
    }
    if (state.scanBusy) return "answering";
    if (state.surpriseUntil > now) return "surprise";
    if (state.scanStatus === "assistant-changed" || state.scanStatus === "assistant-settling") {
      return "answering";
    }
    if (state.bridgeStatus === "disabled") return "hidden";
    if (state.prompts.length) return "ready";
    if (state.bridgeStatus === "ok") return "empty";
    return "idle";
  }

  function resolveOutlineExpression(now = Date.now()) {
    if (!outlineEnabled()) return "hidden";
    if (state.outlineStatus === "pending") return "generating";
    if (state.scanBusy) return "answering";
    if (state.surpriseUntil > now) return "surprise";
    if (state.outlineStatus === "error") return "error";
    if (state.outlineItems.length) return "ready";
    if (state.outlineStatus === "empty") return "empty";
    return "idle";
  }

  function usesOutlineExpression(now = Date.now()) {
    const stepwiseExpression = resolveStepwiseExpression(now);
    return outlineEnabled()
      && (state.activeTab === "outline"
        || stepwiseExpression === "hidden"
        || stepwiseWaitingForManualRefresh());
  }

  function resolveFabExpression(now = Date.now()) {
    if (!runtimeEnabled()) return "hidden";
    return usesOutlineExpression(now)
      ? resolveOutlineExpression(now)
      : resolveStepwiseExpression(now);
  }

  function fabExpressionLabel(expression, outlineExpression = usesOutlineExpression()) {
    if (outlineExpression) {
      return {
        idle: "空闲",
        answering: "回答中",
        surprise: "正在整理回答",
        generating: "正在整理大纲",
        ready: "大纲已准备",
        empty: "暂无大纲",
        error: "生成失败",
        curious: "查看设置",
        hidden: "已关闭",
      }[expression] || "空闲";
    }
    return {
      idle: "空闲",
      answering: "回答中",
      surprise: "正在整理回答",
      generating: "正在生成建议",
      ready: "建议已准备",
      empty: "暂无建议",
      error: "生成失败",
      curious: "查看设置",
      hidden: "已关闭",
    }[expression] || "空闲";
  }

  function scheduleExpressionRefresh(delay) {
    if (!isCurrentRuntime()) return;
    if (state.expressionTimer) window.clearTimeout(state.expressionTimer);
    const generation = state.runtimeGeneration;
    const timer = window.setTimeout(() => {
      if (state.expressionTimer === timer) state.expressionTimer = 0;
      if (isCurrentRuntime(generation)) renderFloat();
    }, delay);
    state.expressionTimer = timer;
  }

  function clearCompletionBeam() {
    if (state.completionBeamTimer) window.clearTimeout(state.completionBeamTimer);
    state.completionBeamTimer = 0;
    if (state.popover) state.popover.dataset.completionBeam = "false";
  }

  function triggerCompletionBeam(promptCount) {
    clearCompletionBeam();
    if (promptCount < 1 || prefersReducedMotion() || !state.popover) return;
    state.popover.dataset.completionBeam = "true";
    const timer = window.setTimeout(() => {
      if (state.completionBeamTimer !== timer) return;
      state.completionBeamTimer = 0;
      if (state.popover) state.popover.dataset.completionBeam = "false";
    }, COMPLETION_BEAM_MS);
    state.completionBeamTimer = timer;
  }

  // View transitions and shell morphs share deterministic completion and cancellation rules.
  function prefersReducedMotion() {
    try {
      return window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches === true;
    } catch {
      return false;
    }
  }

  function cancelViewAnimation() {
    cancelViewStageAnimation();
    cancelViewIndicatorAnimation();
  }

  function cancelViewStageAnimation() {
    const transition = state.viewAnimation;
    state.viewAnimation = null;
    if (!transition) return;
    transition.animations?.forEach((animation) => animation.cancel());
    transition.finish?.();
  }

  function cancelViewIndicatorAnimation() {
    if (state.viewIndicatorFrame) window.cancelAnimationFrame(state.viewIndicatorFrame);
    state.viewIndicatorFrame = 0;
  }

  function deferRender() {
    state.pendingRender = true;
  }

  function flushDeferredRender() {
    if (!state.pendingRender || !isCurrentRuntime()) return false;
    if (state.viewTransitioning || state.morphAnimation) return false;
    state.pendingRender = false;
    renderFloat({ preserveMorph: true, allowDuringTransition: true });
    return true;
  }

  function viewSlideDirection(fromTab, targetTab) {
    const fromIndex = VIEW_ORDER.indexOf(fromTab);
    const targetIndex = VIEW_ORDER.indexOf(targetTab);
    if (fromIndex < 0 || targetIndex < 0 || fromIndex === targetIndex) return 1;
    return targetIndex > fromIndex ? 1 : -1;
  }

  function captureViewStage() {
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    const stage = body?.querySelector(":scope > .csw-mouth-stage");
    if (!body || !stage) return null;
    return {
      node: stage.cloneNode(true),
      scrollTop: body.scrollTop,
    };
  }

  function animateViewSlide(snapshot, direction) {
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    const incoming = body?.querySelector(":scope > .csw-mouth-stage");
    if (!snapshot?.node || !body || !incoming || prefersReducedMotion()
      || typeof incoming.animate !== "function") {
      return Promise.resolve();
    }

    cancelViewStageAnimation();
    const layer = document.createElement("div");
    const outgoing = snapshot.node;
    layer.className = "csw-view-transition-layer";
    outgoing.classList.add("csw-view-transition-copy");
    outgoing.style.top = `${2 - snapshot.scrollTop}px`;
    layer.appendChild(outgoing);
    body.appendChild(layer);
    body.dataset.viewTransition = "true";

    const distance = VIEW_SLIDE_DISTANCE * direction;
    const options = {
      duration: VIEW_SLIDE_MS,
      easing: "cubic-bezier(.2, .72, .2, 1)",
      fill: "forwards",
    };
    const outgoingAnimation = outgoing.animate([
      { opacity: 1, transform: "translate3d(0, 0, 0)" },
      { opacity: 0.08, transform: `translate3d(${-distance}px, 0, 0)` },
    ], options);
    const incomingAnimation = incoming.animate([
      { opacity: 0.42, transform: `translate3d(${distance}px, 0, 0)` },
      { opacity: 1, transform: "translate3d(0, 0, 0)" },
    ], options);
    let cleaned = false;
    const cleanup = () => {
      if (cleaned) return;
      cleaned = true;
      body.removeAttribute("data-view-transition");
      layer.remove();
    };
    let resolveCompletion;
    let settled = false;
    const completion = new Promise((resolve) => {
      resolveCompletion = resolve;
    });
    const transition = {
      animations: [outgoingAnimation, incomingAnimation],
      cleanup,
      fallbackTimer: 0,
      finish: () => {
        if (settled) return;
        settled = true;
        if (transition.fallbackTimer) window.clearTimeout(transition.fallbackTimer);
        cleanup();
        if (state.viewAnimation === transition) state.viewAnimation = null;
        resolveCompletion();
      },
    };
    state.viewAnimation = transition;
    transition.fallbackTimer = window.setTimeout(
      () => {
        transition.animations.forEach((animation) => {
          if (animation.playState !== "finished") animation.cancel();
        });
        transition.finish();
      },
      VIEW_SLIDE_MS + 120,
    );
    void Promise.all(transition.animations.map((animation) => animation.finished.catch(() => null)))
      .then(() => transition.finish());
    return completion;
  }

  function syncViewTabSelection(targetTab, animate = true) {
    const tabs = state.panel?.querySelector(".csw-view-tabs");
    const indicator = tabs?.querySelector(".csw-view-indicator");
    if (!tabs || !indicator) return;

    const buttons = Array.from(tabs.querySelectorAll(".csw-icon[data-view]"));
    const target = buttons.find((button) => button.dataset.view === targetTab) || null;
    buttons.forEach((button) => {
      const selected = button === target;
      button.dataset.active = String(selected);
      button.setAttribute("aria-selected", String(selected));
    });

    indicator.style.transition = animate && !prefersReducedMotion() ? "" : "none";
    if (!target) {
      indicator.style.opacity = "0";
      tabs.dataset.activeView = "";
      return;
    }

    tabs.dataset.activeView = targetTab;
    indicator.style.opacity = "1";
    indicator.style.transform = `translate3d(${target.offsetLeft - indicator.offsetLeft}px, 0, 0)`;
    if (!animate) indicator.getBoundingClientRect();
  }

  function animateViewTabSelection(fromTab, targetTab) {
    syncViewTabSelection(fromTab, false);
    if (fromTab === targetTab) return;
    state.viewIndicatorFrame = window.requestAnimationFrame(() => {
      state.viewIndicatorFrame = 0;
      if (!isCurrentRuntime()) return;
      syncViewTabSelection(targetTab, true);
    });
  }

  async function switchView(nextTab) {
    const generation = state.runtimeGeneration;
    const targetTab = normalizeActiveTab(nextTab);
    if (!isCurrentRuntime(generation) || targetTab === state.activeTab) return;
    if (state.viewTransitioning) {
      state.pendingTab = targetTab;
      return;
    }
    state.viewTransitioning = true;
    try {
      const sourceTab = state.activeTab;
      const snapshot = captureViewStage();
      const direction = viewSlideDirection(sourceTab, targetTab);
      state.activeTab = normalizeActiveTab(targetTab);
      state.pendingRender = false;
      renderFloat({
        preserveMorph: true,
        viewIndicatorFrom: sourceTab,
        allowDuringTransition: true,
      });
      await animateViewSlide(snapshot, direction);
      if (!isCurrentRuntime(generation)) return;
      if (targetTab === "settings") void reloadSettings();
    } finally {
      if (isCurrentRuntime(generation)) {
        state.viewTransitioning = false;
        const pendingTab = state.pendingTab;
        state.pendingTab = "";
        if (pendingTab && pendingTab !== state.activeTab) void switchView(pendingTab);
        else flushDeferredRender();
      }
    }
  }

  // Shell geometry is sampled from the capsule through the horizontal intermediate to the panel.
  function lerp(from, to, progress) {
    return from + (to - from) * progress;
  }

  function axisEase(progress) {
    const value = clamp(progress, 0, 1);
    const eased = 1 - Math.pow(1 - value, 1.25);
    return eased * 0.4 + value * 0.6;
  }

  function expandMotionU(progress) {
    return clamp(progress, 0, 1);
  }

  function defaultPosition() {
    const bounds = contentSafeBounds();
    return clampPosition({
      x: bounds.right - CHIP_WIDTH,
      y: Math.min(bounds.bottom - CHIP_HEIGHT, bounds.top + 44),
    }, false);
  }

  function savedPosition() {
    try {
      const parsed = JSON.parse(localStorage.getItem(POSITION_KEY) || "null");
      if (Number.isFinite(parsed?.x) && Number.isFinite(parsed?.y)) return clampPosition(parsed);
    } catch {}
    return defaultPosition();
  }

  function contentSafeBounds() {
    const viewportWidth = Math.max(80, window.innerWidth || 0);
    const viewportHeight = Math.max(80, window.innerHeight || 0);
    let left = PANEL_SAFE_MARGIN;
    let top = PANEL_SAFE_MARGIN;
    let right = viewportWidth - PANEL_SAFE_MARGIN;
    let bottom = viewportHeight - PANEL_SAFE_MARGIN;

    const leftPanel = document.querySelector("aside.app-shell-left-panel");
    if (leftPanel instanceof Element) {
      const rect = leftPanel.getBoundingClientRect();
      if (rect.width >= 48 && rect.right > 40 && rect.right < viewportWidth * 0.62) {
        left = Math.max(left, rect.right + PANEL_SAFE_MARGIN);
      }
    }

    const mainStage = document.querySelector(
      "main.main-surface, .app-shell-main-content-viewport, .app-shell-main-content-frame"
    );
    if (mainStage instanceof Element) {
      const rect = mainStage.getBoundingClientRect();
      if (rect.width >= 160) {
        if (rect.left > 40 && rect.left < viewportWidth * 0.62) {
          left = Math.max(left, rect.left + PANEL_SAFE_MARGIN);
        }
        if (rect.right > left + 80 && rect.right <= viewportWidth + 2) {
          right = Math.min(right, rect.right - PANEL_SAFE_MARGIN);
        }
        if (rect.top >= 0 && rect.top < viewportHeight * 0.4) {
          top = Math.max(top, rect.top + PANEL_SAFE_MARGIN);
        }
        if (rect.bottom > top + 80 && rect.bottom <= viewportHeight + 2) {
          bottom = Math.min(bottom, rect.bottom - PANEL_SAFE_MARGIN);
        }
      }
    }

    const rightRail = document.querySelector(
      "aside.app-shell-right-panel, [data-testid='right-sidebar'], aside.app-shell-secondary-panel"
    );
    if (rightRail instanceof Element) {
      const rect = rightRail.getBoundingClientRect();
      if (rect.width >= 48 && rect.left > viewportWidth * 0.45 && rect.left < viewportWidth - 40) {
        right = Math.min(right, rect.left - PANEL_SAFE_MARGIN);
      }
    }

    document.querySelectorAll(
      ".app-header-tint, .draggable.flex.h-toolbar, [class*='h-toolbar'].draggable, header"
    ).forEach((bar) => {
      if (!(bar instanceof Element)) return;
      const rect = bar.getBoundingClientRect();
      if (rect.height < 28 || rect.height > 96) return;
      if (rect.top > 24 || rect.width < viewportWidth * 0.45) return;
      top = Math.max(top, rect.bottom + PANEL_SAFE_MARGIN);
    });

    if (bottom - top < CHIP_HEIGHT) {
      top = PANEL_SAFE_MARGIN;
    }

    if (right - left < CHIP_WIDTH) {
      left = PANEL_SAFE_MARGIN;
      right = viewportWidth - PANEL_SAFE_MARGIN;
    }

    return {
      left,
      top,
      right,
      bottom,
      width: Math.max(0, right - left),
      height: Math.max(0, bottom - top),
    };
  }

  function clampPosition(position) {
    const bounds = contentSafeBounds();
    const visibleWidth = Math.min(CHIP_WIDTH, bounds.width);
    const visibleHeight = Math.min(CHIP_HEIGHT, bounds.height);
    const sourceX = Number(position?.x);
    const sourceY = Number(position?.y);
    return {
      x: clamp(Number.isFinite(sourceX) ? sourceX : bounds.left, bounds.left, Math.max(bounds.left, bounds.right - visibleWidth)),
      y: clamp(Number.isFinite(sourceY) ? sourceY : bounds.top, bounds.top, Math.max(bounds.top, bounds.bottom - visibleHeight)),
    };
  }

  function persistPosition() {
    if (!state.position) return;
    try { localStorage.setItem(POSITION_KEY, JSON.stringify(state.position)); } catch {}
  }

  function setPosition(position, persist = false) {
    state.position = clampPosition(position);
    if (persist) persistPosition();
    applyPosition();
  }

  function dockRightKeepHeight(persist = true) {
    const layout = shellLayout();
    setPosition({
      x: layout.bounds.right - layout.chip.width,
      y: layout.anchor.y,
    }, persist);
  }

  function snapRightIfNear(persist = false, animate = false) {
    const layout = shellLayout();
    const visibleRight = state.open
      ? layout.left + layout.width
      : layout.anchor.x + layout.chip.width;
    if (layout.bounds.right - visibleRight > RIGHT_EDGE_SNAP_DISTANCE) return false;
    if (animate && state.popover && !prefersReducedMotion()) {
      if (state.snapTimer) window.clearTimeout(state.snapTimer);
      state.popover.dataset.snapRight = "true";
      const timer = window.setTimeout(() => {
        if (state.snapTimer !== timer) return;
        state.snapTimer = 0;
        state.popover?.removeAttribute("data-snap-right");
      }, 220);
      state.snapTimer = timer;
    }
    dockRightKeepHeight(persist);
    return true;
  }

  function shellLayout() {
    const bounds = contentSafeBounds();
    const width = Math.max(CHIP_WIDTH, Math.min(state.width, bounds.width));
    const anchor = clampPosition(state.position || defaultPosition());
    const chipWidth = Math.min(CHIP_WIDTH, width);
    const chipHeight = Math.min(CHIP_HEIGHT, bounds.height);
    const minimumPanelHeight = Math.min(PANEL_MIN_HEIGHT, bounds.height);
    const roomBelow = Math.max(chipHeight, bounds.bottom - anchor.y);
    const roomAbove = Math.max(chipHeight, anchor.y + chipHeight - bounds.top);
    const panelDrag = state.drag?.source === "panel" ? state.drag : null;
    const resizeDrag = state.resizeDrag;
    const opensDown = typeof panelDrag?.lockedOpensDown === "boolean"
      ? panelDrag.lockedOpensDown
      : typeof resizeDrag?.lockedOpensDown === "boolean"
        ? resizeDrag.lockedOpensDown
      : roomBelow >= minimumPanelHeight || roomBelow >= roomAbove;
    const availableHeight = opensDown ? roomBelow : roomAbove;
    const requestedHeight = Number.isFinite(panelDrag?.panelHeight)
      ? panelDrag.panelHeight
      : state.activeTab === "settings"
        ? clampPanelHeight(SETTINGS_PANEL_HEIGHT)
        : state.height;
    const height = Math.max(CHIP_HEIGHT, Math.min(requestedHeight, bounds.height, availableHeight));
    const compressionProgress = state.activeTab === "settings"
      ? 0
      : clamp(
        (requestedHeight - height) / Math.max(1, requestedHeight - chipHeight),
        0,
        1,
      );
    const desiredLeft = anchor.x - (width - chipWidth) / 2;
    const left = clamp(desiredLeft, bounds.left, Math.max(bounds.left, bounds.right - width));
    const desiredTop = opensDown ? anchor.y : anchor.y + chipHeight - height;
    const top = clamp(desiredTop, bounds.top, Math.max(bounds.top, bounds.bottom - height));
    const chipLeft = clamp(anchor.x - left, 0, Math.max(0, width - chipWidth));
    const chipTop = clamp(anchor.y - top, 0, Math.max(0, height - chipHeight));
    const collapsedShell = {
      left: chipLeft,
      top: chipTop,
      width: chipWidth,
      height: chipHeight,
      radius: CHIP_RADIUS,
    };
    const horizontalShell = {
      left: 0,
      top: chipTop,
      width,
      height: chipHeight,
      radius: CHIP_RADIUS,
    };
    const expandedShell = {
      left: 0,
      top: 0,
      width,
      height,
      radius: PANEL_RADIUS,
    };
    const distX = Math.max(1, expandedShell.width - collapsedShell.width);
    const distY = Math.max(1, expandedShell.height - collapsedShell.height);
    const stageMs = Math.max(
      Math.max(MIN_PHASE_MS, distX / MORPH_EDGE_SPEED),
      Math.max(MIN_PHASE_MS, distY / MORPH_EDGE_SPEED)
    );
    return {
      left,
      top,
      width,
      height,
      requestedHeight,
      availableHeight,
      compressionProgress,
      bounds,
      anchor,
      chip: {
        left: chipLeft,
        top: chipTop,
        width: chipWidth,
        height: chipHeight,
        radius: CHIP_RADIUS,
      },
      collapsedShell,
      horizontalShell,
      expandedShell,
      distX,
      distY,
      opensDown,
      phaseSplit: HORIZONTAL_PHASE,
      morphDurationMs: clamp(Math.round(stageMs * 2), MIN_MORPH_MS, MAX_MORPH_MS),
    };
  }

  function phaseSplitOf(geometry) {
    const split = Number(geometry?.phaseSplit);
    if (Number.isFinite(split) && split > 0.05 && split < 0.95) return split;
    return HORIZONTAL_PHASE;
  }

  // Canceling a morph invalidates its callbacks before stopping animations or clearing state.
  function cancelMorphAnimations() {
    const transition = state.morphTransition;
    if (transition) {
      transition.cancelled = true;
      if (transition.fallbackTimer) window.clearTimeout(transition.fallbackTimer);
    }
    state.morphTransition = null;
    state.morphGeneration += 1;
    const animations = [
      state.morphAnimation,
      state.rimMorphAnimation,
      state.displacementMorphAnimation,
      state.panelMorphAnimation,
      state.fabMorphAnimation,
      ...(transition?.animations || []),
    ];
    [...new Set(animations)].forEach((animation) => animation?.cancel?.());
    state.morphAnimation = null;
    state.rimMorphAnimation = null;
    state.displacementMorphAnimation = null;
    state.panelMorphAnimation = null;
    state.fabMorphAnimation = null;
  }

  function unfoldAxes(progress, collapsing = false, split = HORIZONTAL_PHASE) {
    const value = clamp(progress, 0, 1);
    const elapsed = collapsing ? 1 - value : value;
    const phase = clamp(split, 0.05, 0.95);
    let x;
    let y;
    if (elapsed <= phase) {
      x = axisEase(phase < 0.001 ? 1 : elapsed / phase);
      y = 0;
    } else {
      x = 1;
      y = axisEase((elapsed - phase) / Math.max(0.001, 1 - phase));
    }
    return collapsing ? { x: 1 - x, y: 1 - y } : { x, y };
  }

  function unfoldShell(geometry, progress, collapsing = false) {
    const { x, y } = unfoldAxes(progress, collapsing, phaseSplitOf(geometry));
    const collapsed = geometry.collapsedShell;
    const expanded = geometry.expandedShell;
    return {
      left: lerp(collapsed.left, expanded.left, x),
      top: lerp(collapsed.top, expanded.top, y),
      width: lerp(collapsed.width, expanded.width, x),
      height: lerp(collapsed.height, expanded.height, y),
      radius: lerp(collapsed.radius, expanded.radius, Math.max(x, y)),
    };
  }

  function morphPathProgress(shell, geometry) {
    const split = phaseSplitOf(geometry);
    const collapsed = geometry.collapsedShell;
    const expanded = geometry.expandedShell;
    const widthProgress = clamp(
      (shell.width - collapsed.width) / Math.max(1, expanded.width - collapsed.width),
      0,
      1
    );
    const heightProgress = clamp(
      (shell.height - collapsed.height) / Math.max(1, expanded.height - collapsed.height),
      0,
      1
    );
    if (heightProgress > 0.002 || widthProgress >= 0.998) {
      return split + heightProgress * (1 - split);
    }
    return widthProgress * split;
  }

  function readGlassGeometry(geometry) {
    const fallback = unfoldShell(geometry, state.open ? 1 : 0);
    if (!state.glass) return fallback;
    const computed = getComputedStyle(state.glass);
    const number = (value, fallbackValue) => {
      const parsed = Number.parseFloat(String(value || ""));
      return Number.isFinite(parsed) ? parsed : fallbackValue;
    };
    return {
      left: number(computed.left, fallback.left),
      top: number(computed.top, fallback.top),
      width: Math.max(1, number(computed.width, fallback.width)),
      height: Math.max(1, number(computed.height, fallback.height)),
      radius: Math.max(0, number(computed.borderTopLeftRadius, fallback.radius)),
    };
  }

  function morphPx(value) {
    return `${Number(value.toFixed(3))}px`;
  }

  function glassFrame(shell, offset) {
    return {
      left: morphPx(shell.left),
      top: morphPx(shell.top),
      width: morphPx(shell.width),
      height: morphPx(shell.height),
      borderRadius: morphPx(shell.radius),
      offset: Number(offset.toFixed(4)),
    };
  }

  function panelClipPath(shell, geometry) {
    const top = Math.max(0, shell.top);
    const right = Math.max(0, geometry.width - shell.left - shell.width);
    const bottom = Math.max(0, geometry.height - shell.top - shell.height);
    const left = Math.max(0, shell.left);
    return `inset(${morphPx(top)} ${morphPx(right)} ${morphPx(bottom)} ${morphPx(left)} round ${morphPx(shell.radius)})`;
  }

  function panelFrame(shell, geometry, offset) {
    return {
      clipPath: panelClipPath(shell, geometry),
      offset: Number(offset.toFixed(4)),
    };
  }

  function fabFrame(shell, offset) {
    const headerHeight = Math.min(CHIP_HEIGHT + 8, shell.height);
    return {
      left: morphPx(shell.left + (shell.width - CHIP_WIDTH) / 2),
      top: morphPx(shell.top + Math.max(0, (headerHeight - CHIP_HEIGHT) / 2)),
      offset: Number(offset.toFixed(4)),
    };
  }

  function buildMorphPath(currentShell, expanded, geometry) {
    const startProgress = morphPathProgress(currentShell, geometry);
    const targetProgress = expanded ? 1 : 0;
    const remaining = Math.abs(targetProgress - startProgress);
    const baseDuration = clamp(
      Number(geometry.morphDurationMs) || MIN_MORPH_MS,
      MIN_MORPH_MS,
      MAX_MORPH_MS
    );
    const duration = remaining < 0.002
      ? 0
      : clamp(Math.round(baseDuration * remaining), MIN_REVERSE_MS, MAX_MORPH_MS);
    const samples = [{ shell: currentShell, offset: 0 }];
    const steps = UNFOLD_SAMPLES + 1;
    const progressDelta = targetProgress - startProgress;
    const stageProgress = phaseSplitOf(geometry);
    const stageTimeline = Math.abs(progressDelta) < 0.000001
      ? -1
      : (stageProgress - startProgress) / progressDelta;
    const timelines = [];
    for (let index = 1; index <= steps; index += 1) {
      timelines.push(index / steps);
    }
    if (stageTimeline > 0.000001 && stageTimeline < 0.999999) {
      timelines.push(stageTimeline);
    }
    timelines.sort((left, right) => left - right);
    let previousTimeline = -1;
    for (const timeline of timelines) {
      if (Math.abs(timeline - previousTimeline) < 0.000001) continue;
      const motion = expanded ? expandMotionU(timeline) : timeline;
      const sampledProgress = startProgress + progressDelta * motion;
      const progress = Math.abs(timeline - stageTimeline) < 0.000001
        ? stageProgress
        : sampledProgress;
      samples.push({ shell: unfoldShell(geometry, progress, false), offset: timeline });
      previousTimeline = timeline;
    }
    const targetShell = expanded ? geometry.expandedShell : geometry.collapsedShell;
    samples[samples.length - 1] = { shell: targetShell, offset: 1 };
    return {
      duration,
      frames: samples.map(({ shell, offset }) => glassFrame(shell, offset)),
      panelFrames: samples.map(({ shell, offset }) => panelFrame(shell, geometry, offset)),
      fabFrames: samples.map(({ shell, offset }) => fabFrame(shell, offset)),
      startProgress,
      targetProgress,
      targetShell,
    };
  }

  function applyMorphShell(shell, geometry) {
    [state.glass, state.rim, state.displacementTexture, state.completionBeam].forEach((surface) => {
      if (!surface) return;
      surface.style.left = `${shell.left}px`;
      surface.style.top = `${shell.top}px`;
      surface.style.width = `${shell.width}px`;
      surface.style.height = `${shell.height}px`;
      surface.style.borderRadius = `${shell.radius}px`;
    });
    if (state.panel) {
      state.panel.style.clipPath = panelClipPath(shell, geometry);
    }
    if (state.fab) {
      const frame = fabFrame(shell, 0);
      state.fab.style.left = frame.left;
      state.fab.style.top = frame.top;
    }
  }

  function applyMorphProgress(progress) {
    if (!state.glass && !state.rim && !state.panel && !state.fab) return;
    const geometry = state.layout || shellLayout();
    const shell = unfoldShell(geometry, progress, false);
    applyMorphShell(shell, geometry);
  }

  function settleMorph(progress, focusTarget = "") {
    if (!isCurrentRuntime()) return;
    cancelMorphAnimations();
    resetEyePointer();
    const expanded = progress >= 0.999;
    state.open = expanded;
    state.popover.dataset.open = String(expanded);
    state.popover.dataset.morphing = "false";
    state.panel.inert = !expanded;
    state.panel.setAttribute("aria-hidden", String(!expanded));
    state.fab.setAttribute("aria-expanded", String(expanded));
    applyMorphProgress(expanded ? 1 : 0);
    resetGlassPointer();
    const runtimeGeneration = state.runtimeGeneration;
    if (focusTarget === "panel" && expanded) {
      window.requestAnimationFrame(() => {
        if (isCurrentRuntime(runtimeGeneration)) {
          state.panel?.querySelector("[data-action='collapse']")?.focus({ preventScroll: true });
        }
      });
    }
    if (focusTarget === "chip" && !expanded) {
      window.requestAnimationFrame(() => {
        if (isCurrentRuntime(runtimeGeneration)) state.fab?.focus({ preventScroll: true });
      });
    }
    if (!flushDeferredRender()) syncEyeTracking();
  }

  function startMorph(expanded, focusTarget = "") {
    if (!state.glass || !state.rim || !state.fab || !state.panel || !state.popover) return;
    resetEyePointer();
    const geometry = state.layout || shellLayout();
    const currentShell = readGlassGeometry(geometry);
    cancelMorphAnimations();
    state.open = expanded;
    state.focusAfterMorph = focusTarget;
    state.popover.dataset.open = String(expanded);
    state.popover.dataset.morphing = "true";
    resetGlassPointer();
    state.panel.inert = true;
    state.panel.setAttribute("aria-hidden", "true");
    state.fab.setAttribute("aria-expanded", String(expanded));
    const path = buildMorphPath(currentShell, expanded, geometry);
    applyMorphShell(currentShell, geometry);

    if (prefersReducedMotion() || path.duration === 0) {
      settleMorph(path.targetProgress, focusTarget);
      return;
    }

    const generation = state.morphGeneration;
    const runtimeGeneration = state.runtimeGeneration;
    const timing = {
      duration: path.duration,
      easing: "cubic-bezier(.2, .72, .2, 1)",
      fill: "forwards",
    };
    const animation = state.glass.animate(path.frames, timing);
    state.rimMorphAnimation = state.rim.animate(path.frames, timing);
    state.displacementMorphAnimation = state.displacementTexture?.animate(path.frames, timing) || null;
    state.panelMorphAnimation = state.panel.animate(path.panelFrames, timing);
    state.fabMorphAnimation = state.fab.animate(path.fabFrames, timing);
    state.morphAnimation = animation;
    const animations = [
      animation,
      state.rimMorphAnimation,
      state.displacementMorphAnimation,
      state.panelMorphAnimation,
      state.fabMorphAnimation,
    ].filter(Boolean);
    let settled = false;
    const transition = {
      animations,
      cancelled: false,
      fallbackTimer: 0,
      finish: () => {
        if (transition.cancelled || settled) return;
        settled = true;
        if (transition.fallbackTimer) window.clearTimeout(transition.fallbackTimer);
        if (state.morphTransition === transition) state.morphTransition = null;
        if (!isCurrentRuntime(runtimeGeneration) || generation !== state.morphGeneration) return;
        settleMorph(path.targetProgress, focusTarget);
      },
    };
    state.morphTransition = transition;
    transition.fallbackTimer = window.setTimeout(() => {
      transition.animations.forEach((item) => {
        if (item.playState !== "finished") item.cancel();
      });
      transition.finish();
    }, path.duration + MORPH_FALLBACK_BUFFER_MS);
    void Promise.all(transition.animations.map((item) => item.finished.catch(() => null)))
      .then(() => transition.finish());
  }

  function setOpen(expanded, focusTarget = "") {
    if (!isCurrentRuntime()) return;
    resetEyePointer();
    const target = Boolean(expanded);
    if (target === state.open) return;
    clearCompletionBeam();
    renderFloat({ preserveMorph: true });
    startMorph(target, focusTarget);
  }

  function panelDragPosition(drag, dx, dy) {
    const geometry = drag.originLayout;
    const bounds = contentSafeBounds();
    const maxLeft = Math.max(bounds.left, bounds.right - geometry.width);
    const maxTop = Math.max(bounds.top, bounds.bottom - geometry.height);
    const left = clamp(drag.originPanelLeft + dx, bounds.left, maxLeft);
    const top = clamp(drag.originPanelTop + dy, bounds.top, maxTop);
    return {
      x: left + (geometry.width - geometry.chip.width) / 2,
      y: drag.lockedOpensDown
        ? top
        : top + geometry.height - geometry.chip.height,
    };
  }

  function applyPosition() {
    if (!state.popover || !state.fab || !state.position) return;
    state.position = clampPosition(state.position);
    state.layout = shellLayout();
    state.popover.style.left = `${state.layout.left}px`;
    state.popover.style.top = `${state.layout.top}px`;
    state.popover.style.width = `${state.layout.width}px`;
    state.popover.style.height = `${state.layout.height}px`;
    state.root.style.setProperty("--csw-panel-width", `${state.layout.width}px`);
    state.root.style.setProperty("--csw-panel-height", `${state.layout.height}px`);
    state.popover.style.setProperty("--csw-chip-left", `${state.layout.chip.left}px`);
    const compressionProgress = state.layout.compressionProgress || 0;
    const compressed = state.activeTab !== "settings" && compressionProgress > 0.001;
    state.popover.dataset.compressed = String(compressed);
    state.popover.style.setProperty(
      "--csw-content-fade-size",
      `${compressed ? Math.min(48, 14 + compressionProgress * 34) : 0}px`,
    );
    syncContentFade();
    state.fab.style.left = `${state.layout.chip.left}px`;
    state.fab.style.top = `${state.layout.chip.top}px`;
    if (!state.morphAnimation) applyMorphProgress(state.open ? 1 : 0);
  }

  // SVG filters provide material-specific backdrop treatment without adding third-party runtime code.
  function createDisplacementFilter(id, options) {
    document.getElementById(id)?.ownerSVGElement?.remove();
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("width", "0");
    svg.setAttribute("height", "0");
    svg.setAttribute("aria-hidden", "true");
    svg.style.cssText = "position:absolute;width:0;height:0;overflow:hidden;pointer-events:none";
    const stitchTiles = options.stitchTiles ? ` stitchTiles="${options.stitchTiles}"` : "";
    const blurNode = Number.isFinite(options.blur)
      ? `<feGaussianBlur in="noise" stdDeviation="${options.blur}" result="blurred"></feGaussianBlur>`
      : "";
    const displacementInput = blurNode ? "blurred" : "noise";
    svg.innerHTML = `
      <defs>
        <filter id="${id}" x="${options.x}" y="${options.y}" width="${options.width}" height="${options.height}" color-interpolation-filters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="${options.baseFrequency}" numOctaves="${options.numOctaves}" seed="${options.seed}"${stitchTiles} result="noise"></feTurbulence>
          ${blurNode}
          <feDisplacementMap in="SourceGraphic" in2="${displacementInput}" scale="${options.scale}" xChannelSelector="R" yChannelSelector="G"></feDisplacementMap>
        </filter>
      </defs>
    `;
    return svg;
  }

  function createClearFilter() {
    const svg = createDisplacementFilter(CLEAR_FILTER_ID, {
      x: "-15%",
      y: "-15%",
      width: "130%",
      height: "130%",
      baseFrequency: "0.006 0.010",
      numOctaves: 1,
      seed: 92,
      stitchTiles: "stitch",
      blur: 7,
      scale: 3,
    });
    state.clearDisplacement = svg.querySelector("feDisplacementMap");
    return svg;
  }

  function createLiquidFilter() {
    return createDisplacementFilter(LIQUID_FILTER_ID, {
      x: "-45%",
      y: "-45%",
      width: "190%",
      height: "190%",
      baseFrequency: "0.012 0.012",
      numOctaves: 2,
      seed: 92,
      blur: 2,
      scale: 85,
    });
  }

  function createCrystalFilter() {
    return createDisplacementFilter(CRYSTAL_FILTER_ID, {
      x: "-60%",
      y: "-60%",
      width: "220%",
      height: "220%",
      baseFrequency: "0.03 0.03",
      numOctaves: 2,
      seed: 92,
      blur: 2,
      scale: 140,
    });
  }

  function updateClearDisplacement(expanded, active) {
    if (!state.clearDisplacement) return;
    const scale = active ? (expanded ? 6 : 6) : (expanded ? 3 : 2);
    state.clearDisplacement.setAttribute("scale", String(scale));
  }

  function updateMaterialDistortion(expanded, active) {
    updateClearDisplacement(expanded, active);
  }

  // The DOM shell is created once; later renders update its contents without stacking another overlay.
  function installFloat() {
    if (!isCurrentRuntime()) return;
    document.querySelectorAll?.(`[${ROOT_ATTR}="true"]`).forEach((node) => {
      if (node !== state.root) node.remove();
    });
    if (state.root && document.body.contains(state.root)) return;

    state.position = savedPosition();
    state.root = document.createElement("div");
    state.root.setAttribute(ROOT_ATTR, "true");

    state.fab = document.createElement("button");
    state.fab.className = "csw-fab";
    state.fab.type = "button";
    state.fab.title = "下一步";
    state.fab.setAttribute("aria-controls", POPOVER_ID);
    state.fab.innerHTML = `${statusStageHtml()}${sourceTrackHtml()}`;

    state.popover = document.createElement("div");
    state.popover.className = "csw-popover";
    state.popover.dataset.open = "false";
    state.popover.dataset.morphing = "false";
    state.popover.dataset.completionBeam = "false";

    state.glass = document.createElement("div");
    state.glass.className = "csw-glass";
    state.glass.setAttribute("aria-hidden", "true");

    state.rim = document.createElement("div");
    state.rim.className = "csw-rim";
    state.rim.setAttribute("aria-hidden", "true");

    state.completionBeam = document.createElement("div");
    state.completionBeam.className = "csw-completion-beam";
    state.completionBeam.setAttribute("aria-hidden", "true");

    state.clearFilter = createClearFilter();
    state.liquidFilter = createLiquidFilter();
    state.crystalFilter = createCrystalFilter();

    const clearTexture = document.createElement("div");
    clearTexture.className = "csw-clear-texture";
    clearTexture.setAttribute("aria-hidden", "true");
    state.clearDistortion = document.createElement("div");
    state.clearDistortion.className = "csw-clear-distortion";
    state.clearDistortion.setAttribute("aria-hidden", "true");
    state.glass.append(clearTexture, state.clearDistortion);

    state.displacementTexture = document.createElement("div");
    state.displacementTexture.className = "csw-displacement-texture";
    state.displacementTexture.setAttribute("aria-hidden", "true");

    const materialLayer = document.createElement("div");
    materialLayer.className = "csw-material-layer";
    materialLayer.setAttribute("aria-hidden", "true");
    materialLayer.append(state.displacementTexture, state.glass, state.rim);
    materialLayer.append(state.completionBeam);

    state.panel = document.createElement("section");
    state.panel.id = POPOVER_ID;
    state.panel.className = "csw-panel";
    state.panel.setAttribute("role", "dialog");
    state.panel.setAttribute("aria-label", "下一步建议与回答大纲");
    state.panel.setAttribute("aria-hidden", "true");
    state.panel.inert = true;

    const resizeBottomLeft = document.createElement("span");
    resizeBottomLeft.className = "csw-resize-handle";
    resizeBottomLeft.dataset.corner = "bl";
    resizeBottomLeft.setAttribute("aria-hidden", "true");
    const resizeBottomRight = document.createElement("span");
    resizeBottomRight.className = "csw-resize-handle";
    resizeBottomRight.dataset.corner = "br";
    resizeBottomRight.setAttribute("aria-hidden", "true");

    state.popover.append(materialLayer, state.fab, state.panel, resizeBottomLeft, resizeBottomRight);
    state.root.append(state.clearFilter, state.liquidFilter, state.crystalFilter, state.popover);
    document.body.appendChild(state.root);

    state.fab.addEventListener("pointerdown", onFabPointerDown);
    state.fab.addEventListener("click", onFabClick);
    bindGlassPointerSurface(state.fab);
    state.panel.addEventListener("wheel", onPanelWheel, { passive: false });
    state.glass.addEventListener("click", onGlassClick);
    resetGlassPointer();
    state.keyHandler = onKeyDown;
    document.addEventListener("keydown", state.keyHandler, true);
    window.addEventListener("resize", onResize);
    installEyeTracking();
    installThemeObserver();
    installTypographyObserver();
    syncTheme();
    applyMaterial();
    installResize();
    applyPosition();
    settleMorph(0);
  }

  function onResize() {
    if (!state.position) return;
    const target = state.open ? 1 : 0;
    cancelMorphAnimations();
    state.position = clampPosition(state.position);
    applyPosition();
    settleMorph(target);
    syncContentFade();
  }

  function onPanelWheel(event) {
    if (!state.open || (!event.altKey && !event.metaKey) || event.deltaY === 0) return;
    event.preventDefault();
    event.stopPropagation();
    bumpFontSize(event.deltaY > 0 ? -1 : 1);
  }

  function onFabPointerDown(event) {
    beginDrag(event, "fab");
  }

  function dragTargetBlocked(target) {
    if (!(target instanceof Element)) return false;
    if (target.closest(".csw-head-side")) return true;
    if (target.closest(".csw-head-face")) return false;
    return Boolean(target.closest("button,a,input,textarea,select,[role='button']"));
  }

  function beginDrag(event, source) {
    if (event.button !== 0 || state.morphAnimation || !state.position) return;
    if (source === "fab" && state.open) return;
    if (source === "panel" && (!state.open || dragTargetBlocked(event.target))) return;

    state.dragCleanup?.();
    const handle = event.currentTarget;
    const originLayout = state.layout || shellLayout();
    const drag = {
      pointerId: event.pointerId,
      source,
      startedOnHeadFace: source === "panel" && event.target instanceof Element && Boolean(event.target.closest(".csw-head-face")),
      startX: event.clientX,
      startY: event.clientY,
      originX: state.position.x,
      originY: state.position.y,
      originLayout,
      originPanelLeft: originLayout.left,
      originPanelTop: originLayout.top,
      lockedOpensDown: source === "panel" ? originLayout.opensDown : null,
      panelHeight: source === "panel" ? originLayout.height : null,
      moved: false,
    };
    state.drag = drag;
    state.suppressFabClick = false;
    state.suppressHeadFaceClick = false;
    resetEyePointer();

    const onPointerMove = (moveEvent) => {
      if (state.drag !== drag || moveEvent.pointerId !== drag.pointerId) return;
      const dx = moveEvent.clientX - drag.startX;
      const dy = moveEvent.clientY - drag.startY;
      if (!drag.moved && Math.hypot(dx, dy) < 3) return;
      if (!drag.moved) {
        drag.moved = true;
        handle?.setAttribute?.("data-dragging", "true");
        try { handle?.setPointerCapture?.(drag.pointerId); } catch {}
      }
      moveEvent.preventDefault();
      const nextPosition = source === "panel"
        ? panelDragPosition(drag, dx, dy)
        : { x: drag.originX + dx, y: drag.originY + dy };
      setPosition(nextPosition);
      snapRightIfNear();
    };

    const cleanup = () => {
      window.removeEventListener("pointermove", onPointerMove, true);
      window.removeEventListener("pointerup", onPointerEnd, true);
      window.removeEventListener("pointercancel", onPointerEnd, true);
      handle?.removeAttribute?.("data-dragging");
      try { handle?.releasePointerCapture?.(drag.pointerId); } catch {}
      if (state.dragCleanup === cleanup) state.dragCleanup = null;
    };

    const onPointerEnd = (endEvent) => {
      if (state.drag !== drag || endEvent.pointerId !== drag.pointerId) return;
      cleanup();
      if (!drag.moved) {
        state.drag = null;
        return;
      }
      const snapped = snapRightIfNear(true, true);
      state.drag = null;
      if (!snapped) persistPosition();
      if (source === "fab") {
        state.suppressFabClick = true;
        window.setTimeout(() => { state.suppressFabClick = false; }, 300);
      } else {
        state.suppressHeadFaceClick = true;
        window.setTimeout(() => { state.suppressHeadFaceClick = false; }, 300);
        if (drag.startedOnHeadFace && document.activeElement instanceof HTMLElement) {
          document.activeElement.blur();
        }
      }
      syncEyeTracking();
    };

    state.dragCleanup = cleanup;
    window.addEventListener("pointermove", onPointerMove, { capture: true, passive: false });
    window.addEventListener("pointerup", onPointerEnd, true);
    window.addEventListener("pointercancel", onPointerEnd, true);
    if (source === "panel" && !drag.startedOnHeadFace) event.preventDefault();
  }

  // Dragging, right docking, resizing, and persisted bounds share the same clamped position model.
  function installPanelDrag() {
    const head = state.panel?.querySelector(".csw-head");
    if (head && head.dataset.dragBound !== "1") {
      head.dataset.dragBound = "1";
      head.addEventListener("pointerdown", (event) => beginDrag(event, "panel"));
    }
  }

  function resizePositionFromFace(nextHeight, resize) {
    const panelTop = resize.faceCenterY - resize.faceOffsetY;
    return {
      x: resize.faceCenterX - resize.chipWidth / 2,
      y: resize.lockedOpensDown
        ? panelTop
        : panelTop + nextHeight - resize.chipHeight,
    };
  }

  function installResize() {
    if (!state.popover || state.popover.dataset.resizeBound === "1") return;
    state.popover.dataset.resizeBound = "1";
    state.popover.querySelectorAll(".csw-resize-handle").forEach((handle) => {
      handle.addEventListener("pointerdown", (event) => {
        if (event.button !== 0 || !state.open || state.activeTab === "settings" || state.morphAnimation || !state.layout) return;
        event.preventDefault();
        event.stopPropagation();
        state.resizeCleanup?.();
        const corner = handle.dataset.corner === "bl" ? "bl" : "br";
        const startRect = state.popover.getBoundingClientRect();
        const startWidth = state.layout.width;
        const startHeight = state.layout.height;
        const startX = event.clientX;
        const startY = event.clientY;
        const faceRect = state.panel?.querySelector(".csw-head-face")?.getBoundingClientRect();
        const faceCenterX = faceRect ? faceRect.left + faceRect.width / 2 : startRect.left + startRect.width / 2;
        const faceCenterY = faceRect ? faceRect.top + faceRect.height / 2 : startRect.top + CHIP_HEIGHT / 2;
        const resize = {
          pointerId: event.pointerId,
          corner,
          chipHeight: state.layout.chip.height,
          chipWidth: state.layout.chip.width,
          faceCenterX,
          faceCenterY,
          faceOffsetY: faceCenterY - startRect.top,
          lockedOpensDown: state.layout.opensDown,
        };
        state.resizeDrag = resize;
        state.popover.dataset.resizing = "true";

        const onMove = (moveEvent) => {
          if (state.resizeDrag !== resize || moveEvent.pointerId !== resize.pointerId) return;
          moveEvent.preventDefault();
          const dx = moveEvent.clientX - startX;
          const dy = moveEvent.clientY - startY;
          const nextWidth = clampPanelWidth(corner === "bl" ? startWidth - dx * 2 : startWidth + dx * 2);
          const nextHeight = clampPanelHeight(startHeight + dy);
          state.width = nextWidth;
          state.height = nextHeight;
          state.position = clampPosition(resizePositionFromFace(nextHeight, resize));
          applyPosition();
        };

        const cleanup = () => {
          window.removeEventListener("pointermove", onMove, true);
          window.removeEventListener("pointerup", endResize, true);
          window.removeEventListener("pointercancel", endResize, true);
          window.removeEventListener("blur", finishResize, true);
          document.removeEventListener("visibilitychange", onVisibilityChange, true);
          handle.removeEventListener("lostpointercapture", onLostPointerCapture, true);
          if (state.resizeCleanup === finishResize) state.resizeCleanup = null;
        };

        const finishResize = () => {
          cleanup();
          if (state.resizeDrag === resize) state.resizeDrag = null;
          state.popover?.removeAttribute("data-resizing");
          storage.set(WIDTH_KEY, String(state.width));
          storage.set(HEIGHT_KEY, String(state.height));
          applyPosition();
          try { handle.releasePointerCapture(resize.pointerId); } catch {}
        };

        const endResize = (endEvent) => {
          if (state.resizeDrag !== resize || endEvent.pointerId !== resize.pointerId) return;
          finishResize();
        };

        const onLostPointerCapture = (captureEvent) => {
          if (captureEvent.pointerId !== resize.pointerId) return;
          finishResize();
        };

        const onVisibilityChange = () => {
          if (document.visibilityState === "hidden") finishResize();
        };

        state.resizeCleanup = finishResize;
        try { handle.setPointerCapture?.(event.pointerId); } catch {}
        window.addEventListener("pointermove", onMove, { capture: true, passive: false });
        window.addEventListener("pointerup", endResize, true);
        window.addEventListener("pointercancel", endResize, true);
        window.addEventListener("blur", finishResize, true);
        document.addEventListener("visibilitychange", onVisibilityChange, true);
        handle.addEventListener("lostpointercapture", onLostPointerCapture, true);
      });

      handle.addEventListener("dblclick", (event) => {
        event.preventDefault();
        event.stopPropagation();
        state.resizeCleanup?.();
        state.width = PANEL_WIDTH;
        state.height = clampPanelHeight(PANEL_HEIGHT);
        storage.set(WIDTH_KEY, String(state.width));
        storage.set(HEIGHT_KEY, String(state.height));
        applyPosition();
      });
    });
  }

  function eyeTrackingActive() {
    return state.fabExpression === "answering"
      && !state.open
      && !state.morphAnimation
      && !state.drag
      && state.root?.dataset.hidden !== "true";
  }

  function curiousEyeTrackingActive() {
    return state.activeTab === "settings"
      && state.open
      && !state.morphAnimation
      && !state.drag
      && state.root?.dataset.hidden !== "true";
  }

  function eyeTrackingNeeded() {
    return eyeTrackingActive() || curiousEyeTrackingActive();
  }

  function applyEyeOffset(x = 0, y = 0) {
    state.root?.style.setProperty("--csw-eye-x", `${x.toFixed(2)}px`);
    state.root?.style.setProperty("--csw-eye-y", `${y.toFixed(2)}px`);
  }

  function applyCuriousEyeOffset(x = 0, y = 0) {
    state.root?.style.setProperty("--csw-curious-eye-x", `${x.toFixed(2)}px`);
    state.root?.style.setProperty("--csw-curious-eye-y", `${y.toFixed(2)}px`);
  }

  function pointerInsideRect(pointer, rect) {
    return pointer.x >= rect.left
      && pointer.x <= rect.right
      && pointer.y >= rect.top
      && pointer.y <= rect.bottom;
  }

  function eyeOffset(pointer, rect, maxX, maxY, reachDistance) {
    const dx = pointer.x - (rect.left + rect.width / 2);
    const dy = pointer.y - (rect.top + rect.height / 2);
    const distance = Math.hypot(dx, dy);
    const reach = clamp(distance / reachDistance, 0, 1);
    const angle = Math.atan2(dy, dx);
    return {
      x: Math.cos(angle) * maxX * reach,
      y: Math.sin(angle) * maxY * reach,
    };
  }

  function flushEyePointer(generation = state.runtimeGeneration) {
    if (!isCurrentRuntime(generation)) return;
    state.eyeRaf = 0;
    if (!state.eyePointer || !eyeTrackingNeeded()) {
      applyEyeOffset();
      applyCuriousEyeOffset();
      return;
    }

    if (eyeTrackingActive() && state.fab) {
      const rect = state.fab.getBoundingClientRect();
      if (rect.width && rect.height) {
        const offset = eyeOffset(state.eyePointer, rect, EYE_MAX_X, EYE_MAX_Y, 220);
        applyEyeOffset(offset.x, offset.y);
      } else {
        applyEyeOffset();
      }
    } else {
      applyEyeOffset();
    }

    if (!curiousEyeTrackingActive()) {
      applyCuriousEyeOffset();
      return;
    }
    const surface = state.panel?.querySelector('.csw-mouth-stage[data-mouth-stage="settings"]');
    const face = state.panel?.querySelector('.csw-head-face[data-expression="curious"]');
    const surfaceRect = surface?.getBoundingClientRect();
    const faceRect = face?.getBoundingClientRect();
    if (!surfaceRect?.width || !surfaceRect.height || !faceRect?.width || !faceRect.height
      || !pointerInsideRect(state.eyePointer, surfaceRect)) {
      applyCuriousEyeOffset();
      return;
    }
    const offset = eyeOffset(
      state.eyePointer,
      faceRect,
      CURIOUS_EYE_MAX_X,
      CURIOUS_EYE_MAX_Y,
      Math.max(120, surfaceRect.height)
    );
    applyCuriousEyeOffset(offset.x, offset.y);
  }

  function scheduleEyePointer() {
    if (!isCurrentRuntime() || state.eyeRaf) return;
    const generation = state.runtimeGeneration;
    state.eyeRaf = window.requestAnimationFrame(() => flushEyePointer(generation));
  }

  function resetEyePointer(clearPointer = false) {
    if (state.eyeRaf) window.cancelAnimationFrame(state.eyeRaf);
    state.eyeRaf = 0;
    if (clearPointer) state.eyePointer = null;
    applyEyeOffset();
    applyCuriousEyeOffset();
  }

  function syncEyeTracking() {
    if (!isCurrentRuntime()) return;
    if (!eyeTrackingNeeded()) {
      resetEyePointer();
      return;
    }
    scheduleEyePointer();
  }

  // Eye tracking is pointer-only decoration and is reset whenever the pointer leaves our surfaces.
  function installEyeTracking() {
    if (state.eyeCleanup) return;
    const onPointerMove = (event) => {
      state.eyePointer = { x: event.clientX, y: event.clientY };
      if (eyeTrackingNeeded()) scheduleEyePointer();
    };
    const onPointerLeave = () => resetEyePointer(true);
    window.addEventListener("pointermove", onPointerMove, { passive: true });
    window.addEventListener("blur", onPointerLeave);
    document.addEventListener("mouseleave", onPointerLeave);
    state.eyeCleanup = () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("blur", onPointerLeave);
      document.removeEventListener("mouseleave", onPointerLeave);
      resetEyePointer(true);
      state.eyeCleanup = null;
    };
  }

  function onFabClick(event) {
    if (state.suppressFabClick || state.drag?.moved) {
      state.suppressFabClick = false;
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    setOpen(!state.open, state.open ? "chip" : (event.detail === 0 ? "panel" : ""));
  }

  function onHeadFaceClick(event) {
    if (state.suppressHeadFaceClick || state.drag?.moved) {
      state.suppressHeadFaceClick = false;
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    setOpen(false, "chip");
  }

  function onGlassClick(event) {
    if (state.popover?.dataset.morphing !== "true") return;
    event.preventDefault();
    event.stopPropagation();
    const expanded = !state.open;
    startMorph(expanded, expanded ? "" : "chip");
  }

  function bindGlassPointerSurface(surface) {
    if (!(surface instanceof Element)) return;
    surface.addEventListener("pointerenter", onShellPointerMove);
    surface.addEventListener("pointermove", onShellPointerMove);
    surface.addEventListener("pointerleave", onShellPointerLeave);
    surface.addEventListener("pointercancel", resetGlassPointer);
  }

  function onShellPointerMove(event) {
    if (!state.glass || !state.popover) return;
    const expanded = state.open || state.popover.dataset.open === "true";
    const surface = event.currentTarget;
    const validSurface = expanded
      ? surface instanceof Element && surface.matches(".csw-head-face")
      : surface === state.fab;
    if (!validSurface || !(surface instanceof Element)) {
      resetGlassPointer();
      return;
    }
    const surfaceRect = surface.getBoundingClientRect();
    if (!surfaceRect.width || !surfaceRect.height) return;
    const rect = state.glass.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    state.popover.toggleAttribute("data-csw-hot-hover", true);
    const x = clamp(((event.clientX - rect.left) / rect.width) * 100, 0, 100);
    const y = clamp(((event.clientY - rect.top) / rect.height) * 100, 0, 100);
    const angle = Math.atan2(event.clientY - rect.top - rect.height / 2, event.clientX - rect.left - rect.width / 2) * 180 / Math.PI;
    const normalizedX = (event.clientX - surfaceRect.left) / surfaceRect.width - 0.5;
    const normalizedY = (event.clientY - surfaceRect.top) / surfaceRect.height - 0.5;
    const proximity = 1 - clamp(Math.hypot(normalizedX, normalizedY) / 0.72, 0, 1);
    updateMaterialDistortion(expanded, true);
    const strength = expanded ? 0.1 + proximity * 0.12 : 0.62 + proximity * 0.38;
    const parallaxX = expanded ? 1.6 : 1.8;
    const parallaxY = expanded ? 1.2 : 1.4;
    state.popover.style.setProperty("--csw-glass-x", `${x.toFixed(2)}%`);
    state.popover.style.setProperty("--csw-glass-y", `${y.toFixed(2)}%`);
    state.popover.style.setProperty("--csw-glass-px", `${(normalizedX * parallaxX).toFixed(2)}px`);
    state.popover.style.setProperty("--csw-glass-py", `${(normalizedY * parallaxY).toFixed(2)}px`);
    state.popover.style.setProperty("--csw-glass-strength", strength.toFixed(3));
    state.popover.style.setProperty("--csw-glass-angle", `${angle.toFixed(2)}deg`);
  }

  function onShellPointerLeave() {
    resetGlassPointer();
  }

  function resetGlassPointer() {
    const expanded = state.open || state.popover?.dataset.open === "true";
    state.popover?.removeAttribute("data-csw-hot-hover");
    updateMaterialDistortion(expanded, false);
    state.popover?.style.setProperty("--csw-glass-x", "28%");
    state.popover?.style.setProperty("--csw-glass-y", expanded ? "16%" : "22%");
    state.popover?.style.setProperty("--csw-glass-px", "0px");
    state.popover?.style.setProperty("--csw-glass-py", "0px");
    state.popover?.style.setProperty("--csw-glass-strength", "0");
    state.popover?.style.setProperty("--csw-glass-angle", "-40deg");
  }

  function onKeyDown(event) {
    if (event.key === "Escape" && state.open) {
      event.preventDefault();
      event.stopImmediatePropagation();
      setOpen(false, "chip");
      return;
    }
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const target = event.target;
    if (target instanceof Element && (
      target.closest("input, textarea, select, [contenteditable='true'], .ProseMirror") ||
      target.isContentEditable
    )) return;
    const isOutlineToggle = event.shiftKey && (
      event.code === "KeyO" || String(event.key || "").toUpperCase() === "O"
    );
    if (!isOutlineToggle || !state.panel || !outlineEnabled()) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (state.open && state.activeTab === "outline") {
      setOpen(false, "chip");
      return;
    }
    state.activeTab = "outline";
    renderFloat({ preserveMorph: true });
    void refreshOutline();
    if (!state.open) setOpen(true, "panel");
  }

  function faceEyeHtml() {
    return `<span class="csw-fab-eye"><svg class="csw-fab-happy-arc" viewBox="0 0 18 12" aria-hidden="true" focusable="false"><path d="M1.5 9 C4.6 3.2 13.4 3.2 16.5 9"></path></svg></span>`;
  }

  function faceHtml() {
    return `
      <span class="csw-fab-face" aria-hidden="true">
        ${faceEyeHtml()}
        ${faceEyeHtml()}
      </span>
    `;
  }

  function statusStageHtml() {
    return `<span class="csw-status-stage">${faceHtml()}</span>`;
  }

  function sourceTrackHtml(paneCue = { direction: "single", angle: null }, trackHeight = CHIP_HEIGHT) {
    const angle = Number.isFinite(state.sourceCueAngle) && paneCue.direction !== "single"
      ? state.sourceCueAngle
      : paneCue.angle;
    const cue = paneCueForTrack({ direction: paneCue.direction, angle }, trackHeight);
    return `<span class="csw-source-track" style="--csw-source-track-height:${trackHeight}px" aria-hidden="true"><span class="csw-source-dot" data-direction="${escapeAttr(cue.direction)}" style="--csw-source-x:${cue.x}px;--csw-source-y:${cue.y}px"></span></span>`;
  }

  function normalizeSourceCueDelta(fromAngle, toAngle) {
    return ((toAngle - fromAngle + Math.PI * 3) % (Math.PI * 2)) - Math.PI;
  }

  function cancelSourceCueAnimation() {
    if (!state.sourceCueAnimation) return;
    cancelAnimationFrame(state.sourceCueAnimation);
    state.sourceCueAnimation = 0;
  }

  function applySourceCueAngle(angle, direction) {
    state.sourceCueAngle = angle;
    [
      [state.fab?.querySelector(".csw-source-dot"), CHIP_HEIGHT],
      [state.panel?.querySelector(".csw-head-face .csw-source-dot"), 32],
    ].forEach(([dot, trackHeight]) => {
      if (!dot) return;
      dot.setAttribute("data-direction", direction);
      if (!Number.isFinite(angle)) return;
      const point = capsuleBoundaryPoint(angle, CHIP_WIDTH, trackHeight);
      dot.style.setProperty("--csw-source-x", `${point.x}px`);
      dot.style.setProperty("--csw-source-y", `${point.y}px`);
    });
  }

  function animateSourceCue(paneCue) {
    if (!isCurrentRuntime()) return;
    cancelSourceCueAnimation();
    if (paneCue.direction === "single" || !Number.isFinite(paneCue.angle)) {
      applySourceCueAngle(null, "single");
      return;
    }

    const targetAngle = paneCue.angle;
    if (!Number.isFinite(state.sourceCueAngle) || prefersReducedMotion()) {
      applySourceCueAngle(targetAngle, paneCue.direction);
      return;
    }

    const startAngle = state.sourceCueAngle;
    const delta = normalizeSourceCueDelta(startAngle, targetAngle);
    if (Math.abs(delta) < 0.001) {
      applySourceCueAngle(targetAngle, paneCue.direction);
      return;
    }

    const duration = 180 + Math.min(1, Math.abs(delta) / Math.PI) * 120;
    const generation = state.runtimeGeneration;
    const startedAt = performance.now();
    const tick = (now) => {
      if (!isCurrentRuntime(generation)) return;
      const progress = clamp((now - startedAt) / duration, 0, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      applySourceCueAngle(startAngle + delta * eased, paneCue.direction);
      if (progress < 1) state.sourceCueAnimation = requestAnimationFrame(tick);
      else {
        state.sourceCueAnimation = 0;
        applySourceCueAngle(targetAngle, paneCue.direction);
      }
    };
    state.sourceCueAnimation = requestAnimationFrame(tick);
  }

  function bridgeErrorPresentation(error = state.bridgeError) {
    const text = normalizeText(error);
    const match = FRIENDLY_BRIDGE_ERRORS.find((item) => item.pattern.test(text));
    return match || {
      title: "生成失败，稍后重试",
      message: "",
    };
  }

  function outlineErrorTitle(error = state.outlineError) {
    const text = normalizeText(error);
    if (/找不到对应的小节/i.test(text)) return "找不到对应内容，刷新后再试";
    return FRIENDLY_BRIDGE_ERRORS.find((item) => item.pattern.test(text))?.title || "大纲暂不可用，稍后重试";
  }

  function statusTone(expression) {
    if (expression === "error") return "error";
    if (expression === "answering" || expression === "generating") return "busy";
    if (expression === "ready" || expression === "surprise") return "ready";
    return "idle";
  }

  function statusToneForView(expression) {
    if (state.activeTab === "outline") {
      if (state.outlineStatus === "pending") return "busy";
      if (state.outlineStatus === "error") return "error";
      if (state.outlineItems.length) return "ready";
      return "idle";
    }
    if (state.activeTab === "settings") {
      if (!state.settingsLoaded) return "busy";
      if (/失败|错误|不可用/i.test(state.settingsStatus)) return "error";
      if (outlineEnabled() && !stepwiseEnabled()) return "ready";
      if (stepwiseEnabled()
        && state.settings.baseUrlConfigured
        && state.settings.model
        && state.settings.apiKeyConfigured) return "ready";
      return "idle";
    }
    return statusTone(expression);
  }

  function refreshControlState() {
    if (state.activeTab === "settings") {
      return { blocked: false, title: "重新读取设置" };
    }
    if (state.activeTab === "outline") {
      const blocked = state.outlineStatus === "pending";
      return { blocked, title: blocked ? "正在整理大纲" : "刷新大纲" };
    }
    const blocked = state.bridgeStatus === "pending" || chatBusy();
    return { blocked, title: blocked ? "等待回答完成" : "刷新建议" };
  }

  function refreshCurrentView() {
    if (state.activeTab === "settings") return reloadSettings();
    if (state.activeTab === "outline") return refreshOutline();
    if (!stepwiseEnabled()) return;
    return forceRefreshStepwise();
  }

  // Outline extraction is deliberately conservative: only visible, structured headings become targets.
  function outlineVisible(node) {
    if (!(node instanceof Element)) return false;
    const rect = node.getBoundingClientRect();
    return Boolean(rect.width > 8 && rect.height > 8);
  }

  function outlineMarkdownRoot(messageNode) {
    if (!(messageNode instanceof Element)) return null;
    const preferred = messageNode.querySelector(
      [
        "[class*='markdownContent']",
        "[class*='markdown-content']",
        ".markdown",
        ".prose",
        "article",
      ].join(",")
    );
    if (preferred && !preferred.closest(`[${ROOT_ATTR}="true"]`)) return preferred;
    return messageNode;
  }

  function outlineProtectedSurface(node) {
    if (!(node instanceof Element)) return true;
    return Boolean(node.closest([
      `[${ROOT_ATTR}="true"]`,
      "[contenteditable='true']",
      "textarea",
      "input",
      "form",
      ".ProseMirror",
    ].join(",")));
  }

  function outlineInCodeLike(node) {
    if (!(node instanceof Element)) return true;
    return Boolean(node.closest("pre, code, kbd, samp, [data-code-block], .cm-editor, .monaco-editor"));
  }

  function outlineInTableLike(node) {
    if (!(node instanceof Element)) return true;
    return Boolean(node.closest(OUTLINE_TABLE_SELECTOR));
  }

  function outlineHeadingLevelFromTag(tag) {
    const match = /^h([1-6])$/i.exec(tag || "");
    return match ? Number(match[1]) : 0;
  }

  function outlineIsMarkerOnlyTitle(text) {
    const value = normalizeText(text);
    if (!value) return true;
    if (/^[一二三四五六七八九十百零]+[、.．)]?$/.test(value)) return true;
    if (/^\d{1,2}[\.、．)]?$/.test(value)) return true;
    if (/^[（(]\d{1,2}[）)]$/.test(value)) return true;
    return /^#{1,6}$/.test(value);
  }

  function outlineIsNoiseTitle(text) {
    if (!text || outlineIsMarkerOnlyTitle(text)) return true;
    if (text.length < MIN_OUTLINE_TITLE_LEN || text.length > MAX_OUTLINE_TITLE_LEN) return true;
    if (text.length <= 4 && !/[0-9一二三四五六七八九十#：:]/.test(text) && !outlineHasChapterHeading(text)) return true;
    if (/^https?:\/\//i.test(text)) return true;
    if (/^[\w./~-]+\.(js|ts|json|md|py|sh|log|png|jpg)$/i.test(text)) return true;
    if (/^\$ |^>`|^```/.test(text)) return true;
    if (/^(复制|copy|edit|编辑|share|分享|continue|继续|retry|重试|项|实现|位置|范围|标题|跳转|折叠|刷新)$/i.test(text)) return true;
    if (/^[\d\s:./-]+$/.test(text)) return true;
    if (/^\/Users\/|^~\/|^\.\/|^\/Volumes\//.test(text)) return true;
    return /^(OK|PASS|FAIL|true|false|null)$/i.test(text);
  }

  function outlineHasChapterHeading(text) {
    const value = normalizeText(text);
    if (!value) return false;
    if (/^(摘要|简介|概述|概览|前言|背景|目标|现状|问题(?:分析)?|原因(?:分析)?|分析|方案|解决方案|步骤|实施步骤|实现|验证|验证结果|测试|测试结果|结果|结论|最终结论|总结|建议|后续建议|注意(?:事项)?|说明|补充说明|附录|下一步)(?:\s*[：:—-]\s*\S.*)?$/.test(value)) {
      return value.length <= 24;
    }
    return /^(abstract|introduction|overview|background|goals?|problems?|causes?|analysis|solutions?|steps?|implementation|verification|tests?|results?|conclusions?|summary|recommendations?|notes?|appendix|next steps?)(?:\s*[:：—-]\s*\S.*)?$/i.test(value)
      && value.length <= 32;
  }

  function outlineLooksStructuredHeading(text) {
    const value = normalizeText(text);
    if (!value || outlineIsMarkerOnlyTitle(value)) return false;
    if (/^#{1,6}\s+\S/.test(value)) return true;
    if (/^第[一二三四五六七八九十百零\d]+[章节部分步]/.test(value)) return true;
    if (/^[一二三四五六七八九十]+[、.．]\s*\S{2,}/.test(value)) return true;
    if (/^（?[0-9]{1,2}）\s*\S{2,}/.test(value) || /^\([0-9]{1,2}\)\s*\S{2,}/.test(value)) return true;
    if (/^\d{1,2}[\.、．\)]\s*\S{2,}/.test(value)) return true;
    return outlineHasChapterHeading(value);
  }

  function outlineScorePseudoHeading(text, levelHint) {
    let score = levelHint ? 20 : 0;
    if (!outlineLooksStructuredHeading(text) && !levelHint) return 0;
    if (/^#{1,6}\s+\S/.test(text)) score += 50;
    if (/^第[一二三四五六七八九十百零\d]+[章节部分步]/.test(text)) score += 30;
    if (/^[一二三四五六七八九十]+[、.．]\s*\S{2,}/.test(text)) score += 28;
    if (/^（?[0-9]{1,2}）\s*\S{2,}/.test(text) || /^\([0-9]{1,2}\)\s*\S{2,}/.test(text)) score += 24;
    if (/^\d{1,2}[\.、．\)]\s*\S{2,}/.test(text)) score += 26;
    if (/[：:]$/.test(text) && text.length <= 18 && text.length >= 4) score += 8;
    if (outlineHasChapterHeading(text)) score += 24;
    if (text.length >= 4 && text.length <= 20) score += 6;
    if (text.length >= 28) score -= 8;
    if (/[。！？]$/.test(text)) score -= 12;
    if (text.split(" ").length > 12) score -= 10;
    return score;
  }

  function outlineStripHeadingMarkers(text) {
    const stripped = normalizeText(text)
      .replace(/^#{1,6}\s+/, "")
      .replace(/^([（(]?\d{1,2}[）)]|[一二三四五六七八九十]{1,3}|\d{1,2})[\.、．\)]\s*/, "");
    return stripped && !outlineIsMarkerOnlyTitle(stripped) ? stripped : normalizeText(text);
  }

  function outlineDisplayHeadingTitle(text) {
    const value = normalizeText(text).replace(/^#{1,6}\s+/, "");
    return value.length <= MAX_OUTLINE_TITLE_LEN ? value : `${value.slice(0, MAX_OUTLINE_TITLE_LEN - 1)}…`;
  }

  function outlineTitlesEquivalent(left, right) {
    const a = normalizeText(left);
    const b = normalizeText(right);
    return Boolean(a && b && (a === b || outlineStripHeadingMarkers(a) === outlineStripHeadingMarkers(b)
      || outlineDisplayHeadingTitle(a) === outlineDisplayHeadingTitle(b)));
  }

  function outlineOwnsOwnLine(node, text) {
    if (!(node instanceof Element)) return false;
    const parent = node.parentElement;
    if (!parent) return true;
    const parentText = normalizeText(parent.innerText || parent.textContent || "");
    if (!parentText || parentText === text) return true;
    return parentText.startsWith(text) && parentText.length <= text.length + 4;
  }

  function outlineHeadingNumbering(text) {
    const value = normalizeText(text);
    const patterns = [
      [/^([一二三四五六七八九十]+[、.．])\s*(\S.*)$/, "han"],
      [/^(第[一二三四五六七八九十百零\d]+[章节部分步])\s*(\S.*)$/, "chapter"],
      [/^((?:（[0-9]{1,2}）|\([0-9]{1,2}\)))\s*(\S.*)$/, "arabic-parenthesized"],
    ];
    for (const [pattern, key] of patterns) {
      const match = value.match(pattern);
      if (match) return { prefix: match[1], title: match[2], pattern: key };
    }
    const arabic = value.match(/^((?:\d{1,2}[\.、．\)])+)\s*(\S.*)$/);
    if (!arabic) return { prefix: "", title: value, pattern: "" };
    const segments = arabic[1].match(/\d{1,2}/g)?.length || 1;
    const separators = arabic[1].match(/[\.、．\)]/g)?.join("") || ".";
    return {
      prefix: arabic[1],
      title: arabic[2],
      pattern: `arabic:${separators}:${segments}`,
    };
  }

  function outlineHeadingCandidate(node, kind) {
    if (!(node instanceof Element) || !outlineVisible(node) || outlineProtectedSurface(node) || outlineInCodeLike(node)) return null;
    if (node.closest(`[${ROOT_ATTR}="true"]`)) return null;

    const text = normalizeText(node.innerText || node.textContent || "");
    if (!text || text.length > MAX_OUTLINE_TITLE_LEN + 8) return null;
    const displayText = outlineDisplayHeadingTitle(text);
    if (outlineIsNoiseTitle(displayText) || outlineIsMarkerOnlyTitle(displayText)) return null;
    const numbering = outlineHeadingNumbering(displayText);

    if (kind === "semantic") {
      const tagLevel = outlineHeadingLevelFromTag(node.tagName);
      const ariaLevel = Number(node.getAttribute("aria-level") || 0);
      return {
        el: node,
        text: displayText,
        level: clamp(tagLevel || ariaLevel || 2, 1, 6),
        numberingPattern: numbering.pattern,
        numberPrefix: numbering.prefix,
        labelText: numbering.title,
        kind,
      };
    }

    if (outlineInTableLike(node)) return null;
    const childCount = node.children?.length || 0;
    if (childCount > 3 || node.querySelector("p,div,li,h1,h2,h3,h4,h5,h6,table,pre")) return null;

    if (node.matches("strong,b")) {
      if (!outlineOwnsOwnLine(node, text)) return null;
      const score = outlineScorePseudoHeading(text, 1) + 8;
      if (score < OUTLINE_PSEUDO_MIN_SCORE) return null;
      return {
        el: node,
        text: displayText,
        level: 3,
        numberingPattern: numbering.pattern,
        numberPrefix: numbering.prefix,
        labelText: numbering.title,
        kind,
      };
    }

    const rect = node.getBoundingClientRect();
    if (rect.height > 84 || !outlineLooksStructuredHeading(text)) return null;
    const score = outlineScorePseudoHeading(text, 0);
    if (score < OUTLINE_PSEUDO_MIN_SCORE) return null;
    return {
      el: node,
      text: displayText,
      level: numbering.pattern ? 2 : text.length <= 12 ? 2 : 3,
      numberingPattern: numbering.pattern,
      numberPrefix: numbering.prefix,
      labelText: numbering.title,
      kind,
    };
  }

  function outlineCollectSemanticHeadings(root) {
    if (!(root instanceof Element)) return [];
    const result = [];
    const nodes = root.querySelectorAll(OUTLINE_SEMANTIC_HEADING_SELECTOR);
    for (const node of nodes) {
      const item = outlineHeadingCandidate(node, "semantic");
      if (item) result.push(item);
    }
    return result;
  }

  function outlineCollectPseudoHeadings(root) {
    if (!(root instanceof Element)) return [];
    const result = [];
    const nodes = root.querySelectorAll(OUTLINE_PSEUDO_HEADING_SELECTOR);
    for (const node of nodes) {
      if (node.closest(OUTLINE_SEMANTIC_HEADING_SELECTOR)) continue;
      const item = outlineHeadingCandidate(node, "pseudo");
      if (item) result.push(item);
    }
    return result;
  }

  function outlineSortInDocumentOrder(items) {
    return items.slice().sort((left, right) => {
      if (left.el === right.el) return 0;
      const position = left.el.compareDocumentPosition(right.el);
      if (position & Node.DOCUMENT_POSITION_FOLLOWING) return -1;
      if (position & Node.DOCUMENT_POSITION_PRECEDING) return 1;
      return 0;
    });
  }

  function outlineCollectHeadingElements(root) {
    const semanticItems = outlineCollectSemanticHeadings(root);
    if (semanticItems.length >= MIN_OUTLINE_ITEMS) return outlineSortInDocumentOrder(semanticItems);
    return outlineSortInDocumentOrder([...semanticItems, ...outlineCollectPseudoHeadings(root)]);
  }

  function outlineDedupeItems(items) {
    const seen = new Set();
    const result = [];
    for (const item of items) {
      const key = `${item.level}|${item.text}`;
      if (seen.has(key)) continue;
      const previous = result.at(-1);
      if (previous && (previous.text === item.text || previous.el.contains(item.el) || item.el.contains(previous.el))) {
        continue;
      }
      seen.add(key);
      result.push(item);
      if (result.length >= MAX_OUTLINE_ITEMS) break;
    }
    return result;
  }

  function outlineNormalizeDisplayLevels(items) {
    if (!items.length) return items;
    const minimumLevel = Math.min(...items.map((item) => item.level));
    const numberedLevels = new Map();
    items.forEach((item) => {
      const baseLevel = item.level - minimumLevel;
      if (!item.numberingPattern) {
        item.displayLevel = baseLevel;
        return;
      }
      if (!numberedLevels.has(item.numberingPattern)) numberedLevels.set(item.numberingPattern, baseLevel);
      item.displayLevel = numberedLevels.get(item.numberingPattern);
    });
    return items;
  }

  function outlineMarkItems(items) {
    items.forEach((item, index) => {
      const id = `stepwise-outline-${hashText(`${index}:${item.text}`)}-${index + 1}`;
      item.id = id;
      item.el.setAttribute(MARK_ATTR, id);
    });
    return items;
  }

  function outlineClearMarks(root = document) {
    if (!root?.querySelectorAll) return;
    root.querySelectorAll(`[${MARK_ATTR}]`).forEach((node) => node.removeAttribute(MARK_ATTR));
    root.querySelectorAll(`.${HIGHLIGHT_CLASS}`).forEach((node) => node.classList.remove(HIGHLIGHT_CLASS));
  }

  function outlineFindScrollContainer(fromElement) {
    let node = fromElement instanceof Element ? fromElement.parentElement : null;
    while (node && node !== document.documentElement) {
      const style = window.getComputedStyle(node);
      const overflowY = style.overflowY || style.overflow;
      if (/(auto|scroll|overlay)/.test(overflowY) && node.scrollHeight > node.clientHeight + 4) return node;
      node = node.parentElement;
    }
    return document.scrollingElement || document.documentElement;
  }

  function outlineIsDocumentScroller(container) {
    return container === document.scrollingElement
      || container === document.documentElement
      || container === document.body;
  }

  // Codex thread scrollers may use column-reverse, where valid scrollTop values are negative.
  function outlineScrollBounds(container) {
    const maxDistance = Math.max(0, container.scrollHeight - container.clientHeight);
    const style = window.getComputedStyle(container);
    const reversed = !outlineIsDocumentScroller(container)
      && (style.flexDirection === "column-reverse" || container.scrollTop < -1);
    return reversed
      ? { min: -maxDistance, max: 0 }
      : { min: 0, max: maxDistance };
  }

  function outlineScrollViewportTop(container) {
    const safeTop = Math.max(0, contentSafeBounds().top - PANEL_SAFE_MARGIN);
    if (outlineIsDocumentScroller(container)) return safeTop;
    return Math.max(safeTop, container.getBoundingClientRect().top);
  }

  function outlineScrollScale(container) {
    const layoutHeight = container.clientHeight;
    const visualHeight = container.getBoundingClientRect().height;
    if (!(layoutHeight > 0) || !(visualHeight > 0)) return 1;
    const scale = visualHeight / layoutHeight;
    return Number.isFinite(scale) && scale > 0.01 ? scale : 1;
  }

  function outlineTargetScrollTop(element, container) {
    const bounds = outlineScrollBounds(container);
    const elementTop = element.getBoundingClientRect().top;
    const targetViewportTop = outlineScrollViewportTop(container) + OUTLINE_TARGET_TOP_OFFSET;
    const delta = elementTop - targetViewportTop;
    const deltaInScrollSpace = delta / outlineScrollScale(container);
    return clamp(container.scrollTop + deltaInScrollSpace, bounds.min, bounds.max);
  }

  function outlineScheduleScrollSettle(element, container) {
    state.outlineScrollCleanup?.();
    let settleTimer = 0;
    let recheckTimer = 0;
    let finished = false;
    const cleanup = () => {
      if (settleTimer) window.clearTimeout(settleTimer);
      if (recheckTimer) window.clearTimeout(recheckTimer);
      settleTimer = 0;
      recheckTimer = 0;
      container.removeEventListener("scrollend", settle);
      container.removeEventListener("wheel", cancel);
      container.removeEventListener("pointerdown", cancel);
      if (state.outlineScrollCleanup === cancel) state.outlineScrollCleanup = null;
    };
    const cancel = () => {
      if (finished) return;
      finished = true;
      cleanup();
    };
    const correct = () => {
      if (!isCurrentRuntime() || !element.isConnected || !container.isConnected) return false;
      const targetTop = outlineTargetScrollTop(element, container);
      if (Math.abs(container.scrollTop - targetTop) > 1) container.scrollTop = targetTop;
      return true;
    };
    const finish = () => {
      if (finished) return;
      finished = true;
      cleanup();
    };
    const settle = () => {
      if (finished) return;
      container.removeEventListener("scrollend", settle);
      if (settleTimer) window.clearTimeout(settleTimer);
      settleTimer = 0;
      if (!correct()) {
        finish();
        return;
      }
      recheckTimer = window.setTimeout(() => {
        recheckTimer = 0;
        correct();
        finish();
      }, OUTLINE_SCROLL_RECHECK_MS);
    };
    state.outlineScrollCleanup = cancel;
    container.addEventListener("scrollend", settle, { once: true });
    container.addEventListener("wheel", cancel, { once: true, passive: true });
    container.addEventListener("pointerdown", cancel, { once: true, passive: true });
    settleTimer = window.setTimeout(settle, OUTLINE_SCROLL_SETTLE_MS);
  }

  function outlineScrollToElement(element) {
    const container = outlineFindScrollContainer(element);
    if (!(container instanceof Element)) return false;
    // Use one bounded destination instead of chaining scrollIntoView with a corrective scroll.
    const targetTop = outlineTargetScrollTop(element, container);
    if (!Number.isFinite(targetTop)) return false;
    if (Math.abs(container.scrollTop - targetTop) < 0.5) {
      state.outlineScrollCleanup?.();
      return true;
    }
    outlineScheduleScrollSettle(element, container);
    try {
      container.scrollTo({ top: targetTop, behavior: "smooth" });
    } catch {
      state.outlineScrollCleanup?.();
      container.scrollTop = targetTop;
    }
    return true;
  }

  function outlineScrollToEnd(fromElement) {
    const container = outlineFindScrollContainer(fromElement);
    if (!(container instanceof Element)) return false;
    state.outlineScrollCleanup?.();
    const targetTop = outlineScrollBounds(container).max;
    if (Math.abs(container.scrollTop - targetTop) < 0.5) return true;
    try {
      container.scrollTo({ top: targetTop, behavior: "smooth" });
    } catch {
      container.scrollTop = targetTop;
    }
    return true;
  }

  function outlineResolveElement(id) {
    const item = state.outlineItems.find((entry) => entry.id === id) || null;
    if (item?.el?.isConnected) return item.el;
    const marked = Array.from(document.querySelectorAll(`[${MARK_ATTR}]`))
      .find((node) => node.getAttribute(MARK_ATTR) === String(id));
    if (marked instanceof Element) {
      if (item) item.el = marked;
      return marked;
    }
    const latest = state.outlineMessage?.isConnected ? { node: state.outlineMessage } : findLatestAssistantMessage();
    const root = outlineMarkdownRoot(latest?.node);
    if (!root || !item?.text) return null;
    const kind = item.kind === "semantic" ? "semantic" : "pseudo";
    const selector = kind === "semantic" ? OUTLINE_SEMANTIC_HEADING_SELECTOR : OUTLINE_PSEUDO_HEADING_SELECTOR;
    const candidates = root.querySelectorAll(selector);
    for (const node of candidates) {
      if (kind === "pseudo" && node.closest(OUTLINE_SEMANTIC_HEADING_SELECTOR)) continue;
      const candidate = outlineHeadingCandidate(node, kind);
      if (!candidate || !outlineTitlesEquivalent(candidate.text, item.text)) continue;
      node.setAttribute(MARK_ATTR, id);
      item.el = node;
      return node;
    }
    return null;
  }

  function outlineFlash(element) {
    if (!(element instanceof Element)) return;
    element.classList.add(HIGHLIGHT_CLASS);
    if (state.flashTimer) window.clearTimeout(state.flashTimer);
    state.flashTimer = window.setTimeout(() => {
      element.classList.remove(HIGHLIGHT_CLASS);
      state.flashTimer = 0;
    }, FLASH_MS);
  }

  function outlineSetActiveTarget({ id = "", anchor = "" } = {}) {
    state.panel?.querySelectorAll("[data-outline-id],[data-outline-anchor]").forEach((button) => {
      const isActive = id
        ? button.dataset.outlineId === id
        : anchor && button.dataset.outlineAnchor === anchor;
      button.dataset.active = isActive ? "true" : "false";
      if (isActive) button.setAttribute("aria-current", "location");
      else button.removeAttribute("aria-current");
    });
  }

  function outlineJumpTo(id) {
    const element = outlineResolveElement(id);
    if (!(element instanceof Element)) return false;
    outlineSetActiveTarget({ id });
    outlineScrollToElement(element);
    outlineFlash(element);
    return true;
  }

  function outlineCurrentMessageElement() {
    if (state.outlineMessage?.isConnected) return state.outlineMessage;
    const latest = findLatestAssistantMessage();
    return latest?.node instanceof Element ? latest.node : null;
  }

  function outlineTurnStartElement(message) {
    const turn = message?.closest?.(CONVERSATION_TURN_SELECTOR)
      || (state.latestTurnAnchor?.turnNode?.isConnected ? state.latestTurnAnchor.turnNode : null);
    if (!(turn instanceof Element)) return message;
    return labeledMessageContainer(turn, "user") || turn;
  }

  function outlineJumpToAnchor(anchor) {
    const message = outlineCurrentMessageElement();
    if (!(message instanceof Element)) return false;
    if (anchor === "start") {
      const startElement = outlineTurnStartElement(message);
      if (!(startElement instanceof Element)) return false;
      outlineSetActiveTarget({ anchor });
      outlineScrollToElement(startElement);
      outlineFlash(startElement);
      return true;
    }
    if (anchor === "end") {
      outlineSetActiveTarget({ anchor });
      return outlineScrollToEnd(message);
    }
    return false;
  }

  function outlineBuild(message, sourceHash) {
    if (!message?.node) {
      outlineClearMarks();
      return { items: [], fingerprint: sourceHash || "", message: null };
    }
    const textLength = message.text.length;
    const raw = outlineCollectHeadingElements(outlineMarkdownRoot(message.node));
    const items = outlineNormalizeDisplayLevels(outlineDedupeItems(raw));
    const structuredEnough = items.length >= Math.max(MIN_OUTLINE_ITEMS, 3) && textLength >= 160;
    outlineClearMarks();
    if (textLength < MIN_OUTLINE_TEXT_LEN && !structuredEnough || items.length < MIN_OUTLINE_ITEMS) {
      return { items: [], fingerprint: `${sourceHash}|empty`, message: message.node };
    }
    outlineMarkItems(items);
    return {
      items,
      fingerprint: `${sourceHash}|${hashText(items.map((item) => `${item.level}:${item.text}`).join("|"))}`,
      message: message.node,
    };
  }

  function invalidateOutline(message = null, sourceHash = "") {
    outlineClearMarks();
    state.outlineItems = [];
    state.outlineStatus = chatBusy() ? "pending" : "idle";
    state.outlineError = "";
    state.outlineFingerprint = "";
    state.outlineSourceHash = "";
    state.outlineMessage = message?.node || null;
    if (state.activeTab === "outline" && state.panel) renderFloat({ preserveMorph: true });
  }

  // Outline refresh is keyed to the pinned latest answer, so passive scrolling cannot switch context.
  async function refreshOutline(options = {}) {
    if (!isCurrentRuntime() || !outlineEnabled()) return;
    if (state.outlineRefreshPromise) return state.outlineRefreshPromise;
    const requestContext = contextSnapshot();
    const requestEpoch = state.outlineEpoch;
    const requestCurrent = () => outlineEnabled()
      && requestEpoch === state.outlineEpoch
      && contextMatches(requestContext);
    state.outlineStatus = "pending";
    state.outlineError = "";
    if (state.activeTab === "outline") renderFloat({ preserveMorph: true });

    const task = Promise.resolve().then(() => {
      if (!requestCurrent()) return;
      const message = options.message || findLatestAssistantMessage();
      const sourceHash = options.assistantHash || hashText(message?.text || "");
      if (chatBusy()) {
        state.outlineError = "回答尚未完成，完成后再试";
        state.outlineStatus = "pending";
        scheduleScan(STREAM_IDLE_MS);
        return;
      }
      const result = outlineBuild(message, sourceHash);
      if (!requestCurrent()) return;
      state.outlineItems = result.items;
      state.outlineFingerprint = result.fingerprint;
      state.outlineSourceHash = sourceHash;
      state.outlineMessage = result.message;
      state.outlineStatus = result.items.length ? "ready" : "empty";
      state.outlineError = "";
    }).catch((error) => {
      if (!requestCurrent()) return;
      outlineClearMarks();
      state.outlineItems = [];
      state.outlineStatus = "error";
      state.outlineError = error?.message || "大纲暂不可用";
    }).finally(() => {
      if (!requestCurrent()) return;
      if (state.outlineRefreshPromise === task) state.outlineRefreshPromise = null;
      if (state.activeTab === "outline") renderFloat({ preserveMorph: true });
    });
    state.outlineRefreshPromise = task;
    return task;
  }

  function outlineHtml() {
    if (state.outlineStatus === "pending") {
      return `<div class="csw-progress" aria-label="正在整理大纲">
        <span class="csw-progress-ring" aria-hidden="true"></span>
        <span class="csw-progress-copy">
          <span class="csw-progress-title">正在整理大纲</span>
        </span>
      </div>`;
    }
    if (state.outlineStatus === "error") {
      return `<div class="csw-empty" data-kind="outline">
        <div class="csw-empty-title">${escapeHtml(outlineErrorTitle())}</div>
      </div>`;
    }
    if (!state.outlineItems.length) {
      return `<div class="csw-empty" data-kind="outline">
        <div class="csw-empty-title">暂无大纲</div>
      </div>`;
    }
    return `<div class="csw-outline-view">
      <div class="csw-outline-list" role="list">${state.outlineItems.map((item) => {
      const displayLevel = item.displayLevel ?? 0;
      const numberPrefix = item.numberPrefix || "";
      const labelText = item.labelText || item.text;
      return `
        <button class="csw-outline-row" type="button" role="listitem" data-outline-id="${escapeAttr(item.id)}" data-level="${displayLevel}" data-numbered="${numberPrefix ? "true" : "false"}" aria-label="${escapeAttr(item.text)}" style="--csw-outline-indent:${Math.min(3, displayLevel) * 12}px">
          <span class="csw-outline-heading-marker" aria-hidden="true"></span>
          <span class="csw-outline-prefix" aria-hidden="true">${escapeHtml(numberPrefix)}</span>
          <span class="csw-outline-label">${escapeHtml(labelText)}</span>
        </button>
      `;
      }).join("")}
      </div>
      <div class="csw-outline-toolbar" role="toolbar" aria-label="本轮导航">
        <button class="csw-outline-nav-button" type="button" data-outline-anchor="start" title="本轮开头" aria-label="定位到本轮开头">${iconSvg("turn-start")}</button>
        <button class="csw-outline-nav-button" type="button" data-outline-anchor="end" title="本轮结尾" aria-label="定位到本轮结尾">${iconSvg("turn-end")}</button>
      </div>
    </div>`;
  }

  function attachOutlineEvents() {
    state.panel.querySelectorAll("[data-outline-id]").forEach((button) => {
      button.addEventListener("click", () => {
        if (!outlineJumpTo(button.dataset.outlineId)) {
          state.outlineStatus = "error";
          state.outlineError = "找不到对应的小节，刷新后再试。";
          renderFloat({ preserveMorph: true });
        }
      });
    });
    state.panel.querySelectorAll("[data-outline-anchor]").forEach((button) => {
      button.addEventListener("click", () => {
        if (!outlineJumpToAnchor(button.dataset.outlineAnchor)) {
          state.outlineStatus = "error";
          state.outlineError = "找不到当前回答位置，刷新后再试。";
          renderFloat({ preserveMorph: true });
        }
      });
    });
  }

  function viewScrollTargets(body = state.panel?.querySelector(".csw-body[data-view-body]")) {
    if (!body) return [];
    const targets = [body];
    const previewScroll = body.querySelector(".csw-prompt-preview-scroll");
    if (previewScroll) targets.push(previewScroll);
    return targets;
  }

  function captureViewScroll() {
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    if (!body || body.dataset.viewBody !== state.activeTab) return null;
    const preview = body.querySelector(".csw-prompt-preview");
    const previewScroll = preview?.querySelector(".csw-prompt-preview-scroll");
    return {
      view: state.activeTab,
      top: body.scrollTop,
      preview: preview && previewScroll ? {
        index: preview.dataset.previewIndex || "",
        prompt: preview.querySelector(".csw-prompt-preview-body")?.textContent || "",
        top: previewScroll.scrollTop,
      } : null,
    };
  }

  function restoreViewScroll(snapshot) {
    if (!snapshot || snapshot.view !== state.activeTab) return;
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    if (!body || body.dataset.viewBody !== snapshot.view) return;
    const maxTop = Math.max(0, body.scrollHeight - body.clientHeight);
    body.scrollTop = clamp(snapshot.top, 0, maxTop);

    const preview = body.querySelector(".csw-prompt-preview");
    const previewScroll = preview?.querySelector(".csw-prompt-preview-scroll");
    if (!snapshot.preview || !preview || !previewScroll) return;
    const prompt = preview.querySelector(".csw-prompt-preview-body")?.textContent || "";
    if (preview.dataset.previewIndex !== snapshot.preview.index || prompt !== snapshot.preview.prompt) return;
    const previewMaxTop = Math.max(0, previewScroll.scrollHeight - previewScroll.clientHeight);
    previewScroll.scrollTop = clamp(snapshot.preview.top, 0, previewMaxTop);
  }

  function syncContentFade() {
    const popover = state.popover;
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    if (!popover || !body) return;

    const preview = body.querySelector(".csw-prompt-preview");
    const previewScroll = preview?.querySelector(".csw-prompt-preview-scroll");
    if (preview && previewScroll) {
      const previewMaxTop = Math.max(0, previewScroll.scrollHeight - previewScroll.clientHeight);
      const previewOverflowing = previewMaxTop > 2;
      const previewAtEnd = !previewOverflowing || previewScroll.scrollTop >= previewMaxTop - 2;
      preview.dataset.scrollOverflow = String(previewOverflowing);
      preview.dataset.scrollAtEnd = String(previewAtEnd);
      preview.dataset.scrollFade = String(previewOverflowing && !previewAtEnd);
    }

    const view = body.dataset.viewBody || "";
    const compressed = popover.dataset.compressed === "true";
    const eligible = compressed && (view === "next" || view === "outline");
    const scrollStates = viewScrollTargets(body).map((target) => ({
      target,
      maxTop: Math.max(0, target.scrollHeight - target.clientHeight),
    }));
    const overflowing = eligible && scrollStates.some(({ maxTop }) => maxTop > 2);
    const atEnd = !overflowing || scrollStates.every(({ target, maxTop }) => (
      maxTop <= 2 || target.scrollTop >= maxTop - 2
    ));

    popover.dataset.contentOverflow = String(overflowing);
    popover.dataset.contentAtEnd = String(atEnd);
    popover.dataset.contentFade = String(overflowing && !atEnd);
  }

  function installContentFadeTracking() {
    state.contentFadeCleanup?.();
    state.contentFadeCleanup = null;

    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    const targets = viewScrollTargets(body);
    if (!body || !targets.length) return;

    const onScroll = () => syncContentFade();
    targets.forEach((target) => target.addEventListener("scroll", onScroll, { passive: true }));

    const resizeObserver = typeof window.ResizeObserver === "function"
      ? new window.ResizeObserver(onScroll)
      : null;
    const resizeTargets = new Set();
    targets.forEach((target) => {
      resizeTargets.add(target);
      if (target.firstElementChild) resizeTargets.add(target.firstElementChild);
    });
    resizeTargets.forEach((target) => resizeObserver?.observe(target));

    state.contentFadeCleanup = () => {
      targets.forEach((target) => target.removeEventListener("scroll", onScroll));
      resizeObserver?.disconnect();
    };

    syncContentFade();
    window.requestAnimationFrame(() => {
      if (body.isConnected && state.panel?.contains(body)) syncContentFade();
    });
  }

  // Rendering preserves scroll, active view, and in-flight morph state while replacing only view content.
  function renderFloat(options = {}) {
    if (!isCurrentRuntime()) return;
    if (!options.allowDuringTransition && (state.viewTransitioning || state.morphAnimation)) {
      deferRender();
      return;
    }
    state.activeTab = normalizeActiveTab();
    const viewScroll = captureViewScroll();
    clearPromptInteractionTimers();
    cancelViewAnimation();
    installStyle();
    installFloat();
    if (!state.fab || !state.popover || !state.panel || !state.glass) return;
    syncTheme();
    normalizePromptState();
    const expressionNow = Date.now();
    const outlineExpression = usesOutlineExpression(expressionNow);
    const expression = resolveFabExpression(expressionNow);
    const expressionCount = outlineExpression ? state.outlineItems.length : state.prompts.length;
    const expressionLabel = fabExpressionLabel(expression, outlineExpression);
    const featureLabel = stepwiseEnabled() && outlineEnabled()
      ? "悬浮球"
      : stepwiseEnabled() ? "下一步" : "回答大纲";
    const hidden = expression === "hidden";
    if (hidden) {
      settleMorph(0);
    }
    state.fabExpression = expression;
    state.fab.dataset.expression = expression;
    state.fab.dataset.count = String(expressionCount);
    state.fab.title = state.open ? "收起" : `${featureLabel} · ${expressionLabel}`;
    state.fab.setAttribute("aria-label", state.open
      ? "收起"
      : expressionCount > 0 && expression === "ready"
        ? `${featureLabel} · ${expressionLabel} · ${expressionCount} ${outlineExpression ? "个章节" : "条"}`
        : `${featureLabel} · ${expressionLabel}`);
    state.fab.setAttribute("aria-expanded", String(state.open));
    state.root.dataset.hidden = String(hidden);
    state.popover.dataset.open = state.open ? "true" : "false";
    state.popover.dataset.expression = expression;
    state.popover.dataset.view = state.activeTab;
    applyPosition();

    const refreshState = refreshControlState();
    const refreshBlocked = refreshState.blocked;
    const refreshTitle = refreshState.title;
    const headExpression = state.activeTab === "settings" ? "curious" : expression;
    const tone = statusToneForView(expression);
    const paneCue = activePaneCue();
    state.panel.innerHTML = `
      <div class="csw-head">
        <div class="csw-head-side csw-head-left">
          <div class="csw-tabs csw-view-tabs" role="tablist" aria-label="悬浮球视图">
            <span class="csw-view-indicator" aria-hidden="true"></span>
            ${stepwiseEnabled() ? `<button class="csw-icon" type="button" data-view="next" data-active="${state.activeTab === "next"}" role="tab" aria-selected="${state.activeTab === "next"}" title="下一步建议" aria-label="下一步建议">${iconSvg("next")}</button>` : ""}
            ${outlineEnabled() ? `<button class="csw-icon" type="button" data-view="outline" data-active="${state.activeTab === "outline"}" role="tab" aria-selected="${state.activeTab === "outline"}" title="回答大纲" aria-label="回答大纲">${iconSvg("outline")}</button>` : ""}
          </div>
        </div>
        <button class="csw-head-face" type="button" data-action="collapse" data-expression="${escapeAttr(headExpression)}" data-tone="${tone}" title="收起" aria-label="收起">${statusStageHtml()}${sourceTrackHtml(paneCue, 32)}</button>
        <div class="csw-head-side csw-head-right">
          <button class="csw-icon" type="button" data-action="refresh" title="${escapeAttr(refreshTitle)}" aria-label="${escapeAttr(refreshTitle)}" ${refreshBlocked ? "disabled" : ""}>${iconSvg("refresh")}</button>
          <button class="csw-icon" type="button" data-action="theme" title="${escapeAttr(themeLabel())}" aria-label="${escapeAttr(themeLabel())}">${themeIcon()}</button>
          <button class="csw-icon" type="button" data-view="settings" data-active="${state.activeTab === "settings"}" aria-pressed="${state.activeTab === "settings"}" title="设置" aria-label="设置">${iconSvg("settings")}</button>
        </div>
      </div>
      <div class="csw-body" data-view-body="${state.activeTab}">
        <div class="csw-mouth-stage" data-mouth-stage="${state.activeTab}">${state.activeTab === "settings" ? settingsHtml() : state.activeTab === "outline" ? outlineHtml() : nextHtml()}</div>
      </div>
    `;
    animateViewTabSelection(options.viewIndicatorFrom ?? state.activeTab, state.activeTab);
    animateSourceCue(paneCue);
    restoreViewScroll(viewScroll);
    installContentFadeTracking();
    state.panel.querySelectorAll("[data-view]").forEach((button) => {
      button.addEventListener("click", () => {
        const nextTab = button.dataset.view || "next";
        if (nextTab === state.activeTab) return;
        void switchView(nextTab);
      });
    });
    const headFace = state.panel.querySelector("[data-action='collapse']");
    headFace?.addEventListener("click", onHeadFaceClick);
    bindGlassPointerSurface(headFace);
    state.panel.querySelector("[data-action='refresh']")?.addEventListener("click", () => void refreshCurrentView());
    state.panel.querySelector("[data-action='theme']")?.addEventListener("click", toggleCodexTheme);
    applyMaterial({ animate: false });

    if (state.activeTab === "settings") attachSettingsEvents();
    else if (state.activeTab === "outline") attachOutlineEvents();
    else attachNextEvents();
    installPanelDrag();
    syncEyeTracking();
    if (!options.preserveMorph && !state.morphAnimation) settleMorph(state.open ? 1 : 0);
  }

  function nextProgressState() {
    if (state.bridgeStatus === "pending") {
      return {
        title: "正在生成建议",
      };
    }
    if (stepwiseGenerationMode() === "manual") return null;
    if (state.scanStatus === "assistant-changed" || state.scanStatus === "assistant-settling") {
      return {
        title: "正在整理回答",
      };
    }
    if (state.scanStatus === "not-ready" && state.scanBusy) {
      return {
        title: "等待回答完成",
      };
    }
    return null;
  }

  function nextHtml() {
    const progress = nextProgressState();
    if (progress) {
      return `<div class="csw-progress" aria-label="${progress.title}">
        <span class="csw-progress-ring" aria-hidden="true"></span>
        <span class="csw-progress-copy">
          <span class="csw-progress-title">${progress.title}</span>
        </span>
      </div>`;
    }
    if (!state.prompts.length) {
      const empty = nextEmptyState();
      return `<div class="csw-empty" data-state="${escapeAttr(empty.state || "idle")}">
        <div class="csw-empty-title">${escapeHtml(empty.title)}</div>
      </div>`;
    }
    const previewIndex = clamp(Number(state.promptPreviewIndex) || 0, 0, state.prompts.length - 1);
    const previewItem = state.prompts[previewIndex];
    state.promptPreviewIndex = previewIndex;
    return `<div class="csw-next-layout">
      <div class="csw-list" data-label-only="${state.labelOnly}" aria-label="下一步建议">${state.prompts.map((item, index) => `
        <button class="csw-row" type="button" data-index="${index}" data-preview-active="${index === previewIndex}" aria-current="${index === previewIndex ? "true" : "false"}">
          <span class="csw-row-copy">
            <span class="csw-row-label">${escapeHtml(item.label || labelForPrompt(item.prompt))}</span>
            ${state.labelOnly ? "" : `<span class="csw-row-prompt">${escapeHtml(item.summary || summaryForPrompt(item.prompt))}</span>`}
          </span>
          <span class="csw-row-arrow" aria-hidden="true">›</span>
        </button>
      `).join("")}</div>
      <section class="csw-prompt-preview" data-preview-index="${previewIndex}" aria-label="建议完整内容">
        <div class="csw-prompt-preview-scroll" tabindex="0">
          <div class="csw-prompt-preview-content">
            <span class="csw-prompt-preview-kicker">${previewIndex + 1} / ${state.prompts.length}</span>
            <span class="csw-prompt-preview-title">${escapeHtml(previewItem.label || labelForPrompt(previewItem.prompt))}</span>
            <span class="csw-prompt-preview-body">${escapeHtml(previewItem.prompt)}</span>
          </div>
        </div>
      </section>
    </div>`;
  }

  function nextEmptyState() {
    if (state.bridgeError || state.bridgeStatus === "failed") return bridgeErrorPresentation();
    if (state.bridgeStatus === "ok") {
      return {
        title: "暂无建议",
        message: "",
      };
    }
    if (state.bridgeStatus === "disabled") {
      return {
        title: "功能已关闭",
        message: "",
      };
    }
    if (stepwiseGenerationMode() === "manual") {
      return {
        title: "当前为手动模式",
        message: "",
        state: "manual",
      };
    }
    return {
      title: "等待回答完成",
      message: "",
    };
  }

  function attachNextEvents() {
    state.panel.querySelectorAll(".csw-row").forEach((button) => {
      button.addEventListener("pointerenter", () => schedulePromptPreview(button));
      button.addEventListener("pointerleave", cancelScheduledPromptPreview);
      button.addEventListener("focus", () => showPromptPreview(button, true));
      button.addEventListener("click", (event) => {
        if (event.detail >= 2) {
          event.preventDefault();
          if (state.promptClickTimer) window.clearTimeout(state.promptClickTimer);
          state.promptClickTimer = 0;
          showPromptPreview(button, true);
          selectPrompt(button, promptClickSubmits(event.detail));
          return;
        }

        if (state.promptClickTimer) window.clearTimeout(state.promptClickTimer);
        const generation = state.runtimeGeneration;
        state.promptClickTimer = window.setTimeout(() => {
          state.promptClickTimer = 0;
          if (!isCurrentRuntime(generation) || !button.isConnected) return;
          showPromptPreview(button, true);
          selectPrompt(button, promptClickSubmits(1));
        }, PROMPT_CLICK_DELAY_MS);
      });
      button.addEventListener("dblclick", (event) => event.preventDefault());
    });
  }

  function clearPromptInteractionTimers() {
    if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
    if (state.promptClickTimer) window.clearTimeout(state.promptClickTimer);
    state.promptPreviewTimer = 0;
    state.promptClickTimer = 0;
  }

  function schedulePromptPreview(button) {
    if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
    state.promptPreviewTimer = 0;
    showPromptPreview(button);
  }

  function cancelScheduledPromptPreview() {
    if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
    state.promptPreviewTimer = 0;
  }

  function showPromptPreview(button, immediate = false) {
    const index = Number(button.dataset.index);
    const item = state.prompts[index];
    const preview = state.panel?.querySelector(".csw-prompt-preview");
    if (!item?.prompt || !preview) return;

    if (Number(preview.dataset.previewIndex) === index) {
      state.panel.querySelectorAll(".csw-row").forEach((row) => {
        const active = row === button;
        row.dataset.previewActive = String(active);
        row.setAttribute("aria-current", active ? "true" : "false");
      });
      preview.removeAttribute("data-switching");
      return;
    }

    const applyPreview = () => {
      if (!button.isConnected || !preview.isConnected) return;
      state.panel.querySelectorAll(".csw-row").forEach((row) => {
        const active = row === button;
        row.dataset.previewActive = String(active);
        row.setAttribute("aria-current", active ? "true" : "false");
      });
      const title = preview.querySelector(".csw-prompt-preview-title");
      const kicker = preview.querySelector(".csw-prompt-preview-kicker");
      const body = preview.querySelector(".csw-prompt-preview-body");
      const scroll = preview.querySelector(".csw-prompt-preview-scroll");
      if (kicker) kicker.textContent = `${index + 1} / ${state.prompts.length}`;
      if (title) title.textContent = item.label || labelForPrompt(item.prompt);
      if (body) body.textContent = item.prompt;
      if (scroll) scroll.scrollTop = 0;
      preview.dataset.previewIndex = String(index);
      state.promptPreviewIndex = index;
      syncContentFade();
      window.requestAnimationFrame(() => {
        preview.removeAttribute("data-switching");
        if (preview.isConnected) syncContentFade();
      });
    };

    if (immediate) {
      if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
      state.promptPreviewTimer = 0;
      preview.removeAttribute("data-switching");
      applyPreview();
      return;
    }
    const generation = state.runtimeGeneration;
    state.promptPreviewTimer = window.setTimeout(() => {
      state.promptPreviewTimer = 0;
      if (!isCurrentRuntime(generation) || !button.matches(":hover, :focus, :focus-within")) return;
      preview.dataset.switching = "true";
      applyPreview();
    }, PROMPT_PREVIEW_SWITCH_MS);
  }

  function selectPrompt(button, submit) {
    const item = state.prompts[Number(button.dataset.index)];
    if (!item?.prompt) return;
    pushDiagnostic("prompt:select", {
      submit,
      clickMode: state.promptClickMode,
      index: Number(button.dataset.index),
    });
    fillComposer(item.prompt, submit);
  }

  function promptClickSubmits(clickDetail, value = state.promptClickMode) {
    const mode = normalizePromptClickMode(value);
    if (mode === "direct") return true;
    if (mode === "fill") return false;
    return clickDetail >= 2;
  }

  function settingsModelLabel(settings) {
    if (settings && !stepwiseEnabled(settings)) {
      return outlineEnabled(settings) ? "回答大纲" : "未启用";
    }
    const raw = normalizeText(settings?.model);
    if (!raw) return settings ? "未配置" : "读取中";
    const leaf = raw.split("/").pop() || raw;
    return leaf
      .replace(/^gpt[-_:]?/i, "")
      .split(/[-_\s]+/)
      .filter(Boolean)
      .map((part) => (/^\d/.test(part) ? part : `${part.charAt(0).toUpperCase()}${part.slice(1)}`))
      .join(" ");
  }

  function settingsRuntimePresentation(settings) {
    if (!settings) return { label: "正在读取设置", tone: "busy" };
    if (!runtimeEnabled(settings)) return { label: "已关闭", tone: "idle" };
    if (stepwiseEnabled(settings) && (!settings.baseUrlConfigured || !settings.model || !settings.apiKeyConfigured)) {
      return { label: "等待配置", tone: "error" };
    }
    const expressionNow = Date.now();
    const outlineExpression = usesOutlineExpression(expressionNow);
    const expression = resolveFabExpression(expressionNow);
    const detail = (outlineExpression ? {
      idle: "等待回答",
      answering: "回答中",
      surprise: "正在整理回答",
      generating: "正在整理大纲",
      ready: `${state.outlineItems.length} 个章节已准备`,
      empty: "暂无大纲",
      error: "生成失败",
      hidden: "已关闭",
    } : {
      idle: "等待回答",
      answering: "回答中",
      surprise: "正在整理回答",
      generating: "正在生成建议",
      ready: `${state.prompts.length} 条建议已准备`,
      empty: "暂无建议",
      error: "生成失败",
      hidden: "已关闭",
    })[expression] || "等待回答";
    if (!outlineExpression && stepwiseWaitingForManualRefresh(settings)) {
      return { label: "当前为手动模式", tone: "idle" };
    }
    return { label: detail, tone: statusTone(expression) };
  }

  function settingsCommandHtml(action, icon, label, title, options = {}) {
    return `
      <button class="csw-command-button" type="button" data-action="${escapeAttr(action)}" title="${escapeAttr(title)}" aria-label="${escapeAttr(title)}" ${options.disabled ? "disabled" : ""}>
        <span class="csw-command-icon" data-busy="${options.busy === true}" aria-hidden="true">${iconSvg(icon)}</span>
        <span class="csw-command-label">${escapeHtml(label)}</span>
      </button>
    `;
  }

  function promptClickModeLabel(value = state.promptClickMode) {
    return {
      direct: "直接发送",
      hybrid: "单击填入 · 双击发送",
      fill: "仅填入",
    }[normalizePromptClickMode(value)];
  }

  function generationModeLabel(value = stepwiseGenerationMode()) {
    return normalizeGenerationMode(value) === "manual" ? "手动刷新" : "自动生成";
  }

  function nextGenerationMode(value = stepwiseGenerationMode()) {
    const index = GENERATION_MODES.indexOf(normalizeGenerationMode(value));
    return GENERATION_MODES[(index + 1) % GENERATION_MODES.length];
  }

  function nextPromptClickMode(value = state.promptClickMode) {
    const index = PROMPT_CLICK_MODES.indexOf(normalizePromptClickMode(value));
    return PROMPT_CLICK_MODES[(index + 1) % PROMPT_CLICK_MODES.length];
  }

  function generationModeButtonLabel(value = stepwiseGenerationMode()) {
    return `模式：${generationModeLabel(value)}；切换为${generationModeLabel(nextGenerationMode(value))}`;
  }

  function promptClickModeButtonLabel(value = state.promptClickMode) {
    return `点击：${promptClickModeLabel(value)}；切换为${promptClickModeLabel(nextPromptClickMode(value))}`;
  }

  function toggleGenerationMode(event) {
    event?.preventDefault();
    event?.stopPropagation();
    return setGenerationMode(nextGenerationMode());
  }

  function togglePromptClickMode(event) {
    event?.preventDefault();
    event?.stopPropagation();
    return writePromptClickMode(nextPromptClickMode());
  }

  function appearanceSettingsHtml() {
    return `
      <div class="csw-control-deck" aria-label="外观、字号与显示">
        <div class="csw-control-group">
          <span class="csw-control-label">外观</span>
          <span class="csw-control-row">
            <button class="csw-control-button" type="button" data-action="material" data-material="${state.material}" title="${escapeAttr(materialButtonLabel())}" aria-label="${escapeAttr(materialButtonLabel())}"><span data-material-value>${materialValueLabel()}</span></button>
          </span>
        </div>
        <div class="csw-control-group">
          <span class="csw-control-label" title="同时调整下一步与大纲内容字号">字号</span>
          <span class="csw-stepper" aria-label="下一步与大纲内容字号">
            <button class="csw-step-button" type="button" data-action="font-dec" title="减小字体" aria-label="减小字体" ${effectiveFontSize() <= MIN_FONT ? "disabled" : ""}>−</button>
            <span class="csw-step-value" aria-live="polite">${fontSizeLabel()}</span>
            <button class="csw-step-button" type="button" data-action="font-inc" title="增大字体" aria-label="增大字体" ${effectiveFontSize() >= MAX_FONT ? "disabled" : ""}>+</button>
          </span>
        </div>
        <div class="csw-control-group">
          <span class="csw-control-label">显示</span>
          <span class="csw-control-row">
            <button class="csw-control-button" type="button" data-action="label-only" aria-pressed="${state.labelOnly}" title="切换显示方式：${state.labelOnly ? "标题 + 摘要" : "仅标题"}"><span data-label-only-value>${state.labelOnly ? "仅标题" : "标题 + 摘要"}</span></button>
          </span>
        </div>
      </div>
    `;
  }

  function settingsHtml() {
    const settings = state.settingsLoaded ? state.settings : null;
    const runtime = settingsRuntimePresentation(settings);
    const model = settingsModelLabel(settings);
    const notice = settings ? settingsNotice(settings) : "";
    const noticeTone = /失败|错误|未配置|关闭|不可用|需要/i.test(notice) ? "warn" : "plain";
    const testing = state.settingsStatus === "正在检查连接";
    return `
      <div class="csw-settings">
        <section class="csw-settings-surface" data-loading="${!settings}" aria-label="悬浮球设置" aria-busy="${!settings}">
          <div class="csw-settings-hero">
            <div class="csw-model-pane">
              <strong class="csw-model-value" title="${escapeAttr(settings?.model || model)}">${escapeHtml(model)}</strong>
              <span class="csw-runtime-line">
                <span class="csw-runtime-dot" data-tone="${escapeAttr(runtime.tone)}" aria-hidden="true"></span>
                <span class="csw-runtime-copy">${escapeHtml(runtime.label)}</span>
              </span>
            </div>
            ${appearanceSettingsHtml()}
          </div>
          <div class="csw-settings-footer" aria-label="配置摘要与设置操作">
            <div class="csw-runtime-grid" aria-label="配置摘要">
              <div class="csw-generation-mode" data-generation-mode-control>
                <span class="csw-metric-label">模式</span>
                <button class="csw-metric-action" type="button" data-action="generation-mode" title="${escapeAttr(generationModeButtonLabel())}" aria-label="${escapeAttr(generationModeButtonLabel())}">
                  <span data-generation-mode-value>${generationModeLabel()}</span>
                </button>
              </div>
              <div class="csw-click-mode" data-prompt-click-control>
                <span class="csw-metric-label">点击</span>
                <button class="csw-metric-action" type="button" data-action="prompt-click-mode" title="${escapeAttr(promptClickModeButtonLabel())}" aria-label="${escapeAttr(promptClickModeButtonLabel())}">
                  <span data-prompt-click-mode-value>${promptClickModeLabel()}</span>
                </button>
              </div>
            </div>
            <div class="csw-command-deck" aria-label="设置操作">
              ${settingsCommandHtml("open-manager", "open-config", "配置", "在 Codex++ 中配置")}
              ${settingsCommandHtml("test-settings", testing ? "refresh" : "connection", "检查", "检查连接", { disabled: settings?.enabled !== true, busy: testing })}
            </div>
            ${notice ? `<div class="csw-settings-notice" data-tone="${noticeTone}" aria-live="polite">${escapeHtml(notice)}</div>` : ""}
          </div>
        </section>
      </div>
    `;
  }

  function settingsNotice(settings) {
    const status = state.settingsStatus || "";
    const line = statusLine(settings);
    if (!status || status === line) {
      if (stepwiseEnabled(settings) && settings.baseUrlConfigured && settings.model && settings.apiKeyConfigured) return "";
      if (outlineEnabled(settings) && !stepwiseEnabled(settings)) return "";
      return line;
    }
    return status;
  }

  function statusLine(settings) {
    if (!runtimeEnabled(settings)) return "悬浮球已关闭";
    if (!stepwiseEnabled(settings)) return "仅显示大纲";
    if (!settings.baseUrlConfigured || !settings.model) return "尚未配置服务地址或模型";
    if (!settings.apiKeyConfigured) return "尚未配置密钥";
    return `连接就绪 · ${settings.model || ""}`.replace(/\s+·\s+$/, "");
  }

  function attachSettingsEvents() {
    state.panel.querySelector("[data-action='material']")?.addEventListener("click", toggleMaterial);
    state.panel.querySelector("[data-action='label-only']")?.addEventListener("click", toggleLabelOnly);
    state.panel.querySelector("[data-action='font-dec']")?.addEventListener("click", () => bumpFontSize(-1));
    state.panel.querySelector("[data-action='font-inc']")?.addEventListener("click", () => bumpFontSize(1));
    state.panel.querySelector("[data-action='open-manager']")?.addEventListener("click", () => void openManager());
    state.panel.querySelector("[data-action='test-settings']")?.addEventListener("click", () => void testSettings());
    state.panel.querySelector("[data-action='generation-mode']")?.addEventListener("click", (event) => {
      void toggleGenerationMode(event);
    });
    state.panel.querySelector("[data-action='prompt-click-mode']")?.addEventListener("click", togglePromptClickMode);
  }

  function writePromptClickMode(value) {
    state.promptClickMode = normalizePromptClickMode(value);
    storage.set(PROMPT_CLICK_MODE_KEY, state.promptClickMode);
    const trigger = state.panel?.querySelector("[data-action='prompt-click-mode']");
    const display = trigger?.querySelector("[data-prompt-click-mode-value]");
    const label = promptClickModeButtonLabel();
    if (trigger) {
      trigger.title = label;
      trigger.setAttribute("aria-label", label);
    }
    if (display) display.textContent = promptClickModeLabel();
    return state.promptClickMode;
  }

  function updateGenerationModeControl(value = stepwiseGenerationMode(), busy = false) {
    const mode = normalizeGenerationMode(value);
    const trigger = state.panel?.querySelector("[data-action='generation-mode']");
    const display = trigger?.querySelector("[data-generation-mode-value]");
    const label = generationModeButtonLabel(mode);
    if (trigger) {
      trigger.title = label;
      trigger.setAttribute("aria-label", label);
      trigger.setAttribute("aria-busy", String(busy));
      trigger.disabled = busy;
    }
    if (display) display.textContent = generationModeLabel(mode);
  }

  async function setGenerationMode(value) {
    if (!isCurrentRuntime() || !state.settingsLoaded || !stepwiseEnabled()) return;
    const runtimeGeneration = state.runtimeGeneration;
    const previousMode = stepwiseGenerationMode();
    const nextMode = normalizeGenerationMode(value);
    if (nextMode === previousMode) return;
    const previousSettings = state.settings;
    const cancelAutoRequestImmediately = previousMode === "auto" && nextMode === "manual";
    const requestEpoch = ++settingsSyncEpoch;
    settingsRequestId += 1;
    settingsPromise = null;
    if (cancelAutoRequestImmediately) {
      applyRuntimeSettings({ ...(state.settings || {}), generationMode: nextMode });
      scheduleScan(0);
    }
    updateGenerationModeControl(nextMode, true);

    const payload = await bridgeCall("/settings/set", {
      codexAppStepwiseGenerationMode: nextMode,
    });
    if (!isCurrentRuntime(runtimeGeneration) || requestEpoch !== settingsSyncEpoch) return;
    if (payload?.error) {
      if (cancelAutoRequestImmediately) {
        applyRuntimeSettings(previousSettings);
        scheduleScan(0);
      }
      state.settingsStatus = payload.error || "模式保存失败";
      renderFloat();
      return;
    }

    pendingSettingsPatch = { ...pendingSettingsPatch, generationMode: nextMode };
    if (!cancelAutoRequestImmediately) {
      applyRuntimeSettings({ ...(state.settings || {}), generationMode: nextMode });
    }
    state.settingsStatus = statusLine(state.settings);
    updateGenerationModeControl(nextMode);
    scheduleScan(0);

    settingsPromise = null;
    await reloadSettings();
  }

  // Manager settings are the source of truth; local UI state is updated only after request identity checks.
  async function loadSettings() {
    const requestId = ++settingsRequestId;
    const requestEpoch = settingsSyncEpoch;
    const payload = await bridgeCall("/stepwise/settings", {});
    if (!isCurrentInstance()
      || requestId !== settingsRequestId
      || requestEpoch !== settingsSyncEpoch) return null;
    let shouldRender = false;
    if (payload?.settings) {
      const nextSettings = { ...payload.settings, ...pendingSettingsPatch };
      if (!Object.prototype.hasOwnProperty.call(nextSettings, "generationMode")) {
        nextSettings.generationMode = stepwiseGenerationMode();
      }
      pendingSettingsPatch = {};
      const settingsChanged = !state.settingsLoaded
        || settingsFingerprint(nextSettings) !== state.settingsFingerprint;
      state.settingsLoaded = true;
      if (settingsChanged) applyRuntimeSettings(nextSettings);
      if (runtimeEnabled(nextSettings)) {
        if (!state.runtimeActive) activateRuntime();
        if (settingsChanged) {
          state.settingsStatus = statusLine(nextSettings);
          shouldRender = true;
          scheduleScan(0);
        }
      } else if (state.runtimeActive) {
        stopRuntime();
      }
    } else {
      const nextStatus = payload?.error || "Bridge 未就绪";
      shouldRender = nextStatus !== state.settingsStatus;
      state.settingsStatus = nextStatus;
    }
    if (shouldRender && isCurrentRuntime()) renderFloat();
    return state.settings;
  }

  function reloadSettings() {
    if (!settingsPromise) {
      const request = loadSettings();
      const tracked = request.finally(() => {
        if (settingsPromise === tracked) settingsPromise = null;
      });
      settingsPromise = tracked;
    }
    return settingsPromise;
  }

  function scheduleSettingsSync(delay = SETTINGS_SYNC_INTERVAL_MS) {
    if (!isCurrentInstance()) return;
    if (state.settingsSyncTimer) window.clearTimeout(state.settingsSyncTimer);
    state.settingsSyncTimer = window.setTimeout(async () => {
      state.settingsSyncTimer = 0;
      try {
        await reloadSettings();
      } catch (error) {
        pushDiagnostic("settings:sync-error", {
          message: String(error?.message || error || "settings sync failed"),
        });
      } finally {
        scheduleSettingsSync();
      }
    }, delay);
  }

  async function ensureSettings() {
    if (state.settingsLoaded) return state.settings;
    return reloadSettings();
  }

  async function testSettings() {
    if (!isCurrentRuntime()) return;
    const generation = state.runtimeGeneration;
    state.settingsStatus = "正在检查连接";
    renderFloat();
    const payload = await bridgeCall("/stepwise/test", {});
    if (!isCurrentRuntime(generation)) return;
    const count = Array.isArray(payload?.items) ? payload.items.length : 0;
    state.settingsStatus = payload?.error || (payload?.disabled ? "功能已关闭" : `连接正常 · ${count} 条`);
    renderFloat();
  }

  async function openManager() {
    if (!isCurrentRuntime()) return;
    const generation = state.runtimeGeneration;
    state.settingsStatus = "正在打开 Codex++...";
    renderFloat();
    const payload = await bridgeCall("/manager/open-transient", {
      page: "settings",
      section: "stepwise",
    });
    if (!isCurrentRuntime(generation)) return;
    state.settingsStatus = payload?.status === "ok" ? "已打开 Codex++" : payload?.message || "打开失败";
    renderFloat();
  }

  function bridgeCall(path, payload) {
    if (typeof window[PAGE_BRIDGE] !== "function") {
      return Promise.resolve({ error: "page bridge is not installed", items: [] });
    }
    let timer = 0;
    const timeout = new Promise((resolve) => {
      timer = window.setTimeout(() => resolve({ error: "page bridge timed out", items: [] }), BRIDGE_TIMEOUT_MS);
    });
    const request = Promise.resolve(window[PAGE_BRIDGE](path, payload || {}));
    return Promise.race([request, timeout]).finally(() => window.clearTimeout(timer));
  }

  function roleFromElement(node) {
    if (!(node instanceof Element)) return "";
    const explicit = node.getAttribute("data-message-author-role");
    if (explicit) return explicit.toLowerCase();

    const text = elementText(node);
    if (/^(assistant|codex|assistant\s+said)\b/i.test(text)) return "assistant";
    if (/^(user|you)\b/i.test(text)) return "user";
    return "";
  }

  function threadRoots() {
    return Array.from(document.querySelectorAll(".thread-scroll-container"))
      .filter((node) => node instanceof HTMLElement)
      .filter((node) => visibleElement(node) && !state.root?.contains(node));
  }

  function threadRootOf(node) {
    if (!(node instanceof Element)) return null;
    return node.closest?.(".thread-scroll-container") || null;
  }

  function stablePaneKeyForRoot(root) {
    if (!(root instanceof Element)) return "";
    let current = root;
    for (let depth = 0; current && depth < 10; depth += 1, current = current.parentElement) {
      const controller = current.getAttribute("data-app-shell-tab-panel-controller");
      if (controller) return `pane:controller:${controller}`;
      const focusArea = current.getAttribute("data-app-shell-focus-area");
      if (focusArea) return `pane:focus:${focusArea}`;
      const anchorHost = current.getAttribute("data-pip-anchor-host");
      if (anchorHost) return `pane:anchor:${anchorHost === "codex-main-thread" ? "main" : anchorHost}`;
    }

    const roots = threadRoots();
    if (roots.length <= 1) return "pane:main";
    const ordered = roots
      .map((node) => ({ node, left: visibleRect(node)?.left ?? Number.POSITIVE_INFINITY }))
      .sort((left, right) => left.left - right.left);
    const index = Math.max(0, ordered.findIndex((item) => item.node === root));
    return index === 0 ? "pane:main" : `pane:secondary:${index}`;
  }

  function nodeIdentity(node, prefix = "node") {
    if (!(node instanceof Element)) return "";
    const explicit = [
      node.getAttribute("data-conversation-id"),
      node.getAttribute("data-session-id"),
      node.getAttribute("data-thread-id"),
      node.getAttribute("data-message-id"),
      node.getAttribute("data-turn-id"),
      node.id,
    ].find(Boolean);
    if (explicit) return `${prefix}:${explicit}`;
    if (!state.nodeKeys.has(node)) {
      state.nodeKeySeq += 1;
      state.nodeKeys.set(node, `${prefix}:${state.nodeKeySeq}`);
    }
    return state.nodeKeys.get(node);
  }

  function sessionIdForRoot(root) {
    if (!(root instanceof Element)) return "";

    const conversationMarkers = [
      "data-above-composer-conversation-id",
      "data-response-annotation-conversation",
    ];
    for (const attribute of conversationMarkers) {
      const marker = root.hasAttribute?.(attribute)
        ? root
        : root.querySelector?.(`[${attribute}]`);
      const value = marker?.getAttribute?.(attribute);
      if (value) return String(value);
    }

    let current = root;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      const value = [
        current.getAttribute?.("data-conversation-id"),
        current.getAttribute?.("data-session-id"),
        current.getAttribute?.("data-thread-id"),
      ].find(Boolean);
      if (value) return String(value);
    }

    // Side chats do not expose the main conversation marker; their tab ID is stable.
    current = root;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      const tabId = current.getAttribute?.("data-tab-id");
      if (tabId) return String(tabId);
    }

    const descendant = root.querySelector?.("[data-conversation-id], [data-session-id], [data-thread-id]");
    const descendantValue = [
      descendant?.getAttribute?.("data-conversation-id"),
      descendant?.getAttribute?.("data-session-id"),
      descendant?.getAttribute?.("data-thread-id"),
    ].find(Boolean);
    if (descendantValue) return String(descendantValue);
    const links = Array.from(root.querySelectorAll("a[href*='/c/'], a[href*='/conversation/']"));
    for (const link of links) {
      const match = String(link.getAttribute("href") || "").match(/\/(?:c|conversation)\/([^/?#]+)/i);
      if (match?.[1]) return match[1];
    }
    const routeMatch = location.pathname.match(/\/(?:c|conversation)\/([^/?#]+)/i);
    const paneKey = stablePaneKeyForRoot(root);
    if ((paneKey === "pane:anchor:main" || paneKey === "pane:main") && routeMatch?.[1]) return routeMatch[1];
    if (threadRoots().length <= 1 && routeMatch?.[1]) return routeMatch[1];
    return paneKey;
  }

  function assistantMessageId(message) {
    if (message?.turnKey) return `turn:${message.turnKey}`;
    const node = message?.node;
    if (!(node instanceof Element)) return "";
    return nodeIdentity(node, "assistant");
  }

  function resetContextContent() {
    state.stepwiseEpoch += 1;
    state.latestTurnAnchor = null;
    state.lastAssistantHash = "";
    state.lastAssistantAt = 0;
    state.currentHash = "";
    state.prompts = [];
    state.promptPreviewIndex = 0;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = stepwiseGenerationMode();
    state.bridgeStatus = "idle";
    state.bridgeError = "";
    state.promptContext = null;
    state.outlineRefreshPromise = null;
    invalidateOutline();
  }

  // Conversation tracking pins one thread and latest completed turn independently of virtualized DOM mounts.
  function installContextTracking() {
    if (!state.pointerHandler) {
      state.pointerHandler = (event) => {
        if (pinThreadFromTarget(event.target, "pointer")) scheduleScan(0);
      };
      document.addEventListener("pointerdown", state.pointerHandler, true);
    }
    if (!state.focusHandler) {
      state.focusHandler = (event) => {
        if (pinThreadFromTarget(event.target, "focus")) scheduleScan(0);
      };
      document.addEventListener("focusin", state.focusHandler, true);
    }
    if (!state.selectionHandler) {
      state.selectionHandler = () => {
        const selection = document.getSelection();
        const node = selection?.anchorNode;
        const target = node instanceof Element ? node : node?.parentElement;
        if (target && pinThreadFromTarget(target, "selection")) scheduleScan(0);
      };
      document.addEventListener("selectionchange", state.selectionHandler, true);
    }
  }

  function removeContextTracking() {
    if (state.pointerHandler) document.removeEventListener("pointerdown", state.pointerHandler, true);
    if (state.focusHandler) document.removeEventListener("focusin", state.focusHandler, true);
    if (state.selectionHandler) document.removeEventListener("selectionchange", state.selectionHandler, true);
    state.pointerHandler = null;
    state.focusHandler = null;
    state.selectionHandler = null;
  }

  function setActiveThreadRoot(root, reason = "resolve") {
    if (!(root instanceof HTMLElement) || !root.isConnected) return false;
    const paneKey = stablePaneKeyForRoot(root);
    const sessionId = sessionIdForRoot(root);
    const previous = state.activeContext;
    const sessionChanged = previous.sessionId !== sessionId;
    const identityChanged = previous.paneKey !== paneKey || sessionChanged;
    if (!identityChanged && previous.paneRoot === root) return false;
    if (!identityChanged) {
      state.activeContext = {
        ...previous,
        paneRoot: root,
      };
      if (state.pinnedPaneKey === paneKey && state.pinnedSessionId === sessionId) {
        state.pinnedThreadRoot = root;
      }
      pushDiagnostic("context:rebound", {
        reason,
        paneKey,
        sessionId,
        generation: state.activeContext.generation,
        paneCount: threadRoots().length,
        paneRect: rectSummary(root),
      });
      renderFloat();
      return true;
    }
    state.activeContext = {
      paneRoot: root,
      paneKey,
      sessionId,
      assistantMessageId: "",
      generation: previous.generation + 1,
    };
    if (state.pinnedPaneKey === paneKey && state.pinnedThreadRoot === root) {
      state.pinnedSessionId = sessionId;
    }
    resetContextContent();
    pushDiagnostic("context:changed", {
      reason,
      paneKey,
      sessionId,
      sessionChanged,
      generation: state.activeContext.generation,
      paneCount: threadRoots().length,
      paneRect: rectSummary(root),
    });
    renderFloat();
    return true;
  }

  function contextSnapshot() {
    return {
      runtimeGeneration: state.runtimeGeneration,
      generation: state.activeContext.generation,
      paneKey: state.activeContext.paneKey,
      sessionId: state.activeContext.sessionId,
      assistantMessageId: state.activeContext.assistantMessageId,
    };
  }

  function contextMatches(snapshot) {
    if (!snapshot) return false;
    if (!isCurrentRuntime(snapshot.runtimeGeneration)) return false;
    const current = state.activeContext;
    return snapshot.generation === current.generation &&
      snapshot.paneKey === current.paneKey &&
      snapshot.sessionId === current.sessionId &&
      snapshot.assistantMessageId === current.assistantMessageId;
  }

  function pinThreadFromTarget(target, reason) {
    if (!(target instanceof Element) || state.root?.contains(target)) return false;
    const root = threadRootOf(target);
    if (!root) return false;
    state.pinnedPaneKey = stablePaneKeyForRoot(root);
    state.pinnedSessionId = sessionIdForRoot(root);
    state.pinnedThreadRoot = root;
    state.pinnedThreadAt = Date.now();
    state.threadActivity.set(root, state.pinnedThreadAt);
    return setActiveThreadRoot(root, reason);
  }

  function rootMatchesContext(root, paneKey, sessionId) {
    if (!(root instanceof Element) || !paneKey) return false;
    if (stablePaneKeyForRoot(root) !== paneKey) return false;
    return !sessionId || sessionIdForRoot(root) === sessionId;
  }

  function rootForContext(paneKey, sessionId, roots = threadRoots()) {
    if (!paneKey) return null;
    return roots.find((root) => rootMatchesContext(root, paneKey, sessionId))
      || roots.find((root) => stablePaneKeyForRoot(root) === paneKey)
      || null;
  }

  function resolveActiveThreadRoot() {
    const roots = threadRoots();
    if (!roots.length) {
      state.activeContext.paneRoot = null;
      return null;
    }
    const current = state.activeContext.paneRoot;
    if (current?.isConnected && roots.includes(current)) {
      const sessionId = sessionIdForRoot(current);
      if (sessionId !== state.activeContext.sessionId) setActiveThreadRoot(current, "session-change");
      return current;
    }
    const pinned = rootForContext(state.pinnedPaneKey, state.pinnedSessionId, roots)
      || (state.pinnedThreadRoot?.isConnected && roots.includes(state.pinnedThreadRoot) ? state.pinnedThreadRoot : null);
    if (pinned) {
      state.pinnedThreadRoot = pinned;
      setActiveThreadRoot(pinned, "pinned");
      return pinned;
    }
    const rebound = rootForContext(state.activeContext.paneKey, state.activeContext.sessionId, roots);
    if (rebound) {
      setActiveThreadRoot(rebound, "active-rebound");
      return rebound;
    }
    const focused = threadRootOf(document.activeElement);
    if (focused && roots.includes(focused)) {
      setActiveThreadRoot(focused, "focus");
      return focused;
    }
    const fallback = roots[0];
    setActiveThreadRoot(fallback, roots.length === 1 ? "single-pane" : "fallback");
    return fallback;
  }

  function activePaneCue() {
    const roots = threadRoots();
    const active = state.activeContext.paneRoot;
    const centerCue = paneCueForTrack({ direction: "single", angle: null }, CHIP_HEIGHT);
    if (roots.length < 2 || !active?.isConnected) return centerCue;
    const activeRect = visibleRect(active);
    if (!activeRect) return centerCue;
    const rects = roots.map(visibleRect).filter(Boolean);
    if (rects.length < 2) return centerCue;
    const bounds = {
      left: Math.min(...rects.map((rect) => rect.left)),
      top: Math.min(...rects.map((rect) => rect.top)),
      right: Math.max(...rects.map((rect) => rect.right)),
      bottom: Math.max(...rects.map((rect) => rect.bottom)),
    };
    const boundsWidth = Math.max(1, bounds.right - bounds.left);
    const boundsHeight = Math.max(1, bounds.bottom - bounds.top);
    const offsetX = ((activeRect.left + activeRect.width / 2) - (bounds.left + boundsWidth / 2)) / (boundsWidth / 2);
    const offsetY = ((activeRect.top + activeRect.height / 2) - (bounds.top + boundsHeight / 2)) / (boundsHeight / 2);
    if (Math.abs(offsetX) < 0.01 && Math.abs(offsetY) < 0.01) return centerCue;
    const angle = Math.atan2(offsetY, offsetX);
    const direction = Math.abs(offsetX) >= Math.abs(offsetY)
      ? (offsetX < 0 ? "left" : "right")
      : (offsetY < 0 ? "top" : "bottom");
    return paneCueForTrack({ direction, angle }, CHIP_HEIGHT);
  }

  function paneCueForTrack(paneCue, trackHeight = CHIP_HEIGHT) {
    if (paneCue.direction === "single" || !Number.isFinite(paneCue.angle)) {
      return { direction: "single", angle: null, x: CHIP_WIDTH / 2, y: trackHeight / 2 };
    }
    const point = capsuleBoundaryPoint(paneCue.angle, CHIP_WIDTH, trackHeight);
    return {
      direction: paneCue.direction,
      angle: paneCue.angle,
      x: point.x,
      y: point.y,
    };
  }

  function capsuleBoundaryPoint(angle, width, height) {
    const halfWidth = width / 2;
    const halfHeight = height / 2;
    const radius = halfHeight;
    const innerHalfWidth = Math.max(0, halfWidth - radius);
    const cosine = Math.cos(angle);
    const sine = Math.sin(angle);
    let inside = 0;
    let outside = Math.hypot(halfWidth, halfHeight) + radius;
    for (let index = 0; index < 24; index += 1) {
      const distance = (inside + outside) / 2;
      const x = Math.abs(cosine * distance) - innerHalfWidth;
      const y = Math.abs(sine * distance);
      const outsideX = Math.max(x, 0);
      const outsideY = Math.max(y, 0);
      const signedDistance = Math.hypot(outsideX, outsideY) + Math.min(Math.max(x, y), 0) - radius;
      if (signedDistance <= 0) inside = distance;
      else outside = distance;
    }
    return {
      x: Math.round((halfWidth + cosine * inside) * 10) / 10,
      y: Math.round((halfHeight + sine * inside) * 10) / 10,
    };
  }

  function chatRoot() {
    return resolveActiveThreadRoot();
  }

  function elementCenter(rect) {
    if (!rect) return { x: 0, y: 0 };
    return {
      x: rect.left + rect.width / 2,
      y: rect.top + rect.height / 2,
    };
  }

  function horizontalOverlapRatio(left, right) {
    if (!left || !right) return 0;
    const overlap = Math.max(0, Math.min(left.right, right.right) - Math.max(left.left, right.left));
    return overlap / Math.max(1, Math.min(left.width, right.width));
  }

  function ignoredComposerContainer(node, targetRoot = null) {
    if (!(node instanceof Element)) return true;
    if (state.root?.contains(node)) return true;
    const blockedAncestor = node.closest([
      `[${ROOT_ATTR}="true"]`,
      `[${PAYLOAD_ATTR}="true"]`,
      "nav",
      "[role='dialog']",
      "[aria-modal='true']",
      "[role='menu']",
      "[role='listbox']",
    ].join(","));
    if (blockedAncestor) return true;

    const activeRoot = targetRoot || chatRoot();
    if (activeRoot?.contains(node)) return false;

    const nodeAside = node.closest("aside");
    if (!nodeAside) return false;

    const activeAside = activeRoot?.closest("aside");
    return !(activeAside && nodeAside === activeAside);
  }

  function composerCandidateScore(node, rootRect, targetRoot = null) {
    const rect = visibleRect(node);
    if (!rect || !rootRect) return -Infinity;
    if (rect.width < 120 || rect.height < 20) return -Infinity;
    if (rect.bottom < window.innerHeight * 0.35) return -Infinity;
    if (ignoredComposerContainer(node, targetRoot)) return -Infinity;

    const overlap = horizontalOverlapRatio(rect, rootRect);
    const center = elementCenter(rect);
    const rootCenter = elementCenter(rootRect);
    const centerDrift = Math.abs(center.x - rootCenter.x) / Math.max(1, rootRect.width);
    const centerInsideRoot = center.x >= rootRect.left - 24 && center.x <= rootRect.right + 24;
    if (overlap < 0.45 && !centerInsideRoot) return -Infinity;

    const lowerScreen = rect.bottom / Math.max(1, window.innerHeight);
    const widthMatch = Math.min(rect.width, rootRect.width) / Math.max(1, Math.max(rect.width, rootRect.width));
    return overlap * 100 + lowerScreen * 24 + widthMatch * 18 - centerDrift * 48;
  }

  function mainComposerCandidate(candidates, targetRoot = null) {
    const root = targetRoot || chatRoot();
    const rootRect = visibleRect(root);
    const ranked = candidates
      .map((node) => ({ node, score: composerCandidateScore(node, rootRect, root) }))
      .filter((item) => Number.isFinite(item.score))
      .sort((left, right) => right.score - left.score);
    if (ranked[0]?.node) return ranked[0].node;

    if (targetRoot || threadRoots().length > 1) return null;

    const fallback = candidates
      .map((node) => ({ node, score: globalComposerCandidateScore(node) }))
      .filter((item) => Number.isFinite(item.score))
      .sort((left, right) => right.score - left.score)[0];
    if (fallback?.node) {
      pushDiagnostic("composer:global-fallback", {
        score: fallback.score,
        targetTag: fallback.node.tagName || "",
        targetRole: fallback.node.getAttribute?.("role") || "",
        targetClass: String(fallback.node.className || "").slice(0, 120),
        targetRect: rectSummary(fallback.node),
      });
    }
    return fallback?.node || null;
  }

  function globalComposerCandidateScore(node) {
    const rect = visibleRect(node);
    if (!rect || rect.width < 120 || rect.height < 20) return -Infinity;
    if (rect.bottom < window.innerHeight * 0.35 || ignoredComposerContainer(node)) return -Infinity;

    const label = normalizeText([
      node.getAttribute?.("aria-label"),
      node.getAttribute?.("placeholder"),
      node.getAttribute?.("data-placeholder"),
    ].filter(Boolean).join(" "));
    if (/search|find|查找|搜索/i.test(label)) return -Infinity;

    let score = rect.bottom / Math.max(1, window.innerHeight) * 40;
    score += Math.min(rect.width / Math.max(1, window.innerWidth), 1) * 20;
    if (node.matches?.("div.ProseMirror")) score += 160;
    if (node instanceof HTMLTextAreaElement) score += 130;
    if (node.getAttribute?.("role") === "textbox") score += 90;
    if (node.isContentEditable) score += 70;
    if (/message|prompt|send|ask|消息|输入|提问|发送/i.test(label)) score += 60;
    return score;
  }

  function composerCandidates(targetRoot = null) {
    const scope = targetRoot || document;
    return Array.from(
      scope.querySelectorAll(
        [
          "textarea",
          "[contenteditable='true']",
          "[role='textbox']",
          "div.ProseMirror",
        ].join(",")
      )
    ).filter((node) => {
      if (!(node instanceof HTMLElement)) return false;
      const rect = node.getBoundingClientRect();
      if (rect.width < 120 || rect.height < 20) return false;
      if (rect.bottom < window.innerHeight * 0.35) return false;
      if (targetRoot && threadRootOf(node) !== targetRoot) return false;
      if (ignoredComposerContainer(node, targetRoot)) return false;
      return true;
    });
  }

  function buttonLabel(node) {
    return normalizeText(node.getAttribute("aria-label") || node.getAttribute("title") || node.textContent || "");
  }

  function sendButtonLabel(label) {
    return /^(send message|send|add to queue|发送消息|发送|提交|加入队列|添加到队列)$/i.test(label);
  }

  function stopButtonLabel(label) {
    return /^(stop|停止)$/i.test(label);
  }

  function iconPathData(node) {
    return Array.from(node.querySelectorAll?.("svg path") || [])
      .map((path) => path.getAttribute("d") || "")
      .join("\n");
  }

  function stopButtonIcon(node) {
    const data = iconPathData(node);
    return /H14\.25C14\.9404 4\.5 15\.5 5\.05964 15\.5 5\.75V14\.25C15\.5 14\.9404/.test(data);
  }

  function stopButton(node) {
    return stopButtonLabel(buttonLabel(node)) || stopButtonIcon(node);
  }

  function disabledButton(node) {
    return Boolean(node.disabled || node.getAttribute("aria-disabled") === "true" || node.dataset.disabled === "true");
  }

  function submitButtonCandidate(button, containerRect) {
    const label = buttonLabel(button);
    if (stopButton(button)) return false;
    if (sendButtonLabel(label)) return true;
    if (label) return false;

    const rect = visibleRect(button);
    if (!rect || !containerRect) return false;
    const className = String(button.className || "");
    const compactIcon = rect.width >= 24 && rect.width <= 48 && rect.height >= 24 && rect.height <= 48;
    const composerIcon = className.includes("size-token-button-composer") || className.includes("bg-token-foreground");
    const lowerRight = rect.left > containerRect.left + containerRect.width * 0.58 &&
      rect.top > containerRect.top + containerRect.height * 0.42;
    return compactIcon && composerIcon && lowerRight;
  }

  function nearbySubmitButton(target, options = {}) {
    const includeDisabled = options.includeDisabled === true;
    const targetRoot = options.root || threadRootOf(target);
    let current = target?.parentElement || null;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      if (current === document.body || current === document.documentElement) break;
      if (state.root?.contains(current)) return null;
      if (targetRoot && !targetRoot.contains(current)) break;
      const buttons = Array.from(current.querySelectorAll("button,[role='button']"))
        .filter((node) => node instanceof HTMLElement && !state.root?.contains(node) && visibleElement(node) && (includeDisabled || !disabledButton(node)));

      const labeled = buttons.find((button) => sendButtonLabel(buttonLabel(button)));
      if (labeled) return labeled;

      const rect = visibleRect(current);
      if (rect && rect.width > 260 && rect.height > 52) {
        const lowerRight = buttons
          .filter((button) => !stopButton(button))
          .filter((button) => submitButtonCandidate(button, rect))
          .sort((a, b) => b.getBoundingClientRect().right - a.getBoundingClientRect().right);
        if (lowerRight.length) return lowerRight[0];
      }
    }
    return null;
  }

  function chatSurfaceReady() {
    if (!chatRoot()) return false;
    return !chatBusy();
  }

  function chatBusy() {
    const root = chatRoot();
    if (!root) return false;

    return Array.from(root.querySelectorAll("button,[role='button']")).some((node) => {
      if (!visibleElement(node)) return false;
      const label = normalizeText(node.getAttribute("aria-label") || node.textContent || "");
      return /^(停止|stop)$/i.test(label);
    });
  }

  function setScanStatus(status, details = {}) {
    const key = `${status}:${JSON.stringify(details)}`;
    state.scanStatus = status;
    state.scanBusy = status === "manual-refresh-busy" || Boolean(details.busy);
    if (state.lastScanStatus === key) return false;
    state.lastScanStatus = key;
    pushDiagnostic(`scan:${status}`, details);
    return true;
  }

  function composerBusy(target, options = {}) {
    const targetRoot = options.root || threadRootOf(target);
    let hasStopButton = false;
    let current = target?.parentElement || null;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      if (current === document.body || current === document.documentElement) break;
      if (state.root?.contains(current)) return false;
      if (targetRoot && !targetRoot.contains(current)) break;
      const buttons = Array.from(current.querySelectorAll("button,[role='button']"))
        .filter((node) => node instanceof HTMLElement && visibleElement(node));
      if (buttons.some((node) => !disabledButton(node) && sendButtonLabel(buttonLabel(node)))) return false;
      if (buttons.some((node) => stopButton(node))) hasStopButton = true;
    }
    return hasStopButton;
  }

  // Message discovery tolerates ChatGPT's changing DOM while preferring semantic role and action-row signals.
  function messageCandidates() {
    const root = chatRoot();
    if (!root) return [];

    const selectors = [
      "[data-message-author-role]",
      "[data-thread-find-target]",
      "[data-testid*='message' i]",
      "[data-test-id*='message' i]",
      "article",
    ].join(",");

    return Array.from(root.querySelectorAll(selectors))
      .map((node) => ({
        node,
        role: roleFromElement(node),
        text: elementText(node),
      }))
      .filter((item) => item.text.length > 8);
  }

  function actionButton(node) {
    const label = normalizeText(node.getAttribute("aria-label") || node.textContent || "");
    return /^(复制|喜欢|不喜欢|从此处开始分叉|挂钩|copy|like|dislike|fork)/i.test(label);
  }

  function classTokenMatch(node, token) {
    return node instanceof Element && Array.from(node.classList || []).some((className) => className === token);
  }

  function assistantBubbleCandidates() {
    const root = chatRoot();
    if (!root) return [];

    return Array.from(root.querySelectorAll(".group.flex.min-w-0.flex-col"))
      .filter((node) => {
        if (!(node instanceof HTMLElement)) return false;
        if (state.root?.contains(node)) return false;
        if (classTokenMatch(node, "items-end")) return false;
        const text = directText(node);
        if (text.length < 24 || text.length > MAX_TEXT_LENGTH) return false;
        return true;
      })
      .map((node) => ({
        node,
        role: "assistant",
        text: elementText(node),
      }));
  }

  function roleFromMessageLabel(label) {
    const text = normalizeText(label?.textContent || "");
    if (/^(你说|you said|user)\s*[:：]?$/i.test(text)) return "user";
    if (/^(ChatGPT|assistant|codex)(?:\s+说|\s+said)?\s*[:：]?$/i.test(text)) return "assistant";
    return "";
  }

  function labeledMessageContainer(turn, role) {
    if (!(turn instanceof Element)) return null;
    const labels = Array.from(turn.querySelectorAll("h4.sr-only"));
    for (let index = labels.length - 1; index >= 0; index -= 1) {
      const label = labels[index];
      if (roleFromMessageLabel(label) !== role) continue;
      const container = label.parentElement;
      if (!(container instanceof Element)) continue;
      if (role === "user" && !classTokenMatch(container, "items-end")) continue;
      if (role === "assistant" && !classTokenMatch(container, "group")) continue;
      return container;
    }
    return null;
  }

  function labeledMessageText(container) {
    if (!(container instanceof Element)) return "";
    const clone = stripOwnUi(container.cloneNode(true));
    clone.querySelectorAll?.("h4.sr-only,button,[role='button'],svg").forEach((item) => item.remove());
    return normalizeText(clone.textContent || "");
  }

  function conversationTurn(turn) {
    if (!(turn instanceof Element)) return null;
    const turnKey = normalizeText(turn.getAttribute("data-content-search-turn-key") || "");
    const userNode = labeledMessageContainer(turn, "user");
    const assistantNode = labeledMessageContainer(turn, "assistant");
    const userText = labeledMessageText(userNode);
    const assistantText = labeledMessageText(assistantNode);
    return {
      node: turn,
      turnKey,
      userText,
      assistantMessage: assistantText.length > 8 ? {
        node: assistantNode,
        role: "assistant",
        text: assistantText,
        turnKey,
      } : null,
    };
  }

  function conversationTurns() {
    const root = chatRoot();
    if (!root) return [];
    return Array.from(root.querySelectorAll(CONVERSATION_TURN_SELECTOR))
      .map(conversationTurn)
      .filter(Boolean);
  }

  function compareConversationTurnKeys(left, right) {
    if (left === right) return 0;
    return left < right ? -1 : 1;
  }

  function latestConversationTurnByKey(turns) {
    return turns.reduce((latest, turn) => {
      if (!turn?.turnKey) return latest;
      if (!latest || compareConversationTurnKeys(latest.turnKey, turn.turnKey) < 0) return turn;
      return latest;
    }, null);
  }

  function nextLatestTurnAnchor(previous, turns, sessionId) {
    const mounted = latestConversationTurnByKey(turns);
    if (!mounted) return previous;
    const sameSession = Boolean(sessionId) && previous?.sessionId === sessionId;
    if (sameSession && compareConversationTurnKeys(mounted.turnKey, previous.turnKey) < 0) return previous;

    const sameTurn = sameSession && previous?.turnKey === mounted.turnKey;
    const assistant = mounted.assistantMessage;
    return {
      sessionId,
      turnKey: mounted.turnKey,
      userText: mounted.userText || (sameTurn ? previous.userText : ""),
      assistantText: assistant?.text || (sameTurn ? previous.assistantText : ""),
      turnNode: mounted.node || (sameTurn ? previous.turnNode : null),
      assistantNode: assistant?.node || (sameTurn ? previous.assistantNode : null),
    };
  }

  function assistantMessageFromTurnAnchor(anchor) {
    if (!anchor?.assistantText || anchor.assistantText.length <= 8) return null;
    return {
      node: anchor.assistantNode,
      role: "assistant",
      text: anchor.assistantText,
      turnKey: anchor.turnKey,
      userText: anchor.userText,
      turnNode: anchor.turnNode,
    };
  }

  function updateLatestTurnAnchor(turns) {
    state.latestTurnAnchor = nextLatestTurnAnchor(
      state.latestTurnAnchor,
      turns,
      state.activeContext.sessionId,
    );
    return state.latestTurnAnchor;
  }

  function latestMessageByDocumentOrder(candidates) {
    return candidates
      .filter((item) => item?.node instanceof Node && item.text?.length > 8)
      .sort((left, right) => {
        if (left.node === right.node) return 0;
        const position = left.node.compareDocumentPosition(right.node);
        if (position & Node.DOCUMENT_POSITION_FOLLOWING) return -1;
        if (position & Node.DOCUMENT_POSITION_PRECEDING) return 1;
        if (left.node.contains(right.node)) return -1;
        if (right.node.contains(left.node)) return 1;
        return 0;
      })
      .at(-1) || null;
  }

  function actionRowForMessage(root) {
    const buttons = Array.from(root.querySelectorAll("button,[role='button']")).filter(actionButton);
    for (const button of buttons) {
      let current = button.parentElement;
      for (let depth = 0; current && depth < 5; depth += 1, current = current.parentElement) {
        const rect = visibleRect(current);
        if (!rect || rect.height > 96) continue;
        const count = Array.from(current.querySelectorAll("button,[role='button']")).filter(actionButton).length;
        if (count >= 2) return current;
      }
    }
    return null;
  }

  function containsActionRow(node) {
    return Boolean(node && actionRowForMessage(node));
  }

  function assistantContainerForActionRow(actionRow) {
    let current = actionRow?.parentElement;

    for (let depth = 0; current && depth < 7; depth += 1, current = current.parentElement) {
      const text = directText(current);
      if (text.length < 24) continue;
      if (text.length > MAX_TEXT_LENGTH) continue;
      if (!containsActionRow(current)) continue;
      return current;
    }

    return null;
  }

  function allActionRows() {
    const root = chatRoot();
    if (!root) return [];

    const rows = [];
    const seen = new Set();
    const buttons = Array.from(root.querySelectorAll("button,[role='button']")).filter(actionButton);

    for (const button of buttons) {
      let current = button.parentElement;
      for (let depth = 0; current && depth < 5; depth += 1, current = current.parentElement) {
        if (seen.has(current)) continue;
        if (!visibleElement(current)) continue;
        const rect = visibleRect(current);
        if (!rect || rect.height > 96) continue;
        const count = Array.from(current.querySelectorAll("button,[role='button']")).filter(actionButton).length;
        if (count < 2) continue;
        seen.add(current);
        rows.push(current);
        break;
      }
    }

    return rows;
  }

  function findLatestAssistantMessage() {
    const turns = conversationTurns();
    if (turns.length || state.latestTurnAnchor) {
      return assistantMessageFromTurnAnchor(updateLatestTurnAnchor(turns));
    }

    const candidates = [];
    const rows = allActionRows();
    for (let index = 0; index < rows.length; index += 1) {
      const node = assistantContainerForActionRow(rows[index]);
      const text = elementText(node);
      if (text.length > 8) candidates.push({ node, role: "assistant", text });
    }

    candidates.push(...messageCandidates().filter((item) => item.role === "assistant"));
    candidates.push(...assistantBubbleCandidates());
    return latestMessageByDocumentOrder(candidates);
  }

  function findPreviousUserText(message) {
    const snapshotUserText = normalizeText(message?.userText || "");
    if (snapshotUserText) return shortText(snapshotUserText, 2000);

    const assistantNode = message?.node || message;
    const turn = assistantNode?.closest?.(CONVERSATION_TURN_SELECTOR);
    const turnUserText = conversationTurn(turn)?.userText || "";
    if (turnUserText) return shortText(turnUserText, 2000);

    const candidates = messageCandidates();
    const before = candidates.filter((item) => {
      if (item.node === assistantNode) return false;
      if (!(item.node instanceof Node) || !(assistantNode instanceof Node)) return false;
      return Boolean(item.node.compareDocumentPosition(assistantNode) & Node.DOCUMENT_POSITION_FOLLOWING);
    });

    for (let cursor = before.length - 1; cursor >= 0; cursor -= 1) {
      const item = before[cursor];
      if (item.role === "user") return shortText(item.text, 2000);
      if (/^(user|you)\b/i.test(item.text)) return shortText(item.text, 2000);
    }
    return "";
  }

  function hideStepwisePayload(root) {
    if (!(root instanceof Element)) return;

    const blocks = Array.from(root.querySelectorAll("pre, code")).filter((node) => {
      if (!(node instanceof Element)) return false;
      return /"codex_stepwise"\s*:\s*true/.test(node.textContent || "");
    });

    for (const block of blocks) {
      const container = block.closest("[class*='_codeBlock_'], pre") || block;
      container.setAttribute(PAYLOAD_ATTR, "true");
    }
  }

  function clearStepwisePayloadMarks() {
    document.querySelectorAll(`[${PAYLOAD_ATTR}]`).forEach((node) => {
      node.removeAttribute(PAYLOAD_ATTR);
    });
  }

  function uniquePrompts(items) {
    const seen = new Set();
    const result = [];
    const maxItems = configuredMaxPromptItems();
    for (const item of Array.isArray(items) ? items : []) {
      const prompt = normalizeText(typeof item === "string" ? item : item.prompt);
      const dedupeKey = prompt.replace(/\s+/g, " ");
      if (!prompt || seen.has(dedupeKey)) continue;
      seen.add(dedupeKey);
      result.push({
        label: leadingPromptText(typeof item === "string" ? labelForPrompt(prompt) : item.label || labelForPrompt(prompt), 36),
        summary: leadingPromptText(
          typeof item === "string" ? summaryForPrompt(prompt) : item.summary || summaryForPrompt(prompt),
          MAX_PROMPT_SUMMARY_LENGTH,
        ),
        prompt,
      });
      if (result.length >= maxItems) break;
    }
    return result;
  }

  function normalizePromptState(items = state.prompts) {
    const normalized = uniquePrompts(items);
    state.prompts = normalized;
    state.promptPreviewIndex = normalized.length
      ? clamp(Number(state.promptPreviewIndex) || 0, 0, normalized.length - 1)
      : 0;
    return normalized;
  }

  function leadingPromptText(value, limit) {
    const characters = Array.from(normalizeText(value).replace(/\s+/g, " "));
    if (characters.length <= limit) return characters.join("");
    return `${characters.slice(0, Math.max(0, limit - 1)).join("").trimEnd()}…`;
  }

  function summaryForPrompt(prompt) {
    return leadingPromptText(prompt, MAX_PROMPT_SUMMARY_LENGTH);
  }

  function labelForPrompt(prompt) {
    const text = normalizeText(prompt);
    const rules = [
      [/diff|风险分级|改动.*总结/i, "查看 diff"],
      [/commit|提交/i, "整理 commit"],
      [/截图验证|遮挡|浮球|面板/i, "验证界面"],
      [/设置|配置|Bridge|API/i, "检查配置"],
      [/Codex\+\+|用户脚本|reload|生效/i, "检查脚本"],
      [/只读验证|确认.*生效|验证步骤/i, "验证生效"],
      [/错误|失败|最小复现|排查/i, "继续排查"],
      [/P0|P1|P2|执行顺序/i, "分级排序"],
      [/维护成本|长期稳定性|审查/i, "重新审查"],
      [/文件路径|当前状态|继续追踪/i, "列出路径"],
      [/下一步|改哪些文件/i, "继续下一步"],
      [/遗漏的风险|回滚方式/i, "风险回滚"],
    ];

    for (const [pattern, label] of rules) {
      if (pattern.test(text)) return label;
    }

    return text
      .replace(/^(帮我|请|把|给我|继续|检查|执行一次|基于刚才的)/, "")
      .replace(/[，。,.].*$/, "")
      .trim()
      .slice(0, 10) || "继续";
  }

  // Stepwise payload parsing accepts the backend's strict JSON contract and legacy embedded payload shapes.
  function parseStepwiseJson(text) {
    const blocks = Array.from(text.matchAll(/```(?:json)?\s*([\s\S]*?)```/gi))
      .map((match) => match[1])
      .filter((block) => /"codex_stepwise"\s*:\s*true/.test(block));

    for (const block of blocks.reverse()) {
      const parsed = parsePayloadCandidate(block);
      if (parsed) return parsed;
    }
    return parsePayloadCandidate(extractJsonObject(text));
  }

  function parsePayloadCandidate(value) {
    const text = normalizeText(value)
      .replace(/^```(?:json)?/i, "")
      .replace(/```$/i, "")
      .replace(/^json\s+/i, "")
      .trim();

    if (!/"codex_stepwise"\s*:\s*true/.test(text)) return null;

    try {
      const parsed = JSON.parse(text);
      return parsed && parsed.codex_stepwise === true ? parsed : null;
    } catch {
      return null;
    }
  }

  function extractJsonObject(text) {
    const source = String(text || "");
    const marker = source.search(/"codex_stepwise"\s*:\s*true/);
    if (marker < 0) return "";

    const start = source.lastIndexOf("{", marker);
    if (start < 0) return "";

    let depth = 0;
    let inString = false;
    let escaped = false;

    for (let index = start; index < source.length; index += 1) {
      const char = source[index];
      if (escaped) {
        escaped = false;
        continue;
      }
      if (char === "\\") {
        escaped = true;
        continue;
      }
      if (char === "\"") {
        inString = !inString;
        continue;
      }
      if (inString) continue;
      if (char === "{") depth += 1;
      if (char === "}") depth -= 1;
      if (depth === 0) return source.slice(start, index + 1);
    }

    return "";
  }

  function stripStepwisePayloadText(text) {
    const withoutFence = String(text || "").replace(/```(?:json)?\s*[\s\S]*?"codex_stepwise"\s*:\s*true[\s\S]*?```/gi, "");
    const payloadObject = extractJsonObject(withoutFence);
    return normalizeText(payloadObject ? withoutFence.replace(payloadObject, "") : withoutFence);
  }

  function payloadFromDom(root) {
    if (!(root instanceof Element)) return null;
    const blocks = Array.from(root.querySelectorAll("pre, code"))
      .filter((node) => /"codex_stepwise"\s*:\s*true/.test(node.textContent || ""));

    for (const block of blocks.reverse()) {
      const parsed = parsePayloadCandidate(block.textContent || "");
      if (parsed) return parsed;
    }

    return null;
  }

  function payloadItems(payload) {
    if (!payload) return [];
    if (Array.isArray(payload)) return payload;
    for (const key of ["items", "suggestions", "next_steps", "nextSteps", "actions", "prompts"]) {
      if (Array.isArray(payload[key])) return payload[key];
    }
    return [];
  }

  function payloadPrompts(payload) {
    const rawItems = payloadItems(payload);
    if (!rawItems.length) return [];
    const items = rawItems
      .slice(0, configuredMaxPromptItems())
      .map((item) => {
        const prompt = normalizeText(
          typeof item === "string"
            ? item
            : item?.prompt || item?.text || item?.action || item?.content || item?.message || "",
        );
        const label = leadingPromptText(
          typeof item === "string" ? "" : item?.label || item?.title || item?.name || "",
          36,
        );
        const summary = leadingPromptText(
          typeof item === "string" ? "" : item?.summary || item?.preview || item?.description || "",
          MAX_PROMPT_SUMMARY_LENGTH,
        );
        return prompt ? {
          label: label || labelForPrompt(prompt),
          summary: summary || summaryForPrompt(prompt),
          prompt,
        } : null;
      })
      .filter(Boolean);
    return uniquePrompts(items);
  }

  function extractStepwisePayload(message) {
    const text = elementText(message.node);
    const payload = payloadFromDom(message.node) || parseStepwiseJson(text);
    return {
      payload,
      prompts: payloadPrompts(payload),
      textWithoutPayload: stripStepwisePayloadText(text),
    };
  }

  function bridgeRequestKey(userText, assistantText) {
    return hashText(`${state.activeContext.sessionId}\n${shortText(userText, 2400)}\n\n--- assistant ---\n\n${shortText(assistantText, 5200)}`);
  }

  // Bridge requests are deduplicated by answer identity and guarded against late responses from older turns.
  function requestBridgeStepwise(key, userText, assistantText, requestMode = stepwiseGenerationMode(), options = {}) {
    if (!stepwiseEnabled() || !key || state.bridgePendingHash === key || state.bridgeCache.has(key)) return;

    const normalizedMode = normalizeGenerationMode(requestMode);
    if (normalizedMode === "manual" && options.userInitiated !== true) return;
    pushDiagnostic("bridge:generate-request", {
      userTextLength: userText.length,
      assistantTextLength: assistantText.length,
      mode: normalizedMode,
    });
    const requestContext = contextSnapshot();
    const requestEpoch = state.stepwiseEpoch;
    const requestId = ++state.bridgeRequestSequence;
    const requestAssistantMessageId = requestContext.assistantMessageId;
    const requestOwned = () => state.bridgePendingHash === key
      && state.bridgePendingRequestId === requestId
      && state.bridgePendingMode === normalizedMode;
    const requestCurrent = () => stepwiseEnabled()
      && stepwiseGenerationMode() === normalizedMode
      && requestEpoch === state.stepwiseEpoch
      && contextMatches(requestContext)
      && state.activeContext.assistantMessageId === requestAssistantMessageId
      && state.bridgeActiveKey === key
      && !chatBusy();
    state.bridgePendingHash = key;
    state.bridgePendingRequestId = requestId;
    state.bridgePendingMode = normalizedMode;
    state.bridgeStatus = "pending";
    state.bridgeError = "";
    state.promptContext = requestContext;
    renderFloat();

    bridgeCall(
      "/stepwise/generate",
      {
        request: {
        lastUserMessage: userText,
        lastAssistantMessage: assistantText,
        threadTitle: document.title || "",
        pageUrl: location.href,
      },
      }
    )
      .then((payload) => {
        if (!requestOwned() || !requestCurrent()) return;
        const prompts = payload?.disabled || payload?.error ? [] : payloadPrompts(payload);
        pushDiagnostic("bridge:generate-result", {
          status: payload?.status || "",
          disabled: Boolean(payload?.disabled),
          error: normalizeText(payload?.error || ""),
          rawItemCount: payloadItems(payload).length,
          promptCount: prompts.length,
          payloadKeys: payload && typeof payload === "object" ? Object.keys(payload).slice(0, 12) : [],
        });
        const bridgeStatus = payload?.disabled ? "disabled" : payload?.error ? "failed" : "ok";
        state.bridgeCache.set(key, {
          status: bridgeStatus,
          disabled: Boolean(payload?.disabled),
          error: normalizeText(payload?.error || ""),
          prompts,
        });
        state.bridgeStatus = bridgeStatus;
        state.bridgeError = normalizeText(payload?.error || "");
        state.promptContext = requestContext;
        if (bridgeStatus === "ok") triggerCompletionBeam(prompts.length);
      })
      .catch((error) => {
        if (!requestOwned() || !requestCurrent()) return;
        pushDiagnostic("bridge:generate-failed", { error: error.message });
        state.bridgeCache.set(key, {
          status: "failed",
          disabled: true,
          error: error.message,
          prompts: [],
        });
        state.bridgeStatus = "failed";
        state.bridgeError = error.message;
      })
      .finally(() => {
        if (!requestOwned()) return;
        state.bridgePendingHash = "";
        state.bridgePendingRequestId = 0;
        state.bridgePendingMode = stepwiseGenerationMode();
        if (state.bridgeStatus === "pending") {
          state.bridgeStatus = "idle";
          state.bridgeError = "";
          state.promptContext = null;
        }
        scheduleScan(0);
      });
  }

  function forceRefreshStepwise() {
    if (!isCurrentRuntime() || !stepwiseEnabled()) return;
    if (state.bridgeStatus === "pending") {
      setScanStatus("manual-refresh-pending", {});
      return;
    }
    if (chatBusy()) {
      if (!state.prompts.length) state.bridgeError = "回答生成中，结束后再刷新";
      setScanStatus("manual-refresh-busy", {});
      renderFloat();
      return;
    }

    const message = findLatestAssistantMessage();
    if (!message) {
      state.bridgeError = "未找到可用于生成的回答";
      state.prompts = [];
      state.promptContext = null;
      state.promptPreviewIndex = 0;
      setScanStatus("manual-refresh-no-assistant", {});
      renderFloat();
      return;
    }

    const nextAssistantMessageId = assistantMessageId(message);
    if (state.activeContext.assistantMessageId !== nextAssistantMessageId) {
      state.activeContext.assistantMessageId = nextAssistantMessageId;
    }

    const stepwisePayload = extractStepwisePayload(message);
    hideStepwisePayload(message.node);
    const assistantText = shortText(stepwisePayload.textWithoutPayload || message.text);
    const userText = findPreviousUserText(message);
    const bridgeKey = bridgeRequestKey(userText, assistantText);
    const generationMode = stepwiseGenerationMode();
    state.bridgeActiveKey = bridgeKey;
    state.stepwiseEpoch += 1;
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = generationMode;
    if (bridgeKey) state.bridgeCache.delete(bridgeKey);

    state.lastAssistantHash = hashText(assistantText);
    state.lastAssistantAt = 0;
    state.currentHash = `${state.lastAssistantHash}:manual-refresh`;
    state.prompts = [];
    state.promptContext = contextSnapshot();
    state.promptPreviewIndex = 0;
    state.bridgeError = "";
    setScanStatus("manual-refresh", { hash: state.lastAssistantHash, textLength: assistantText.length });
    requestBridgeStepwise(bridgeKey, userText, assistantText, generationMode, { userInitiated: true });
    renderFloat();
  }

  function clearPromptsForNewAssistant(hash) {
    state.stepwiseEpoch += 1;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = stepwiseGenerationMode();
    state.bridgeStatus = state.bridgePendingMode === "manual" ? "manual-ready" : "idle";
    state.currentHash = `${hash}:pending`;
    state.prompts = [];
    state.promptContext = contextSnapshot();
    state.promptPreviewIndex = 0;
    state.bridgeError = "";
    renderFloat();
  }

  function composerRootForContext(snapshot = state.promptContext) {
    if (snapshot?.paneKey) return rootForContext(snapshot.paneKey, snapshot.sessionId);
    return chatRoot();
  }

  function composerTargetForContext(snapshot = state.promptContext) {
    const root = composerRootForContext(snapshot);
    if (!root) return null;
    return mainComposerCandidate(composerCandidates(root), root);
  }

  function setNativeValue(element, value) {
    const prototype = Object.getPrototypeOf(element);
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "value");
    if (descriptor && typeof descriptor.set === "function") descriptor.set.call(element, value);
    else element.value = value;
  }

  function composerText(target) {
    if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) return normalizeText(target.value);
    return normalizeText(target?.innerText || target?.textContent || "");
  }

  function pressEnter(target) {
    target.focus();
    const base = {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
      bubbles: true,
      cancelable: true,
      composed: true,
    };
    const down = target.dispatchEvent(new KeyboardEvent("keydown", base));
    target.dispatchEvent(new KeyboardEvent("keypress", base));
    target.dispatchEvent(new KeyboardEvent("keyup", base));
    pushDiagnostic("submit:enter-fallback", { defaultAllowed: down });
    return true;
  }

  function submitComposer(target, allowFallback = false) {
    if (!(target instanceof HTMLElement)) return false;
    if (composerBusy(target)) {
      pushDiagnostic("submit:blocked-local-stop", { attemptFallback: allowFallback });
      return false;
    }

    const button = nearbySubmitButton(target);
    if (button) {
      pushDiagnostic("submit:button-click", {
        label: buttonLabel(button),
        disabled: disabledButton(button),
        rect: rectSummary(button),
        className: String(button.className || "").slice(0, 160),
        composerTextLength: composerText(target).length,
        iconPath: iconPathData(button).slice(0, 160),
      });
      button.click();
      return true;
    }

    const pendingButton = nearbySubmitButton(target, { includeDisabled: true });
    if (pendingButton && disabledButton(pendingButton)) {
      pushDiagnostic("submit:button-disabled", {
        label: buttonLabel(pendingButton),
        rect: rectSummary(pendingButton),
        className: String(pendingButton.className || "").slice(0, 160),
        composerTextLength: composerText(target).length,
        iconPath: iconPathData(pendingButton).slice(0, 160),
      });
      return false;
    }

    const form = target.closest("form");
    if (form && allowFallback) {
      pushDiagnostic("submit:form-fallback", { rect: rectSummary(form) });
      try {
        form.requestSubmit();
      } catch {
        pushDiagnostic("submit:form-fallback-failed", {});
        return false;
      }
      return true;
    }

    if (allowFallback) return pressEnter(target);
    pushDiagnostic("submit:no-button-yet", { allowFallback });
    return false;
  }

  function submitComposerWhenReady(target, expectedText = "", attempt = 0) {
    let currentTarget = target;
    if (!(currentTarget instanceof HTMLElement)) return false;
    if (!document.contains(currentTarget)) {
      currentTarget = composerTargetForContext(state.promptContext || state.activeContext);
      pushDiagnostic("submit:target-detached", {
        attempt,
        rebound: Boolean(currentTarget),
        paneKey: state.promptContext?.paneKey || state.activeContext.paneKey,
        sessionId: state.promptContext?.sessionId || state.activeContext.sessionId,
      });
      if (!currentTarget) {
        if (attempt >= SUBMIT_RETRY_LIMIT) return false;
        window.setTimeout(() => submitComposerWhenReady(target, expectedText, attempt + 1), SUBMIT_RETRY_DELAY_MS);
        return false;
      }
    }
    if (normalizeText(expectedText) && composerText(currentTarget) !== normalizeText(expectedText)) {
      pushDiagnostic("submit:composer-changed", {
        attempt,
        expectedLength: normalizeText(expectedText).length,
        actualLength: composerText(currentTarget).length,
      });
      return false;
    }
    if (composerBusy(currentTarget)) {
      if (attempt === 0 || attempt % 10 === 0 || attempt >= SUBMIT_RETRY_LIMIT) {
        pushDiagnostic("submit:blocked-local-stop", {
          attempt,
          retrying: attempt < SUBMIT_RETRY_LIMIT,
          targetRect: rectSummary(currentTarget),
        });
      }
      if (attempt >= SUBMIT_RETRY_LIMIT) {
        pushDiagnostic("submit:blocked-local-stop-timeout", { attempt, targetRect: rectSummary(currentTarget) });
        return false;
      }
      window.setTimeout(() => submitComposerWhenReady(currentTarget, expectedText, attempt + 1), SUBMIT_RETRY_DELAY_MS);
      return false;
    }
    if (submitComposer(currentTarget, attempt >= SUBMIT_RETRY_LIMIT)) return true;
    if (attempt >= SUBMIT_RETRY_LIMIT) return false;
    window.setTimeout(() => submitComposerWhenReady(currentTarget, expectedText, attempt + 1), SUBMIT_RETRY_DELAY_MS);
    return false;
  }

  function setEditableText(target, prompt) {
    target.focus();
    const selection = window.getSelection?.();
    const range = document.createRange();
    range.selectNodeContents(target);
    selection?.removeAllRanges();
    selection?.addRange(range);

    let inserted = false;
    try {
      inserted = document.execCommand?.("insertText", false, prompt) === true;
    } catch {
      inserted = false;
    }
    if (!inserted) target.textContent = prompt;
  }

  function fillComposer(prompt, submit = false) {
    const context = state.promptContext || state.activeContext;
    const targetRoot = composerRootForContext(context);
    const candidates = targetRoot ? composerCandidates(targetRoot) : [];
    const target = targetRoot
      ? mainComposerCandidate(candidates, targetRoot)
      : null;
    pushDiagnostic("fill:start", {
      submit,
      candidateCount: candidates.length,
      paneKey: context?.paneKey || "",
      sessionId: context?.sessionId || "",
      targetTag: target?.tagName || "",
      targetRole: target?.getAttribute?.("role") || "",
      targetClass: String(target?.className || "").slice(0, 120),
      targetRect: rectSummary(target),
      chatRootRect: rectSummary(targetRoot),
      promptLength: normalizeText(prompt).length,
    });
    if (!target) {
      pushDiagnostic("fill:no-main-composer", { candidateCount: candidates.length });
      window.prompt("Copy Stepwise prompt", prompt);
      return false;
    }

    target.focus();
    if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
      setNativeValue(target, prompt);
      target.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: prompt }));
      target.dispatchEvent(new Event("change", { bubbles: true }));
      pushDiagnostic("fill:text-control", { valueLength: normalizeText(target.value).length });
      if (submit) submitComposerWhenReady(target, prompt);
      return true;
    }

    if (target.isContentEditable || target.getAttribute("role") === "textbox") {
      setEditableText(target, prompt);
      target.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: prompt }));
      pushDiagnostic("fill:editable", { valueLength: normalizeText(target.textContent).length });
      if (submit) window.setTimeout(() => submitComposerWhenReady(target, prompt), EDITABLE_SUBMIT_DELAY_MS);
      return true;
    }

    window.prompt("Copy Stepwise prompt", prompt);
    return false;
  }

  // Scanning observes the pinned conversation, settles streamed answers, and schedules only necessary work.
  function scan(generation = state.runtimeGeneration, timerId = 0) {
    if (!isCurrentRuntime(generation)) return;
    if (timerId && state.timer !== timerId) return;
    if (timerId) state.timer = 0;
    state.scans += 1;
    installStyle();
    installFloat();
    const stepwiseActive = stepwiseEnabled();
    const outlineActive = outlineEnabled();

    if (!chatSurfaceReady()) {
      if (outlineActive && (state.outlineItems.length || state.outlineMessage)) invalidateOutline();
      const statusChanged = setScanStatus("not-ready", {
        hasRoot: Boolean(chatRoot()),
        composerCount: composerCandidates().length,
        busy: chatBusy(),
      });
      if (statusChanged) renderFloat();
      return;
    }

    const message = findLatestAssistantMessage();
    if (!message) {
      if (outlineActive && (state.outlineItems.length || state.outlineMessage)) invalidateOutline();
      const statusChanged = setScanStatus("no-assistant-message", {
        messageCandidateCount: messageCandidates().length,
        actionRowCount: allActionRows().length,
      });
      if (statusChanged) renderFloat();
      return;
    }

    const stepwisePayload = stepwiseActive
      ? extractStepwisePayload(message)
      : { payload: null, prompts: [], textWithoutPayload: "" };
    if (stepwiseActive) hideStepwisePayload(message.node);

    const nextAssistantMessageId = assistantMessageId(message);
    if (state.activeContext.assistantMessageId !== nextAssistantMessageId) {
      state.activeContext.assistantMessageId = nextAssistantMessageId;
    }

    const assistantText = shortText(stepwiseActive
      ? stepwisePayload.textWithoutPayload || message.text
      : message.text);
    const hash = hashText(assistantText);
    const now = Date.now();

    if (hash !== state.lastAssistantHash) {
      state.lastAssistantHash = hash;
      state.lastAssistantAt = now;
      state.surpriseUntil = now + NEW_ANSWER_EXPRESSION_MS;
      scheduleExpressionRefresh(NEW_ANSWER_EXPRESSION_MS);
      setScanStatus("assistant-changed", { hash, textLength: assistantText.length });
      if (outlineActive) invalidateOutline(message, hash);
      if (stepwiseActive) {
        clearPromptsForNewAssistant(hash);
      } else {
        renderFloat();
      }
      scheduleScan(STREAM_IDLE_MS + 120);
      return;
    }

    if (now - state.lastAssistantAt < STREAM_IDLE_MS) {
      setScanStatus("assistant-settling", { hash });
      scheduleScan(STREAM_IDLE_MS);
      return;
    }

    if (outlineActive && state.outlineSourceHash !== hash && !state.outlineRefreshPromise) {
      void refreshOutline({ message, assistantHash: hash });
    }
    if (!stepwiseActive) {
      const statusChanged = setScanStatus("ready", {
        hash,
        outlineOnly: true,
        outlineCount: state.outlineItems.length,
      });
      if (statusChanged) renderFloat();
      return;
    }

    const userText = findPreviousUserText(message);
    const bridgeKey = bridgeRequestKey(userText, assistantText);
    const generationMode = stepwiseGenerationMode();
    state.bridgeActiveKey = bridgeKey;
    const bridgeResult = state.bridgeCache.get(bridgeKey);
    const hasSuccessfulCache = bridgeResult?.status === "ok";
    let prompts = [];

    const manualResultVisible = generationMode === "manual"
      && state.bridgeStatus === "ok"
      && state.bridgeActiveKey === bridgeKey;

    if (generationMode === "manual" && !manualResultVisible) {
      state.bridgeStatus = "manual-ready";
      state.bridgeError = "";
      state.promptContext = contextSnapshot();
    } else if (hasSuccessfulCache) {
      prompts = Array.isArray(bridgeResult.prompts) ? bridgeResult.prompts : [];
      state.bridgeStatus = "ok";
      state.bridgeError = "";
      state.promptContext = contextSnapshot();
    } else {
      prompts = bridgeResult ? [] : stepwisePayload.prompts;
      if (bridgeResult) {
        state.bridgeStatus = bridgeResult.status || (bridgeResult.error ? "failed" : bridgeResult.disabled ? "disabled" : "ok");
        state.bridgeError = bridgeResult.error || "";
        state.promptContext = contextSnapshot();
      } else {
        requestBridgeStepwise(bridgeKey, userText, assistantText, "auto");
      }
    }
    setScanStatus("ready", {
      hash,
      bridgeCached: Boolean(bridgeResult),
      promptCount: prompts.length,
    });

    const nextHash = hashText(`${generationMode}:${state.bridgeStatus}:${prompts.map((item) => `${item.label}\n${item.prompt}`).join("\n\n")}`);
    const renderedHash = `${hash}:${nextHash}`;
    if (state.currentHash !== renderedHash) {
      state.currentHash = renderedHash;
      state.prompts = prompts;
      state.promptContext = contextSnapshot();
      state.promptPreviewIndex = 0;
      renderFloat();
    }
  }

  function scheduleScan(delay = SCAN_DELAY_MS) {
    if (!isCurrentRuntime()) return;
    if (state.timer) window.clearTimeout(state.timer);
    const generation = state.runtimeGeneration;
    const timer = window.setTimeout(() => scan(generation, timer), delay);
    state.timer = timer;
  }

  function installObserver() {
    if (!isCurrentRuntime()) return false;
    const root = document.body || document.documentElement;
    if (!root) return false;

    const generation = state.runtimeGeneration;
    state.observer = new MutationObserver((mutations) => {
      if (!isCurrentRuntime(generation)) return;
      const relevant = mutations.some((mutation) => {
        if (state.root?.contains(mutation.target)) return false;
        return mutation.addedNodes.length || mutation.type === "characterData";
      });
      if (relevant) scheduleScan();
    });
    state.observer.observe(root, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    return true;
  }

  // Stopping invalidates every generation, removes observers, and leaves no page-owned runtime behind.
  function stopRuntime() {
    state.runtimeActive = false;
    state.runtimeGeneration += 1;
    state.latestTurnAnchor = null;
    if (state.domReadyHandler) document.removeEventListener("DOMContentLoaded", state.domReadyHandler);
    state.domReadyHandler = null;
    if (state.timer) window.clearTimeout(state.timer);
    if (state.expressionTimer) window.clearTimeout(state.expressionTimer);
    if (state.keepAliveTimer) window.clearTimeout(state.keepAliveTimer);
    if (state.flashTimer) window.clearTimeout(state.flashTimer);
    if (state.materialAnimTimer) window.clearTimeout(state.materialAnimTimer);
    if (state.completionBeamTimer) window.clearTimeout(state.completionBeamTimer);
    if (state.snapTimer) window.clearTimeout(state.snapTimer);
    if (state.eyeRaf) window.cancelAnimationFrame(state.eyeRaf);
    state.timer = 0;
    state.expressionTimer = 0;
    state.keepAliveTimer = 0;
    state.flashTimer = 0;
    state.materialAnimTimer = 0;
    state.completionBeamTimer = 0;
    state.snapTimer = 0;
    state.eyeRaf = 0;
    state.surpriseUntil = 0;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.viewTransitioning = false;
    state.pendingTab = "";
    state.pendingRender = false;
    state.popover?.removeAttribute?.("data-snap-right");
    cancelViewAnimation();
    cancelSourceCueAnimation();
    cancelMorphAnimations();
    state.dragCleanup?.();
    state.resizeCleanup?.();
    state.contentFadeCleanup?.();
    state.eyeCleanup?.();
    state.eyeCleanup = null;
    state.contentFadeCleanup = null;
    state.eyePointer = null;
    document.querySelectorAll(".codex-stepwise-active-pane, .codex-stepwise-pane-flash").forEach((node) => {
      node.classList.remove("codex-stepwise-active-pane", "codex-stepwise-pane-flash");
    });
    removeContextTracking();
    if (state.keyHandler) document.removeEventListener("keydown", state.keyHandler, true);
    state.keyHandler = null;
    window.removeEventListener("resize", onResize);
    state.observer?.disconnect();
    state.observer = null;
    state.themeObserver?.disconnect();
    state.themeObserver = null;
    state.typographyObserver?.disconnect();
    state.typographyObserver = null;
    clearPromptInteractionTimers();
    clearStepwisePayloadMarks();
    outlineClearMarks();
    state.outlineItems = [];
    state.outlineRefreshPromise = null;
    state.outlineMessage = null;
    state.outlineScrollCleanup?.();
    state.outlineScrollCleanup = null;
    state.outlineSourceHash = "";
    state.outlineFingerprint = "";
    state.outlineStatus = "idle";
    state.outlineError = "";
    state.root?.remove();
    state.root = null;
    state.fab = null;
    state.popover = null;
    state.glass = null;
    state.rim = null;
    state.completionBeam = null;
    state.clearFilter = null;
    state.clearDisplacement = null;
    state.clearDistortion = null;
    state.liquidFilter = null;
    state.crystalFilter = null;
    state.displacementTexture = null;
    state.panel = null;
    state.layout = null;
    state.drag = null;
    state.resizeDrag = null;
    state.dragCleanup = null;
    state.resizeCleanup = null;
    state.focusAfterMorph = "";
    state.pinnedThreadRoot = null;
    state.pinnedThreadAt = 0;
    state.activeContext = {
      paneRoot: null,
      paneKey: "",
      sessionId: "",
      assistantMessageId: "",
      generation: state.activeContext.generation + 1,
    };
    document.getElementById(STYLE_ID)?.remove();
    state.open = false;
  }

  function activateRuntime() {
    if (!isCurrentInstance()) return false;
    if (!state.runtimeActive) {
      state.runtimeGeneration += 1;
      state.runtimeActive = true;
    }
    const generation = state.runtimeGeneration;
    state.activeTab = normalizeActiveTab();
    installStyle();
    installFloat();
    installContextTracking();
    if (!state.observer && !installObserver()) {
      const domReadyHandler = () => {
        if (state.domReadyHandler === domReadyHandler) state.domReadyHandler = null;
        if (!isCurrentRuntime(generation)) return;
        installObserver();
        installFloat();
        void ensureSettings();
        scheduleScan(0);
      };
      state.domReadyHandler = domReadyHandler;
      document.addEventListener("DOMContentLoaded", domReadyHandler, { once: true });
    }
    scheduleScan(0);
    return true;
  }

  async function syncSettings(patch = {}) {
    if (!isCurrentInstance()) return null;
    const normalizedPatch = {};
    if (patch && typeof patch === "object") {
      Object.entries(patch).forEach(([key, value]) => {
        if (value !== undefined) normalizedPatch[key] = value;
      });
    }
    if (Object.keys(normalizedPatch).length) {
      if (!state.settingsLoaded) {
        pendingSettingsPatch = { ...pendingSettingsPatch, ...normalizedPatch };
      }
      applyRuntimeSettings({ ...(state.settings || {}), ...normalizedPatch });
    }
    const hasRuntimePatch = typeof normalizedPatch.enabled === "boolean"
      || typeof normalizedPatch.answerOutlineEnabled === "boolean"
      || Object.prototype.hasOwnProperty.call(normalizedPatch, "generationMode");
    if (patch?.enabled === true) {
      pushDiagnostic("settings:enabled-sync", {});
    }
    if (patch?.answerOutlineEnabled === true) pushDiagnostic("settings:outline-enabled-sync", {});
    if (Object.prototype.hasOwnProperty.call(normalizedPatch, "generationMode")) {
      pushDiagnostic("settings:generation-mode-sync", {
        mode: stepwiseGenerationMode(),
      });
    }
    if (hasRuntimePatch) {
      const hasInFlightSettingsRequest = Boolean(settingsPromise);
      settingsSyncEpoch += 1;
      if (!state.settingsLoaded || hasInFlightSettingsRequest) {
        pendingSettingsPatch = { ...pendingSettingsPatch, ...normalizedPatch };
        settingsPromise = null;
        void reloadSettings();
      }
      if (!runtimeEnabled()) {
        pushDiagnostic("settings:disabled-sync", {});
        if (state.runtimeActive) stopRuntime();
        return state.settings;
      }
      activateRuntime();
      renderFloat();
      scheduleScan(0);
      return state.settings;
    }

    settingsPromise = null;
    startupPromise = null;
    const settings = await loadSettings();
    if (!isCurrentInstance()) return null;
    if (!runtimeEnabled(settings)) {
      pushDiagnostic("settings:disabled-sync", {});
      if (state.runtimeActive) stopRuntime();
      return settings;
    }
    pushDiagnostic("settings:enabled-sync", {});
    activateRuntime();
    renderFloat();
    scheduleScan(0);
    return settings;
  }

  function destroy() {
    state.destroyed = true;
    state.promptContext = null;
    state.latestTurnAnchor = null;
    state.pinnedPaneKey = "";
    state.pinnedSessionId = "";
    state.pinnedThreadRoot = null;
    if (state.settingsSyncTimer) window.clearTimeout(state.settingsSyncTimer);
    state.settingsSyncTimer = 0;
    cancelSourceCueAnimation();
    cancelViewAnimation();
    stopRuntime();
    if (window[API_KEY]?.instanceId === INSTANCE_ID) delete window[API_KEY];
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

  async function start() {
    scheduleSettingsSync();
    if (startupPromise) return startupPromise;
    const generation = state.runtimeGeneration;
    startupPromise = (async () => {
      const settings = await ensureSettings();
      if (!isCurrentInstance() || generation !== state.runtimeGeneration) return;
      if (!runtimeEnabled(settings)) {
        pushDiagnostic("startup:disabled", {});
        startupPromise = null;
        return;
      }
      activateRuntime();
    })();
    return startupPromise;
  }

  // A small debug surface exposes state and lifecycle controls without leaking chat contents by default.
  window[API_KEY] = {
    version: SCRIPT_VERSION,
    instanceId: INSTANCE_ID,
    state,
    scan,
    start,
    destroy,
    loadSettings,
    syncSettings,
    setOpen,
    setMaterial: writeMaterial,
    toggleMaterial,
    dockRight: dockRightKeepHeight,
    getFabExpression: () => resolveFabExpression(),
    renderFloat,
    diagnostics: () => state.diagnostics.slice(),
  };

  void start();
})();
