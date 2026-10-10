export type DictationSettings = {
  enabled: boolean;
  provider: "api" | "sensevoice";
  baseUrl: string;
  apiKey: string;
  apiKeyEnv: string;
  model: string;
  language: string;
  timeoutSeconds: number;
};

export type DictationPreset = "groq" | "openai" | "local" | "sensevoice" | "custom";
export type DictationSettingsIssue = "baseUrl" | "model" | "apiKeyEnv";

const presets = {
  groq: { baseUrl: "https://api.groq.com/openai/v1", model: "whisper-large-v3-turbo" },
  openai: { baseUrl: "https://api.openai.com/v1", model: "whisper-1" },
  local: { baseUrl: "http://127.0.0.1:8000/v1", model: "whisper-1" },
  sensevoice: { baseUrl: "local://sensevoice", model: "SenseVoiceSmall (INT8)" },
};

export function defaultDictationSettings(): DictationSettings {
  return {
    enabled: false,
    provider: "api",
    ...presets.groq,
    apiKey: "",
    apiKeyEnv: "",
    language: "",
    timeoutSeconds: 120,
  };
}

export function normalizeDictationSettings(value: Partial<DictationSettings> | undefined): DictationSettings {
  const defaults = defaultDictationSettings();
  const text = (entry: unknown, fallback = "") => typeof entry === "string" ? entry.trim() : fallback;
  const timeout = value?.timeoutSeconds;
  return {
    enabled: value?.enabled === true,
    provider: value?.provider === "sensevoice" ? "sensevoice" : "api",
    baseUrl: text(value?.baseUrl, defaults.baseUrl).replace(/\/+$/, ""),
    apiKey: text(value?.apiKey),
    apiKeyEnv: text(value?.apiKeyEnv),
    model: text(value?.model, defaults.model),
    language: text(value?.language),
    timeoutSeconds: typeof timeout === "number" && Number.isFinite(timeout)
      ? Math.min(600, Math.max(1, Math.round(timeout)))
      : defaults.timeoutSeconds,
  };
}

export function dictationPreset(settings: DictationSettings): DictationPreset {
  if (settings.provider === "sensevoice") return "sensevoice";
  const baseUrl = settings.baseUrl.trim().replace(/\/+$/, "");
  for (const [name, preset] of Object.entries(presets)) {
    if (baseUrl === preset.baseUrl) return name as Exclude<DictationPreset, "custom">;
  }
  return "custom";
}

export function applyDictationPreset(settings: DictationSettings, preset: DictationPreset): DictationSettings {
  // 自定义保留当前输入，预设只调整语音服务的端点与模型。
  if (preset === "custom") return { ...settings };
  return {
    ...settings,
    provider: preset === "sensevoice" ? "sensevoice" : "api",
    ...presets[preset],
  };
}

export function dictationSettingsIssue(settings: DictationSettings): DictationSettingsIssue | null {
  if (!settings.enabled) return null;
  if (settings.provider === "sensevoice") return null;
  try {
    const url = new URL(settings.baseUrl);
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
      return "baseUrl";
    }
  } catch {
    return "baseUrl";
  }
  if (!settings.model.trim()) return "model";
  if (settings.apiKeyEnv.trim() && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(settings.apiKeyEnv.trim())) return "apiKeyEnv";
  return null;
}
