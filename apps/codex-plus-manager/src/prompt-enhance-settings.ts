export type PromptEnhanceSettings = {
  enabled: boolean;
  useStepwiseConnection: boolean;
  baseUrl: string;
  apiKey: string;
  apiKeyEnv: string;
  protocol: string;
  model: string;
  systemPrompt: string;
  timeoutSeconds: number;
};

export type PromptEnhanceSettingsIssue = "baseUrl" | "model";

const protocols = new Set(["chat_completions", "responses", "anthropic_messages", "auto"]);

export function defaultPromptEnhanceSettings(): PromptEnhanceSettings {
  return {
    enabled: false,
    useStepwiseConnection: true,
    baseUrl: "",
    apiKey: "",
    apiKeyEnv: "",
    protocol: "chat_completions",
    model: "",
    systemPrompt: "",
    timeoutSeconds: 60,
  };
}

export function normalizePromptEnhanceSettings(value: Partial<PromptEnhanceSettings> | undefined): PromptEnhanceSettings {
  const defaults = defaultPromptEnhanceSettings();
  const text = (entry: unknown) => typeof entry === "string" ? entry.trim() : "";
  const protocol = text(value?.protocol);
  const timeout = value?.timeoutSeconds;
  return {
    enabled: value?.enabled === true,
    useStepwiseConnection: value?.useStepwiseConnection !== false,
    baseUrl: text(value?.baseUrl).replace(/\/+$/, ""),
    apiKey: text(value?.apiKey),
    apiKeyEnv: text(value?.apiKeyEnv),
    protocol: protocols.has(protocol) ? protocol : defaults.protocol,
    model: text(value?.model),
    systemPrompt: text(value?.systemPrompt),
    timeoutSeconds: typeof timeout === "number" && Number.isFinite(timeout)
      ? Math.min(300, Math.max(5, Math.round(timeout)))
      : defaults.timeoutSeconds,
  };
}

// 复用下一步建议连接时不校验本组字段，由后端按那一组配置报错。
export function promptEnhanceSettingsIssue(settings: PromptEnhanceSettings): PromptEnhanceSettingsIssue | null {
  if (!settings.enabled || settings.useStepwiseConnection) return null;
  if (!settings.baseUrl.trim()) return "baseUrl";
  if (!settings.model.trim()) return "model";
  return null;
}
