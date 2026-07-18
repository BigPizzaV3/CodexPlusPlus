export type DreamSkinColors = {
  background: string;
  panel: string;
  panelAlt: string;
  accent: string;
  accentAlt: string;
  secondary: string;
  highlight: string;
  text: string;
  muted: string;
  line: string;
  [key: string]: unknown;
};

export type DreamSkinThemeConfig = {
  schemaVersion: number;
  id: string;
  name: string;
  stylePreset?: string;
  brandSubtitle: string;
  tagline: string;
  projectPrefix: string;
  projectLabel: string;
  statusText: string;
  quote: string;
  colors?: DreamSkinColors;
  appearance?: "auto" | "light" | "dark";
  art?: {
    focusX?: number;
    focusY?: number;
    safeArea?: "auto" | "left" | "right" | "center" | "none";
    taskMode?: "auto" | "ambient" | "banner" | "off";
    [key: string]: unknown;
  };
  palette?: {
    accent?: string;
    [key: string]: unknown;
  };
  promoTitle?: string;
  promoSub?: string;
  promoUrl?: string;
  image?: string;
  [key: string]: unknown;
};

export type DreamSkinState = "pass" | "warning" | "fail" | "not_running";

export type DreamSkinCheckLevel = "pass" | "warning" | "fail";

export type DreamSkinCheck = {
  id: string;
  label: string;
  level: DreamSkinCheckLevel;
  message: string;
};

export type DreamSkinRuntimeStatus = {
  state: DreamSkinState;
  enabled: boolean;
  paused: boolean;
  liveApplied: boolean;
  checks: DreamSkinCheck[];
};

export type DreamSkinVerification = {
  state: DreamSkinState;
  pass: boolean;
  version: string | null;
  checks: DreamSkinCheck[];
  screenshotPath: string | null;
  raw: unknown;
};

export type DreamSkinImagePayload = {
  path: string;
  contentType: string;
  sizeBytes: number;
};

export type DreamSkinThemeKind = "builtin" | "stored" | "activeUnsaved";

export type DreamSkinThemeSummary = {
  key: string;
  id: string;
  name: string;
  previewPath: string;
  kind: DreamSkinThemeKind;
  builtin: boolean;
  active: boolean;
  modified: boolean;
};

export type DreamSkinThemeDraft = {
  config: DreamSkinThemeConfig;
  imagePath: string;
  builtin: boolean;
};

export type DreamSkinThemeLibrary = {
  themes: DreamSkinThemeSummary[];
  activeDraft: DreamSkinThemeDraft;
};

export type DreamSkinThemeActivationPayload = {
  library: DreamSkinThemeLibrary;
  runtime: DreamSkinRuntimeStatus;
  savedForNextLaunch: boolean;
};

export type DreamSkinCommandResult<T> = T & {
  status: string;
  message: string;
};

export type DreamSkinRuntimeResult = DreamSkinCommandResult<DreamSkinRuntimeStatus>;
export type DreamSkinVerificationResult = DreamSkinCommandResult<DreamSkinVerification>;
export type DreamSkinImageResult = DreamSkinCommandResult<DreamSkinImagePayload>;
export type DreamSkinThemeLibraryResult = DreamSkinCommandResult<DreamSkinThemeLibrary>;
export type DreamSkinThemeDraftResult = DreamSkinCommandResult<DreamSkinThemeDraft>;
export type DreamSkinThemeActivationResult = DreamSkinCommandResult<DreamSkinThemeActivationPayload>;

export type DreamSkinMarketTheme = {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  license: string;
  sourceUrl: string;
  tags: string[];
  theme: string;
  image: string;
  preview: string;
  themeSha256: string;
  imageSha256: string;
  previewUrl: string;
  installed: boolean;
  installedVersion: string;
  updateAvailable: boolean;
};

export type DreamSkinMarketResult = DreamSkinCommandResult<{
  schemaVersion: number;
  updatedAt: string;
  repositoryUrl: string;
  cached: boolean;
  warning: string;
  themes: DreamSkinMarketTheme[];
}>;

export function defaultDreamSkinColors(): DreamSkinColors {
  return {
    background: "#F7F4F5",
    panel: "#FFFFFF",
    panelAlt: "#FFF7F8",
    accent: "#E25563",
    accentAlt: "#F07A86",
    secondary: "#F3A8AF",
    highlight: "#C93D4C",
    text: "#2B2224",
    muted: "#8A7A7D",
    line: "rgba(196, 120, 128, .22)",
  };
}

