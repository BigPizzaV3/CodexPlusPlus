export type WhaleBalanceProtocol = "auto" | "custom" | "off";

export type WhaleBalanceSettings = {
  codexAppWhaleWidgetEnabled: boolean;
  codexAppWhaleBalanceProtocol: WhaleBalanceProtocol;
  codexAppWhaleBalancePath: string;
  codexAppWhaleBalanceField: string;
  codexAppWhaleBalanceCurrency: string;
  codexAppWhaleBalanceScale: number;
};

export function defaultWhaleBalanceSettings(): WhaleBalanceSettings {
  return {
    codexAppWhaleWidgetEnabled: false,
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
    codexAppWhaleWidgetEnabled: raw.codexAppWhaleWidgetEnabled === true,
    codexAppWhaleBalanceProtocol: protocol === "custom" || protocol === "off" ? protocol : "auto",
    codexAppWhaleBalancePath: String(raw.codexAppWhaleBalancePath ?? "").trim(),
    codexAppWhaleBalanceField: String(raw.codexAppWhaleBalanceField ?? "").trim(),
    codexAppWhaleBalanceCurrency: String(raw.codexAppWhaleBalanceCurrency ?? "USD").trim().toUpperCase(),
    codexAppWhaleBalanceScale: raw.codexAppWhaleBalanceScale ?? 1,
  };
}

export type WhaleBalanceIssue = "path" | "field" | "currency" | "scale";

export function whaleBalanceSettingsIssue(settings: WhaleBalanceSettings): WhaleBalanceIssue | null {
  if (!settings.codexAppWhaleWidgetEnabled || settings.codexAppWhaleBalanceProtocol !== "custom") return null;
  const path = settings.codexAppWhaleBalancePath;
  if (!path || path.length > 1024 || path.startsWith("//") || /[\\?#\s\x00-\x1f]/.test(path) || /:/.test(path)) return "path";
  const field = settings.codexAppWhaleBalanceField;
  if (!field || field.length > 256 || !/^[A-Za-z0-9_-]+(?:\[\d+\])*(?:\.[A-Za-z0-9_-]+(?:\[\d+\])*)*$/.test(field)) return "field";
  if (!/^[A-Z]{3}$/.test(settings.codexAppWhaleBalanceCurrency)) return "currency";
  if (!Number.isFinite(settings.codexAppWhaleBalanceScale) || settings.codexAppWhaleBalanceScale <= 0 || settings.codexAppWhaleBalanceScale > 1e12) return "scale";
  return null;
}
