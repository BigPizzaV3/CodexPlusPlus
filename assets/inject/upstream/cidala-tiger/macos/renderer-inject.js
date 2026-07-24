((cssText, artDataUrl, themeConfig) => {
  const STATE_KEY = "__CODEX_DREAM_SKIN_STATE__";
  const DISABLED_KEY = "__CODEX_DREAM_SKIN_DISABLED__";
  const STYLE_ID = "codex-dream-skin-style";
  const CHROME_ID = "codex-dream-skin-chrome";
  const OUTPUTS_PANEL_CLASS = "dream-skin-outputs-panel";
  const PREVIEW_PANEL_CLASS = "dream-skin-preview-panel";
  const PREVIEW_LAUNCHER_CLASS = "dream-skin-preview-launcher";
  const SHELL_ATTR = "data-dream-shell";
  const ART_ATTRS = [
    "data-dream-art-wide", "data-dream-art-safe", "data-dream-task-mode",
    "data-dream-art-safe-area", "data-dream-art-task-mode", "data-dream-art-aspect",
    "data-dream-art-ready", "data-dream-decor",
  ];
  const VERSION = __DREAM_SKIN_VERSION_JSON__;
  const STYLE_REVISION = __DREAM_SKIN_STYLE_REVISION_JSON__;
  const THEME = themeConfig && typeof themeConfig === "object" ? themeConfig : {};
  const ART = THEME.art && typeof THEME.art === "object" ? THEME.art : {};
  const DECOR = THEME.decor && typeof THEME.decor === "object" ? THEME.decor : {};
  const ART_METADATA = THEME.artMetadata && typeof THEME.artMetadata === "object"
    ? THEME.artMetadata : null;
  const ANALYSIS_CACHE_KEY = "__CODEX_DREAM_SKIN_ANALYSIS_CACHE__";
  const THEME_VARIABLES = [
    "--ds-bg", "--ds-panel", "--ds-panel-2", "--ds-green", "--ds-lime",
    "--ds-cyan", "--ds-purple", "--ds-text", "--ds-muted", "--ds-line",
    "--ds-bg-rgb", "--ds-panel-rgb", "--ds-panel-2-rgb", "--ds-accent-rgb",
    "--ds-accent-alt-rgb", "--ds-secondary-rgb", "--ds-highlight-rgb",
    "--ds-text-rgb", "--ds-muted-rgb", "--ds-line-rgb",
    "--dream-art-focus-x", "--dream-art-focus-y", "--dream-art-position",
    "--dream-skin-focus-x", "--dream-skin-focus-y", "--dream-skin-art-position",
    "--dream-skin-decor-intensity",
    "--dream-skin-name", "--dream-skin-tagline", "--dream-skin-project-prefix",
    "--dream-skin-project-label",
  ];
  const installToken = {};
  const existingAnalysisCache = window[ANALYSIS_CACHE_KEY];
  const analysisCache = existingAnalysisCache && typeof existingAnalysisCache.get === "function" &&
    typeof existingAnalysisCache.set === "function" ? existingAnalysisCache : new Map();
  window[ANALYSIS_CACHE_KEY] = analysisCache;
  let artAnalysis = typeof THEME.artKey === "string" ? analysisCache.get(THEME.artKey) ?? null : null;
  let analysisTimer = null;
  let samplingNativeShell = false;
  let rootObserver = null;
  const now = () => typeof performance === "object" && typeof performance.now === "function"
    ? performance.now() : Date.now();
  const metrics = {
    ensureCalls: 0,
    rootPasses: 0,
    routePasses: 0,
    layoutReads: 0,
    attributeWrites: 0,
    styleWrites: 0,
    textWrites: 0,
    analysisRuns: 0,
    analysisCacheHits: artAnalysis ? 1 : 0,
    firstEnsureMs: null,
    analysisMs: null,
  };
  window[DISABLED_KEY] = false;

  const previous = window[STATE_KEY];
  const objectUrlFromDataUrl = (dataUrl) => {
    if (typeof dataUrl !== "string") return null;
    const comma = dataUrl.indexOf(",");
    if (comma < 0) return null;
    const mime = /^data:([^;,]+)/.exec(dataUrl)?.[1] || "image/png";
    const binary = atob(dataUrl.slice(comma + 1));
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
    return URL.createObjectURL(new Blob([bytes], { type: mime }));
  };
  const artUrl = objectUrlFromDataUrl(artDataUrl);
  const decorUrls = {};
  for (const key of ["ribbon", "cloud", "seal", "accent"]) {
    const url = objectUrlFromDataUrl(DECOR.images?.[key]);
    if (url) decorUrls[key] = url;
  }

  if (previous?.observer) previous.observer.disconnect();
  if (previous?.rootObserver) previous.rootObserver.disconnect();
  if (previous?.resizeObserver) previous.resizeObserver.disconnect();
  if (previous?.timer) clearInterval(previous.timer);
  if (previous?.scheduler?.timeout) clearTimeout(previous.scheduler.timeout);
  if (previous?.scheduler?.frame != null && typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(previous.scheduler.frame);
  }
  if (previous?.analysisTimer) clearTimeout(previous.analysisTimer);
  if (previous?.resizeHandler) window.removeEventListener("resize", previous.resizeHandler);
  if (previous?.mediaHandler && previous?.mediaQuery) {
    try { previous.mediaQuery.removeEventListener("change", previous.mediaHandler); } catch {}
  }

  const cssString = (value) => JSON.stringify(String(value ?? ""));

  const setStyleProperty = (root, name, value) => {
    if (root.style.getPropertyValue(name) !== value) {
      root.style.setProperty(name, value);
      metrics.styleWrites += 1;
    }
  };

  const setAttribute = (root, name, value) => {
    const normalized = String(value);
    if (root.getAttribute(name) !== normalized) {
      root.setAttribute(name, normalized);
      metrics.attributeWrites += 1;
    }
  };

  const setTextContent = (node, value) => {
    if (node && node.textContent !== value) {
      node.textContent = value;
      metrics.textWrites += 1;
    }
  };

  const setImageSource = (node, value) => {
    if (node && value && node.src !== value) {
      node.src = value;
      metrics.attributeWrites += 1;
    }
  };

  const parseRgb = (value) => {
    if (!value || value === "transparent") return null;
    const hex = String(value).trim().match(/^#([0-9a-f]{6})$/i);
    if (hex) {
      const number = Number.parseInt(hex[1], 16);
      return { r: number >> 16, g: (number >> 8) & 255, b: number & 255 };
    }
    const m = String(value).match(/rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)/i);
    if (!m) return null;
    return { r: Number(m[1]), g: Number(m[2]), b: Number(m[3]) };
  };

  const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

  const rgbString = (value) => {
    const rgb = parseRgb(value);
    return rgb ? `${Math.round(rgb.r)} ${Math.round(rgb.g)} ${Math.round(rgb.b)}` : null;
  };

  const rgbToHex = ({ r, g, b }) => `#${[r, g, b]
    .map((value) => clamp(Math.round(value), 0, 255).toString(16).padStart(2, "0"))
    .join("")}`;

  const rgbToHsl = ({ r, g, b }) => {
    const values = [r, g, b].map((value) => value / 255);
    const max = Math.max(...values);
    const min = Math.min(...values);
    const lightness = (max + min) / 2;
    if (max === min) return { h: 0, s: 0, l: lightness };
    const delta = max - min;
    const saturation = lightness > 0.5 ? delta / (2 - max - min) : delta / (max + min);
    let hue;
    if (max === values[0]) hue = (values[1] - values[2]) / delta + (values[1] < values[2] ? 6 : 0);
    else if (max === values[1]) hue = (values[2] - values[0]) / delta + 2;
    else hue = (values[0] - values[1]) / delta + 4;
    return { h: hue * 60, s: saturation, l: lightness };
  };

  const hslToRgb = ({ h, s, l }) => {
    const hue = ((h % 360) + 360) % 360 / 360;
    if (s === 0) {
      const neutral = Math.round(l * 255);
      return { r: neutral, g: neutral, b: neutral };
    }
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    const channel = (offset) => {
      let t = hue + offset;
      if (t < 0) t += 1;
      if (t > 1) t -= 1;
      if (t < 1 / 6) return p + (q - p) * 6 * t;
      if (t < 1 / 2) return q;
      if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
      return p;
    };
    return { r: channel(1 / 3) * 255, g: channel(0) * 255, b: channel(-1 / 3) * 255 };
  };

  const luminance = ({ r, g, b }) => {
    const lin = [r, g, b].map((c) => {
      const x = c / 255;
      return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
  };

  /** Detect Codex app light/dark shell for CSS branching. */
  const detectShellMode = () => {
    const root = document.documentElement;
    const body = document.body;
    const cls = `${root.className || ""} ${body?.className || ""}`.toLowerCase();

    if (/\b(dark|theme-dark|appearance-dark)\b/.test(cls)) return "dark";
    if (/\b(light|theme-light|appearance-light)\b/.test(cls)) return "light";

    const dataTheme = (
      root.getAttribute("data-theme") ||
      root.getAttribute("data-appearance") ||
      root.getAttribute("data-color-mode") ||
      body?.getAttribute("data-theme") ||
      body?.getAttribute("data-appearance") ||
      ""
    ).toLowerCase();
    if (dataTheme.includes("dark")) return "dark";
    if (dataTheme.includes("light")) return "light";

    // Radios in profile menu (if present in DOM)
    const checked = document.querySelector('input[name="appearance-theme"]:checked');
    if (checked) {
      const label = (checked.getAttribute("aria-label") || checked.value || "").toLowerCase();
      if (label.includes("暗") || label.includes("dark")) return "dark";
      if (label.includes("浅") || label.includes("light")) return "light";
      if (label.includes("系统") || label.includes("system")) {
        return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
      }
    }

    // The skin itself declares color-scheme on :root.  Once installed,
    // reading getComputedStyle(root) directly would therefore keep `auto`
    // themes locked to the previous shell mode. Temporarily remove only our
    // own root class/attribute, sample the native computed scheme, then restore
    // synchronously. Mutation records created by this probe are drained below
    // so the root observer does not schedule a redundant ensure pass.
    try {
      const hadSkin = root.classList.contains("codex-dream-skin");
      const savedShell = root.getAttribute(SHELL_ATTR);
      samplingNativeShell = true;
      if (hadSkin) root.classList.remove("codex-dream-skin");
      if (savedShell !== null) root.removeAttribute(SHELL_ATTR);
      let colorScheme = "";
      try {
        colorScheme = getComputedStyle(root).colorScheme || "";
      } finally {
        if (hadSkin) root.classList.add("codex-dream-skin");
        if (savedShell !== null) root.setAttribute(SHELL_ATTR, savedShell);
        rootObserver?.takeRecords?.();
        samplingNativeShell = false;
      }
      if (colorScheme.includes("dark") && !colorScheme.includes("light")) return "dark";
      if (colorScheme.includes("light") && !colorScheme.includes("dark")) return "light";
    } catch {
      samplingNativeShell = false;
    }

    try {
      return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    } catch {}

    // Only use surface luminance before the skin owns those surfaces. Sampling
    // our own translucent layers would create route-dependent light/dark flips.
    if (!root.classList.contains("codex-dream-skin")) {
      const samples = [
        body,
        document.querySelector("main.main-surface"),
        document.querySelector("aside.app-shell-left-panel"),
      ].filter(Boolean);
      let votesLight = 0;
      let votesDark = 0;
      for (const el of samples) {
        try {
          const rgb = parseRgb(getComputedStyle(el).backgroundColor);
          if (!rgb) continue;
          const L = luminance(rgb);
          if (L >= 0.55) votesLight += 1;
          else if (L <= 0.25) votesDark += 1;
        } catch {}
      }
      if (votesLight > votesDark) return "light";
      if (votesDark > votesLight) return "dark";
    }
    return "light";
  };

  const makeAdaptivePalette = (sample, shell) => {
    const source = sample || { r: 108, g: 126, b: 136 };
    const hsl = rgbToHsl(source);
    const hue = hsl.s < 0.12 ? 214 : hsl.h;
    const saturation = clamp(hsl.s, 0.38, 0.72);
    const accent = hslToRgb({ h: hue, s: saturation, l: shell === "light" ? 0.42 : 0.66 });
    const accentAlt = hslToRgb({ h: hue + 12, s: saturation * 0.82, l: shell === "light" ? 0.52 : 0.73 });
    const secondary = hslToRgb({ h: hue - 24, s: saturation * 0.64, l: shell === "light" ? 0.56 : 0.62 });
    const highlight = hslToRgb({ h: hue + 24, s: saturation * 0.76, l: shell === "light" ? 0.36 : 0.58 });
    const neutral = (lightness, chroma = 0.08) => rgbToHex(hslToRgb({ h: hue, s: chroma, l: lightness }));
    return shell === "light" ? {
      background: neutral(0.965, 0.07),
      panel: neutral(0.987, 0.035),
      panelAlt: neutral(0.945, 0.09),
      accent: rgbToHex(accent),
      accentAlt: rgbToHex(accentAlt),
      secondary: rgbToHex(secondary),
      highlight: rgbToHex(highlight),
      text: neutral(0.13, 0.10),
      muted: neutral(0.42, 0.08),
      line: `rgba(${Math.round(accent.r)}, ${Math.round(accent.g)}, ${Math.round(accent.b)}, .24)`,
    } : {
      background: neutral(0.055, 0.045),
      panel: neutral(0.085, 0.04),
      panelAlt: neutral(0.125, 0.05),
      accent: rgbToHex(accent),
      accentAlt: rgbToHex(accentAlt),
      secondary: rgbToHex(secondary),
      highlight: rgbToHex(highlight),
      text: neutral(0.93, 0.025),
      muted: neutral(0.69, 0.03),
      line: `rgba(${Math.round(accent.r)}, ${Math.round(accent.g)}, ${Math.round(accent.b)}, .28)`,
    };
  };

  const resolvedShell = () => {
    if (THEME.appearance === "light" || THEME.appearance === "dark") return THEME.appearance;
    // Image luminance may tune accents and scrims, but auto appearance follows
    // Codex/ChatGPT (or the OS fallback) so a bright wallpaper cannot flip a
    // native dark session back to a light shell after analysis.
    return detectShellMode();
  };

  const applyTheme = (root, shell) => {
    const colors = THEME.colors || {};
    const explicit = new Set(Array.isArray(THEME.explicitColorKeys) ? THEME.explicitColorKeys : []);
    const adaptive = makeAdaptivePalette(artAnalysis?.accentRgb, shell);
    const legacyLight = !THEME.appearance && shell === "light";
    const structural = new Set(["background", "panel", "panelAlt", "text", "muted"]);
    const pick = (name) => {
      const allowExplicit = explicit.has(name) && !(legacyLight && structural.has(name));
      return allowExplicit && typeof colors[name] === "string" ? colors[name] : adaptive[name];
    };
    const accent = pick("accent");
    const accentAlt = explicit.has("accentAlt") ? pick("accentAlt") : (explicit.has("accent") ? accent : adaptive.accentAlt);
    const variables = {
      "--ds-bg": pick("background"),
      "--ds-panel": pick("panel"),
      "--ds-panel-2": pick("panelAlt"),
      "--ds-green": accent,
      "--ds-lime": accentAlt,
      "--ds-cyan": pick("secondary"),
      "--ds-purple": pick("highlight"),
      "--ds-text": pick("text"),
      "--ds-muted": pick("muted"),
      "--ds-line": explicit.has("line") && typeof colors.line === "string" ? colors.line : adaptive.line,
    };

    for (const [name, value] of Object.entries(variables)) {
      if (typeof value === "string" && value) setStyleProperty(root, name, value);
    }
    const rgbVariables = {
      "--ds-bg-rgb": variables["--ds-bg"],
      "--ds-panel-rgb": variables["--ds-panel"],
      "--ds-panel-2-rgb": variables["--ds-panel-2"],
      "--ds-accent-rgb": variables["--ds-green"],
      "--ds-accent-alt-rgb": variables["--ds-lime"],
      "--ds-secondary-rgb": variables["--ds-cyan"],
      "--ds-highlight-rgb": variables["--ds-purple"],
      "--ds-text-rgb": variables["--ds-text"],
      "--ds-muted-rgb": variables["--ds-muted"],
      "--ds-line-rgb": variables["--ds-line"],
    };
    for (const [name, value] of Object.entries(rgbVariables)) {
      const rgb = rgbString(value);
      if (rgb) setStyleProperty(root, name, rgb);
    }
    setStyleProperty(root, "--dream-skin-name", cssString(THEME.name || "Codex Dream Skin"));
    setStyleProperty(root, "--dream-skin-tagline", cssString(THEME.tagline || "Make something wonderful."));
    setStyleProperty(root, "--dream-skin-project-prefix", cssString(THEME.projectPrefix || "选择项目 · "));
    setStyleProperty(root, "--dream-skin-project-label", cssString(THEME.projectLabel || "◉  选择项目"));
  };

  const applyArtMetadata = (root) => {
    const profile = artAnalysis || ART_METADATA;
    const inferredSafe = profile?.safeArea || "center";
    const safeArea = ART.safeArea && ART.safeArea !== "auto" ? ART.safeArea : inferredSafe;
    const canonicalSafe = ["left", "right", "center", "none"].includes(safeArea)
      ? safeArea : "center";
    const focusX = typeof ART.focusX === "number" ? ART.focusX
      : profile?.focusX ?? (safeArea === "left" ? 0.72 : safeArea === "right" ? 0.28 : 0.5);
    const focusY = typeof ART.focusY === "number" ? ART.focusY : profile?.focusY ?? 0.5;
    const taskMode = ART.taskMode && ART.taskMode !== "auto"
      ? ART.taskMode : profile?.taskMode || "ambient";
    const wide = profile?.wide || false;
    const aspect = profile?.aspect || "unknown";
    const focusXValue = `${(clamp(focusX, 0, 1) * 100).toFixed(2)}%`;
    const focusYValue = `${(clamp(focusY, 0, 1) * 100).toFixed(2)}%`;

    setAttribute(root, "data-dream-art-wide", wide ? "true" : "false");
    setAttribute(root, "data-dream-art-safe", canonicalSafe);
    setAttribute(root, "data-dream-task-mode", taskMode);
    setAttribute(root, "data-dream-art-safe-area", safeArea);
    setAttribute(root, "data-dream-art-task-mode", taskMode);
    setAttribute(root, "data-dream-art-aspect", aspect);
    setAttribute(root, "data-dream-art-ready", artAnalysis ? "true" : "false");
    setStyleProperty(root, "--dream-art-focus-x", focusXValue);
    setStyleProperty(root, "--dream-art-focus-y", focusYValue);
    setStyleProperty(root, "--dream-art-position", `${focusXValue} ${focusYValue}`);
    setStyleProperty(root, "--dream-skin-focus-x", focusXValue);
    setStyleProperty(root, "--dream-skin-focus-y", focusYValue);
    setStyleProperty(root, "--dream-skin-art-position", `${focusXValue} ${focusYValue}`);
  };

  const applyDecor = (root) => {
    const preset = DECOR.preset === "tiger-festival" ? "tiger-festival" : "none";
    const intensity = preset === "tiger-festival" && typeof DECOR.intensity === "number"
      ? clamp(DECOR.intensity, 0, 1)
      : preset === "tiger-festival" ? 0.78 : 0;
    setAttribute(root, "data-dream-decor", preset);
    setStyleProperty(root, "--dream-skin-decor-intensity", intensity.toFixed(2));
  };

  const analyzeArt = () => new Promise((resolve) => {
    const startedAt = now();
    metrics.analysisRuns += 1;
    if (typeof window.Image !== "function" || !document?.createElement) {
      metrics.analysisMs = Number((now() - startedAt).toFixed(3));
      resolve(null);
      return;
    }
    const image = new window.Image();
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      if (analysisTimer) clearTimeout(analysisTimer);
      analysisTimer = null;
      metrics.analysisMs = Number((now() - startedAt).toFixed(3));
      resolve(value);
    };
    analysisTimer = setTimeout(() => finish(null), 6000);
    image.onerror = () => finish(null);
    image.onload = () => {
      try {
        const ratio = image.naturalWidth / image.naturalHeight;
        if (!Number.isFinite(ratio) || ratio <= 0) throw new Error("Invalid image dimensions");
        const maxDimension = 96;
        const width = Math.max(16, Math.round(ratio >= 1 ? maxDimension : maxDimension * ratio));
        const height = Math.max(16, Math.round(ratio >= 1 ? maxDimension / ratio : maxDimension));
        const canvas = document.createElement("canvas");
        canvas.width = width;
        canvas.height = height;
        const context = canvas.getContext?.("2d", { willReadFrequently: true });
        if (!context) throw new Error("Canvas is unavailable");
        context.drawImage(image, 0, 0, width, height);
        const data = context.getImageData(0, 0, width, height).data;
        const samples = new Array(width * height);
        const bins = Array.from({ length: 24 }, () => ({ weight: 0, r: 0, g: 0, b: 0 }));
        let lightTotal = 0;
        let count = 0;

        for (let y = 0; y < height; y += 1) {
          for (let x = 0; x < width; x += 1) {
            const offset = (y * width + x) * 4;
            if (data[offset + 3] < 32) continue;
            const rgb = { r: data[offset], g: data[offset + 1], b: data[offset + 2] };
            const light = (0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b) / 255;
            const hsl = rgbToHsl(rgb);
            samples[y * width + x] = { light, saturation: hsl.s };
            lightTotal += light;
            count += 1;
            if (hsl.s >= 0.16 && hsl.l >= 0.16 && hsl.l <= 0.86) {
              const bin = bins[Math.min(23, Math.floor(hsl.h / 15))];
              const weight = hsl.s * (1 - Math.abs(hsl.l - 0.52) * 0.85);
              bin.weight += weight;
              bin.r += rgb.r * weight;
              bin.g += rgb.g * weight;
              bin.b += rgb.b * weight;
            }
          }
        }
        if (!count) throw new Error("Image has no visible pixels");
        const brightness = lightTotal / count;
        const information = (start, end) => {
          let total = 0;
          let totalSquared = 0;
          let edges = 0;
          let edgeCount = 0;
          let pixels = 0;
          for (let y = 0; y < height; y += 1) {
            for (let x = start; x < end; x += 1) {
              const sample = samples[y * width + x];
              if (!sample) continue;
              total += sample.light;
              totalSquared += sample.light * sample.light;
              pixels += 1;
              const previous = x > start ? samples[y * width + x - 1] : null;
              const above = y > 0 ? samples[(y - 1) * width + x] : null;
              if (previous) { edges += Math.abs(sample.light - previous.light); edgeCount += 1; }
              if (above) { edges += Math.abs(sample.light - above.light); edgeCount += 1; }
            }
          }
          const mean = pixels ? total / pixels : 0;
          const variance = pixels ? Math.max(0, totalSquared / pixels - mean * mean) : 1;
          return Math.sqrt(variance) * 0.58 + (edgeCount ? edges / edgeCount : 1) * 0.42;
        };
        const zoneWidth = Math.max(1, Math.floor(width * 0.38));
        const leftInformation = information(0, zoneWidth);
        const rightInformation = information(width - zoneWidth, width);
        let safeArea = "center";
        if (leftInformation < rightInformation * 0.86) safeArea = "left";
        else if (rightInformation < leftInformation * 0.86) safeArea = "right";

        let saliencyTotal = 0;
        let saliencyX = 0;
        let saliencyY = 0;
        for (let y = 0; y < height; y += 1) {
          for (let x = 0; x < width; x += 1) {
            const sample = samples[y * width + x];
            if (!sample) continue;
            const previous = x > 0 ? samples[y * width + x - 1] : null;
            const above = y > 0 ? samples[(y - 1) * width + x] : null;
            const edge = (previous ? Math.abs(sample.light - previous.light) : 0) +
              (above ? Math.abs(sample.light - above.light) : 0);
            const weight = 0.01 + Math.abs(sample.light - brightness) * 0.48 +
              sample.saturation * 0.34 + edge * 0.28;
            saliencyTotal += weight;
            saliencyX += (x + 0.5) / width * weight;
            saliencyY += (y + 0.5) / height * weight;
          }
        }
        let focusX = saliencyTotal ? saliencyX / saliencyTotal : 0.5;
        let focusY = saliencyTotal ? saliencyY / saliencyTotal : 0.5;
        if (safeArea === "left") focusX = Math.max(0.64, focusX);
        if (safeArea === "right") focusX = Math.min(0.36, focusX);
        focusX = clamp(focusX, 0.12, 0.88);
        focusY = clamp(focusY, 0.18, 0.82);

        const accentBin = bins.reduce((best, candidate) => candidate.weight > best.weight ? candidate : best, bins[0]);
        const accentRgb = accentBin.weight > 0 ? {
          r: accentBin.r / accentBin.weight,
          g: accentBin.g / accentBin.weight,
          b: accentBin.b / accentBin.weight,
        } : null;
        const aspect = ratio >= 2.25 ? "ultrawide" : ratio >= 1.45 ? "wide"
          : ratio >= 1.08 ? "landscape" : ratio >= 0.9 ? "square" : "portrait";
        finish({
          width: image.naturalWidth,
          height: image.naturalHeight,
          ratio,
          wide: ratio >= 1.75,
          aspect,
          brightness,
          shell: brightness >= 0.58 ? "light" : "dark",
          safeArea,
          focusX,
          focusY,
          taskMode: ratio >= 2.25 ? "banner" : "ambient",
          accentRgb,
        });
      } catch {
        finish(null);
      }
    };
    image.src = artUrl;
  });

  let chromeParts = null;
  let observedShellMain = null;
  let resizeObserver = null;

  const ensureStyle = (root) => {
    let style = document.getElementById(STYLE_ID);
    if (!style) {
      style = document.createElement("style");
      style.id = STYLE_ID;
      style.textContent = cssText;
      style.dataset.dreamSkinVersion = VERSION;
      (document.head || root).appendChild(style);
    } else if (style.dataset.dreamSkinStyleRevision !== STYLE_REVISION) {
      style.textContent = cssText;
    }
    style.dataset.dreamSkinVersion = VERSION;
    style.dataset.dreamSkinStyleRevision = STYLE_REVISION;
    return style;
  };

  const applyRootState = (root) => {
    metrics.rootPasses += 1;
    ensureStyle(root);
    const shell = resolvedShell();
    setAttribute(root, SHELL_ATTR, shell);
    setStyleProperty(root, "--dream-skin-art", `url("${artUrl}")`);
    applyTheme(root, shell);
    applyArtMetadata(root);
    applyDecor(root);
    root.classList.add("codex-dream-skin");
    return shell;
  };

  const syncOutputsPanel = () => {
    const currentPanels = new Set();
    for (const title of document.querySelectorAll("span.truncate")) {
      if ((title.textContent || "").trim() !== "Outputs") continue;
      const section = title.closest?.("section");
      const content = section?.parentElement;
      const panel = content?.parentElement;
      if (!section || !content || !panel?.classList) continue;
      const hasSources = [...content.querySelectorAll("span.truncate")]
        .some((candidate) => (candidate.textContent || "").trim() === "Sources");
      if (hasSources) currentPanels.add(panel);
    }
    for (const panel of document.querySelectorAll(`.${OUTPUTS_PANEL_CLASS}`)) {
      if (!currentPanels.has(panel)) panel.classList.remove(OUTPUTS_PANEL_CLASS);
    }
    for (const panel of currentPanels) panel.classList.add(OUTPUTS_PANEL_CLASS);
  };

  const syncPreviewPanel = () => {
    const currentPanels = new Set();
    const previewLabels = new Set(["Side chat", "Browser", "Terminal"]);
    const structuralCandidates = document.querySelectorAll("main.main-surface aside");
    for (const panel of structuralCandidates) {
      if (
        panel.classList?.contains("z-[41]") &&
        panel.classList.contains("ml-auto") &&
        panel.classList.contains("overflow-visible")
      ) {
        currentPanels.add(panel);
      }
    }
    for (const list of document.querySelectorAll("ul")) {
      const labels = new Set(
        [...list.querySelectorAll("span")]
          .map((candidate) => (candidate.textContent || "").trim())
          .filter((label) => previewLabels.has(label))
      );
      if (labels.size !== previewLabels.size) continue;
      const panel = list.closest?.("aside");
      if (panel?.classList) currentPanels.add(panel);
    }
    for (const panel of document.querySelectorAll(`.${PREVIEW_PANEL_CLASS}`)) {
      if (!currentPanels.has(panel)) {
        panel.classList.remove(PREVIEW_PANEL_CLASS);
        panel.classList.remove(PREVIEW_LAUNCHER_CLASS);
      }
    }
    for (const panel of currentPanels) {
      const labels = new Set(
        [...panel.querySelectorAll("span")]
          .map((candidate) => (candidate.textContent || "").trim())
          .filter((label) => previewLabels.has(label))
      );
      panel.classList.add(PREVIEW_PANEL_CLASS);
      panel.classList.toggle(
        PREVIEW_LAUNCHER_CLASS,
        labels.has("Browser") && labels.has("Terminal")
      );
    }
  };

  const syncRouteState = (shell, { layout = false } = {}) => {
    metrics.routePasses += 1;
    const root = document.documentElement;
    if (!root) return;
    shell ||= root.getAttribute(SHELL_ATTR) || resolvedShell();
    const shellMain = document.querySelector("main.main-surface") || document.querySelector("main");
    const homeIndicator = document.querySelector('[data-testid="home-icon"]');
    const home = homeIndicator?.closest('[role="main"]') ||
      [...document.querySelectorAll('[role="main"]')].find((candidate) =>
        candidate.querySelector('[data-feature="game-source"]') &&
        candidate.querySelector('.group\\\\/home-suggestions')) || null;
    for (const candidate of document.querySelectorAll('[role="main"].dream-skin-home')) {
      if (candidate !== home) candidate.classList.remove("dream-skin-home");
    }
    if (home) home.classList.add("dream-skin-home");
    const homeUtilityBars = new Set(home
      ? home.querySelectorAll('[class*="_homeUtilityBar_"]')
      : []);
    for (const candidate of document.querySelectorAll(".dream-skin-home-utility")) {
      if (!homeUtilityBars.has(candidate)) candidate.classList.remove("dream-skin-home-utility");
    }
    for (const candidate of homeUtilityBars) candidate.classList.add("dream-skin-home-utility");
    syncOutputsPanel();
    syncPreviewPanel();

    if (!shellMain || !document.body) return;
    if (observedShellMain !== shellMain) {
      resizeObserver?.disconnect();
      resizeObserver?.observe(shellMain);
      observedShellMain = shellMain;
      layout = true;
    }
    shellMain.classList.toggle("dream-skin-home-shell", Boolean(home));
    let chrome = document.getElementById(CHROME_ID);
    const currentChrome = chrome?.parentElement === document.body &&
      [
        ".dream-skin-decor-ribbon",
        ".dream-skin-decor-cloud",
        ".dream-skin-decor-seal",
        ".dream-skin-decor-accent",
      ].every((selector) => chrome.querySelector(selector));
    let created = false;
    if (!currentChrome) {
      chrome?.remove();
      chrome = document.createElement("div");
      chrome.id = CHROME_ID;
      chrome.setAttribute("aria-hidden", "true");
      chrome.innerHTML = `
        <div class="dream-skin-brand">
          <span class="dream-skin-portal-mark">◉</span>
          <span><b></b><small></small></span>
        </div>
        <div class="dream-skin-status"><i></i><span></span></div>
        <div class="dream-skin-quote"></div>
        <div class="dream-skin-particles"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>
        <div class="dream-skin-orbit"></div>
        <div class="dream-skin-decor">
          <img class="dream-skin-decor-ribbon" alt="" draggable="false">
          <img class="dream-skin-decor-cloud" alt="" draggable="false">
          <img class="dream-skin-decor-seal" alt="" draggable="false">
          <img class="dream-skin-decor-accent" alt="" draggable="false">
        </div>`;
      document.body.appendChild(chrome);
      created = true;
      chromeParts = null;
    }
    if (!chromeParts || chromeParts.chrome !== chrome) {
      chromeParts = {
        chrome,
        name: chrome.querySelector(".dream-skin-brand b"),
        subtitle: chrome.querySelector(".dream-skin-brand small"),
        status: chrome.querySelector(".dream-skin-status span"),
        quote: chrome.querySelector(".dream-skin-quote"),
        decorRibbon: chrome.querySelector(".dream-skin-decor-ribbon"),
        decorCloud: chrome.querySelector(".dream-skin-decor-cloud"),
        decorSeal: chrome.querySelector(".dream-skin-decor-seal"),
        decorAccent: chrome.querySelector(".dream-skin-decor-accent"),
      };
    }
    setTextContent(chromeParts.name, THEME.name || "Codex Dream Skin");
    setTextContent(chromeParts.subtitle, THEME.brandSubtitle || "CODEX DREAM SKIN");
    setTextContent(chromeParts.status, THEME.statusText || "DREAM SKIN ONLINE");
    setTextContent(chromeParts.quote, THEME.quote || "MAKE SOMETHING WONDERFUL");
    setImageSource(chromeParts.decorRibbon, decorUrls.ribbon);
    setImageSource(chromeParts.decorCloud, decorUrls.cloud);
    setImageSource(chromeParts.decorSeal, decorUrls.seal);
    setImageSource(chromeParts.decorAccent, decorUrls.accent);
    if (layout || created) {
      metrics.layoutReads += 1;
      const shellBox = shellMain.getBoundingClientRect();
      setStyleProperty(chrome, "left", `${Math.round(shellBox.left)}px`);
      setStyleProperty(chrome, "top", `${Math.round(shellBox.top)}px`);
      setStyleProperty(chrome, "width", `${Math.round(shellBox.width)}px`);
      setStyleProperty(chrome, "height", `${Math.round(shellBox.height)}px`);
    }
    chrome.classList.toggle("dream-skin-home-shell", Boolean(home));
    if (chrome.dataset.dreamShell !== shell) {
      chrome.dataset.dreamShell = shell;
      metrics.attributeWrites += 1;
    }
  };

  const ensure = ({ root: rootPass = true, route = true, layout = true } = {}) => {
    if (window[DISABLED_KEY]) return;
    const root = document.documentElement;
    if (!root) return;
    metrics.ensureCalls += 1;
    const shell = rootPass ? applyRootState(root) : null;
    if (route) syncRouteState(shell, { layout });
  };

  const cleanup = () => {
    const state = window[STATE_KEY];
    if (state?.installToken !== installToken) return false;
    window[DISABLED_KEY] = true;
    document.documentElement?.classList.remove("codex-dream-skin");
    document.documentElement?.removeAttribute(SHELL_ATTR);
    for (const name of ART_ATTRS) document.documentElement?.removeAttribute(name);
    document.documentElement?.style.removeProperty("--dream-skin-art");
    for (const name of THEME_VARIABLES) document.documentElement?.style.removeProperty(name);
    document.querySelectorAll(".dream-skin-home").forEach((node) => node.classList.remove("dream-skin-home"));
    document.querySelectorAll(".dream-skin-home-shell").forEach((node) => node.classList.remove("dream-skin-home-shell"));
    document.querySelectorAll(".dream-skin-home-utility").forEach((node) => node.classList.remove("dream-skin-home-utility"));
    document.querySelectorAll(`.${OUTPUTS_PANEL_CLASS}`).forEach((node) => node.classList.remove(OUTPUTS_PANEL_CLASS));
    document.querySelectorAll(`.${PREVIEW_PANEL_CLASS}`).forEach((node) => {
      node.classList.remove(PREVIEW_PANEL_CLASS);
      node.classList.remove(PREVIEW_LAUNCHER_CLASS);
    });
    document.getElementById(STYLE_ID)?.remove();
    document.getElementById(CHROME_ID)?.remove();
    state?.observer?.disconnect();
    state?.rootObserver?.disconnect();
    state?.resizeObserver?.disconnect();
    if (state?.timer) clearInterval(state.timer);
    if (state?.scheduler?.timeout) clearTimeout(state.scheduler.timeout);
    if (state?.scheduler?.frame != null && typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(state.scheduler.frame);
    }
    if (analysisTimer) clearTimeout(analysisTimer);
    if (state?.resizeHandler) window.removeEventListener("resize", state.resizeHandler);
    if (state?.mediaHandler && state?.mediaQuery) {
      try { state.mediaQuery.removeEventListener("change", state.mediaHandler); } catch {}
    }
    if (state?.artUrl) URL.revokeObjectURL(state.artUrl);
    for (const url of Object.values(state?.decorUrls || {})) URL.revokeObjectURL(url);
    delete window[STATE_KEY];
    return true;
  };

  const scheduler = { timeout: null, frame: null, root: false, route: false, layout: false };
  const flushScheduledEnsure = () => {
    if (scheduler.frame !== null && typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(scheduler.frame);
    }
    if (scheduler.timeout) clearTimeout(scheduler.timeout);
    scheduler.frame = null;
    scheduler.timeout = null;
    const pending = { root: scheduler.root, route: scheduler.route, layout: scheduler.layout };
    scheduler.root = false;
    scheduler.route = false;
    scheduler.layout = false;
    ensure(pending);
  };
  const scheduleEnsure = ({ root = false, route = true, layout = false } = {}) => {
    scheduler.root ||= root;
    scheduler.route ||= route;
    scheduler.layout ||= layout;
    if (scheduler.timeout || scheduler.frame !== null) return;
    if (typeof requestAnimationFrame === "function") {
      scheduler.frame = requestAnimationFrame(flushScheduledEnsure);
      scheduler.timeout = setTimeout(flushScheduledEnsure, 96);
    } else {
      scheduler.timeout = setTimeout(flushScheduledEnsure, 64);
    }
  };
  const observer = new MutationObserver(() => scheduleEnsure({ route: true }));
  rootObserver = new MutationObserver(() => {
    if (samplingNativeShell) return;
    scheduleEnsure({ root: true, route: true });
  });
  const resizeHandler = () => scheduleEnsure({ route: true, layout: true });
  if (typeof ResizeObserver === "function") {
    resizeObserver = new ResizeObserver(() => scheduleEnsure({ route: true, layout: true }));
  }

  let mediaQuery = null;
  let mediaHandler = null;
  try {
    mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    mediaHandler = () => scheduleEnsure({ root: true, route: true });
  } catch {}

  window[STATE_KEY] = {
    ensure,
    cleanup,
    observer,
    rootObserver,
    resizeObserver,
    timer: null,
    scheduler,
    resizeHandler,
    mediaQuery,
    mediaHandler,
    artUrl,
    decorUrls,
    installToken,
    analysis: artAnalysis,
    artMetadata: ART_METADATA,
    metrics,
    version: VERSION,
    themeId: THEME.id || "custom",
    detectShellMode,
  };
  const firstEnsureStartedAt = now();
  ensure({ layout: !previous || !document.getElementById(CHROME_ID) });
  metrics.firstEnsureMs = Number((now() - firstEnsureStartedAt).toFixed(3));
  if (previous?.artUrl && previous.artUrl !== artUrl) URL.revokeObjectURL(previous.artUrl);
  for (const url of Object.values(previous?.decorUrls || {})) {
    if (!Object.values(decorUrls).includes(url)) URL.revokeObjectURL(url);
  }

  observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
  });
  rootObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class", "data-theme", "data-appearance", "data-color-mode", "style"],
  });
  if (document.body) {
    rootObserver.observe(document.body, {
      attributes: true,
      attributeFilter: ["class", "data-theme", "data-appearance", "data-color-mode", "style"],
    });
  }
  const timer = setInterval(() => ensure(), 4000);
  window[STATE_KEY].timer = timer;
  window.addEventListener("resize", resizeHandler, { passive: true });
  if (mediaHandler && mediaQuery) {
    mediaQuery.addEventListener("change", mediaHandler);
  }
  const analysisPromise = artAnalysis ? Promise.resolve(null) : analyzeArt();
  window[STATE_KEY].analysisTimer = analysisTimer;
  analysisPromise.then((analysis) => {
    const state = window[STATE_KEY];
    if (!analysis || state?.installToken !== installToken || window[DISABLED_KEY]) return;
    artAnalysis = analysis;
    state.analysis = analysis;
    if (typeof THEME.artKey === "string") {
      analysisCache.set(THEME.artKey, analysis);
      while (analysisCache.size > 8) analysisCache.delete(analysisCache.keys().next().value);
    }
    ensure({ root: true, route: false, layout: false });
  }).catch(() => {});
  return {
    installed: true,
    version: VERSION,
    themeId: THEME.id || "custom",
    shell: resolvedShell(),
    analysis: artAnalysis,
  };
})(__DREAM_SKIN_CSS_JSON__, __DREAM_SKIN_ART_JSON__, __DREAM_SKIN_THEME_JSON__)