export function defaultDreamSkinTheme(): DreamSkinThemeConfig {
  if (typeof navigator !== "undefined" && /\bWindows\b/i.test(navigator.userAgent)) {
    return {
      schemaVersion: 1,
      id: "preset-arina-hashimoto",
      name: "桥本有菜",
      brandSubtitle: "CODEX DREAM SKIN",
      tagline: "把柔光与玫瑰带进今天的工作台。",
      projectPrefix: "选择项目 · ",
      projectLabel: "◉  选择项目",
      statusText: "DREAM SKIN ONLINE",
      quote: "MAKE SOMETHING WONDERFUL",
      image: "dream-reference.jpg",
      appearance: "auto",
      art: {
        focusX: 0.72,
        focusY: 0.45,
        safeArea: "left",
        taskMode: "ambient",
      },
    };
  }
  return {
    schemaVersion: 1,
    id: "custom-1784123441349",
    name: "Dream Skin",
    brandSubtitle: "CODEX DREAM SKIN",
    tagline: "把喜欢的画面变成可交互的 Codex 工作台。",
    projectPrefix: "选择项目 · ",
    projectLabel: "◉  选择项目",
    statusText: "THEME ONLINE",
    quote: "Make something wonderful",
    colors: defaultDreamSkinColors(),
    image: "portal-hero.png",
    promoTitle: "感谢 Passion8 赞助",
    promoSub: "passion8.cc",
    promoUrl: "https://passion8.cc/register?aff=TuPe",
  };
}

const validColor = (value: unknown): value is string => {
  if (typeof value !== "string") return false;
  const color = value.trim();
  return /^#[0-9a-f]{3,8}$/i.test(color)
    || /^rgba?\(\s*[\d.]+\s*,\s*[\d.]+\s*,\s*[\d.]+(?:\s*,\s*[\d.]+)?\s*\)$/i.test(color);
};

const textOr = (value: unknown, fallback: string): string => {
  return typeof value === "string" && value.trim() ? value.trim() : fallback;
};

export function resolveDreamSkinStylePreset(id: string, stylePreset: unknown): string {
  const preset = typeof stylePreset === "string" ? stylePreset.trim() : "";
  if (preset && preset !== "dream-original") return preset;

  const legacyPresets: Record<string, string> = {
    "caishen-lite": "caishen-lite",
    "caishen-max": "caishen-max",
    "caishen-readable": "caishen-readable",
    "export-night": "export-night",
    "global-founder-bright": "global-founder-bright",
    "mythic-guardian-noir": "mythic-guardian-noir",
    "codex-snow-skin": "codex-snow",
    "glass-vision": "glass-vision",
    "preset-midnight-aurora": "midnight-aurora",
    "preset-amber-dusk": "amber-dusk",
    "preset-forest-mist": "forest-mist",
    "preset-cyber-neon": "cyber-neon",
    "preset-sakura-dawn": "sakura-dawn",
  };
  return legacyPresets[id.trim()] ?? "dream-original";
}

export function normalizeDreamSkinTheme(
  value: Partial<DreamSkinThemeConfig> | null | undefined,
): DreamSkinThemeConfig {
  const fallback = defaultDreamSkinTheme();
  const fallbackColors = fallback.colors ?? defaultDreamSkinColors();
  const colors = value?.colors as Partial<DreamSkinColors> | undefined;
  const colorOr = (candidate: unknown, defaultValue: string) => (
    validColor(candidate) ? candidate.trim() : defaultValue
  );

  const id = textOr(value?.id, fallback.id);
  const normalized: DreamSkinThemeConfig = {
    ...(value ?? {}),
    schemaVersion: value?.schemaVersion === 1 ? 1 : fallback.schemaVersion,
    id,
    name: textOr(value?.name, fallback.name),
    brandSubtitle: textOr(value?.brandSubtitle, fallback.brandSubtitle),
    tagline: textOr(value?.tagline, fallback.tagline),
    projectPrefix: textOr(value?.projectPrefix, fallback.projectPrefix),
    projectLabel: textOr(value?.projectLabel, fallback.projectLabel),
    statusText: textOr(value?.statusText, fallback.statusText),
    quote: textOr(value?.quote, fallback.quote),
  };
  if (typeof value?.stylePreset === "string" && value.stylePreset.trim()) {
    normalized.stylePreset = value.stylePreset.trim();
  } else {
    delete normalized.stylePreset;
  }
  if (colors) {
    normalized.colors = {
      ...colors,
      background: colorOr(colors?.background, fallbackColors.background),
      panel: colorOr(colors?.panel, fallbackColors.panel),
      panelAlt: colorOr(colors?.panelAlt, fallbackColors.panelAlt),
      accent: colorOr(colors?.accent, fallbackColors.accent),
      accentAlt: colorOr(colors?.accentAlt, fallbackColors.accentAlt),
      secondary: colorOr(colors?.secondary, fallbackColors.secondary),
      highlight: colorOr(colors?.highlight, fallbackColors.highlight),
      text: colorOr(colors?.text, fallbackColors.text),
      muted: colorOr(colors?.muted, fallbackColors.muted),
      line: colorOr(colors?.line, fallbackColors.line),
    } as DreamSkinColors;
  } else if (!value) {
    normalized.colors = fallbackColors;
  } else {
    delete normalized.colors;
  }
  return normalized;
}

export function dreamSkinDraftFingerprint(draft: DreamSkinThemeDraft): string {
  return JSON.stringify({
    config: normalizeDreamSkinTheme(draft.config),
    imagePath: draft.imagePath.trim(),
    builtin: draft.builtin,
  });
}

export function isDreamSkinDraftDirty(
  saved: DreamSkinThemeDraft,
  draft: DreamSkinThemeDraft,
): boolean {
  return dreamSkinDraftFingerprint(saved) !== dreamSkinDraftFingerprint(draft);
}
