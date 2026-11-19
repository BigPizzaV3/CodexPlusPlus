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

// 预设与校验在 Codex 设置页（46-service-settings.js）里；管理器只负责读写时补默认值。
const defaultEndpoint = { baseUrl: "https://api.groq.com/openai/v1", model: "whisper-large-v3-turbo" };

export function defaultDictationSettings(): DictationSettings {
  return {
    enabled: false,
    provider: "api",
    ...defaultEndpoint,
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
  const baseUrl = text(value?.baseUrl, defaults.baseUrl).replace(/\/+$/, "");
  // 早期版本保存 SenseVoice 预设时遗漏了 provider 字段，只留下了
  // `local://sensevoice` 标记。读配置时把这个旧格式恢复成离线 provider，
  // 避免它被当成远程 HTTP 转写服务校验。
  const provider = value?.provider === "sensevoice" || baseUrl.toLowerCase() === "local://sensevoice"
    ? "sensevoice"
    : "api";
  return {
    enabled: value?.enabled === true,
    provider,
    baseUrl,
    apiKey: text(value?.apiKey),
    apiKeyEnv: text(value?.apiKeyEnv),
    model: text(value?.model, defaults.model),
    language: text(value?.language),
    timeoutSeconds: typeof timeout === "number" && Number.isFinite(timeout)
      ? Math.min(600, Math.max(1, Math.round(timeout)))
      : defaults.timeoutSeconds,
  };
}
