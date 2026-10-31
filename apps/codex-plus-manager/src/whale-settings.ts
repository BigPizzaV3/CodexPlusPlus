/**
 * 大肥鱼（旧版 DeepSeek 鲸鱼用量挂件）的余额配置，与 Rust `codexAppWhaleBalance*` 对应。
 *
 * 大肥鱼现在是「Agent 桌宠」的一种类型（`agentPet.kind === "whaleWidget"`），
 * 原来的 `codexAppWhaleWidgetEnabled` 开关已并入 `agentPet`，这里只保留余额相关字段。
 */
export type WhaleBalanceProtocol = "auto" | "custom" | "off";

export type WhaleBalanceSettings = {
  codexAppWhaleBalanceProtocol: WhaleBalanceProtocol;
  codexAppWhaleBalancePath: string;
  codexAppWhaleBalanceField: string;
  codexAppWhaleBalanceCurrency: string;
  codexAppWhaleBalanceScale: number;
};

export function defaultWhaleBalanceSettings(): WhaleBalanceSettings {
  return {
    codexAppWhaleBalanceProtocol: "auto",
    codexAppWhaleBalancePath: "",
    codexAppWhaleBalanceField: "",
    codexAppWhaleBalanceCurrency: "USD",
    codexAppWhaleBalanceScale: 1,
  };
}

export function normalizeWhaleBalanceSettings(raw: Partial<WhaleBalanceSettings>): WhaleBalanceSettings {
  const protocol = raw.codexAppWhaleBalanceProtocol;
  return {
    codexAppWhaleBalanceProtocol: protocol === "custom" || protocol === "off" ? protocol : "auto",
    codexAppWhaleBalancePath: String(raw.codexAppWhaleBalancePath ?? "").trim(),
    codexAppWhaleBalanceField: String(raw.codexAppWhaleBalanceField ?? "").trim(),
    codexAppWhaleBalanceCurrency: String(raw.codexAppWhaleBalanceCurrency ?? "USD").trim().toUpperCase(),
    codexAppWhaleBalanceScale: raw.codexAppWhaleBalanceScale ?? 1,
  };
}

export type WhaleBalanceIssue = "path" | "field" | "currency" | "scale";

/** 只有大肥鱼正在使用（`whaleActive`）且选了自定义余额接口时才校验。 */
export function whaleBalanceSettingsIssue(settings: WhaleBalanceSettings, whaleActive: boolean): WhaleBalanceIssue | null {
  if (!whaleActive || settings.codexAppWhaleBalanceProtocol !== "custom") return null;
  const path = settings.codexAppWhaleBalancePath;
  if (!path || path.length > 1024 || path.startsWith("//") || /[\\?#\s\x00-\x1f]/.test(path) || /:/.test(path)) return "path";
  const field = settings.codexAppWhaleBalanceField;
  if (!field || field.length > 256 || !/^[A-Za-z0-9_-]+(?:\[\d+\])*(?:\.[A-Za-z0-9_-]+(?:\[\d+\])*)*$/.test(field)) return "field";
  if (!/^[A-Z]{3}$/.test(settings.codexAppWhaleBalanceCurrency)) return "currency";
  if (!Number.isFinite(settings.codexAppWhaleBalanceScale) || settings.codexAppWhaleBalanceScale <= 0 || settings.codexAppWhaleBalanceScale > 1e12) return "scale";
  return null;
}
