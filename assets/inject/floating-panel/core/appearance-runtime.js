/* Floating-panel appearance runtime: typography, materials, theme, and icons. */

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
