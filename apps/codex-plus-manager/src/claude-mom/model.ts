/**
 * ClaudeMom 界面的纯逻辑：后端数据类型、状态文案、表单校验。不依赖 React / Tauri，便于单测。
 */

export type CommandStatus = "ok" | "failed" | string;
export type CommandResult<T> = T & { status: CommandStatus; message: string };

export type UpstreamSource = "manual" | "ccswitch" | "magpie" | "codexpp";

/** 一组上游（与 core `claude_mom::UpstreamProfile` 字段一致）。 */
export type UpstreamProfile = {
  id: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  model: string;
  smallModel: string;
  models: string[];
  contextWindow: number | null;
  /** 模型显示名（上游 `/v1/models` 的 display_name）。 */
  modelNames: Record<string, string>;
  source: UpstreamSource | string;
  sourceId: string;
};

export type UpstreamBook = { profiles: UpstreamProfile[]; activeId: string | null };

export type UpstreamModel = { id: string; displayName: string | null; contextWindow: number | null };

/** 界面上显示的模型名：有显示名用显示名，否则用 id。 */
export function modelLabel(profile: Pick<UpstreamProfile, "modelNames">, id: string): string {
  return profile.modelNames?.[id] || id;
}

/** Claude 本地服务的设置（与 core `ClaudeLocalSettings` 一致，只列界面用到的）。 */
export type ClaudeConfig = {
  enabled: boolean;
  model: string;
  localization: boolean;
  voiceInput: boolean;
  computerUse: boolean;
  backgroundMode: boolean;
  displayName?: string;
  email?: string;
  plan?: string;
  sessionSync?: boolean;
  listAllModels?: boolean;
  hideUnroutedModels?: boolean;
};

export type ClaudeStatus = {
  supported: boolean;
  sourceApp: string;
  version: string;
  claudeRunning: boolean;
  running: boolean;
  localProcess?: boolean;
  connected: boolean;
  port?: number | null;
  caReady: boolean;
  caTrusted: boolean;
  nodeAvailable: boolean;
  dictationReady: boolean;
  config: ClaudeConfig;
  models: string[];
  contextWindow?: number | null;
  warning?: string | null;
  needsRestart: boolean;
};

export const SOURCE_LABELS: Record<string, string> = {
  manual: "手动",
  ccswitch: "cc-switch",
  magpie: "Magpie",
  codexpp: "Codex++",
};

export const IMPORT_SOURCES: Array<{ id: UpstreamSource; label: string; detail: string }> = [
  { id: "ccswitch", label: "cc-switch", detail: "导入 cc-switch 里的 Claude 供应商" },
  { id: "magpie", label: "Magpie", detail: "把 Magpie 的本机网关当上游" },
  { id: "codexpp", label: "Codex++", detail: "把 Codex++ 的本机网关当上游" },
];

export function emptyProfile(): UpstreamProfile {
  return { id: "", name: "", baseUrl: "", apiKey: "", model: "", smallModel: "", models: [], contextWindow: null, modelNames: {}, source: "manual", sourceId: "" };
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url.trim();
  }
}

/** 表单校验：返回第一条问题，没有问题返回 null。 */
export function profileProblem(profile: UpstreamProfile): string | null {
  const base = profile.baseUrl.trim();
  if (!base) return "填一个 Base URL";
  let parsed: URL;
  try {
    parsed = new URL(base);
  } catch {
    return "Base URL 不是有效地址";
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return "Base URL 要以 http:// 或 https:// 开头";
  if (/\/v1\/messages\/?$/.test(parsed.pathname)) return "Base URL 只写到根地址，不要带 /v1/messages";
  if (!profile.model.trim()) return "选一个默认模型";
  return null;
}

/** 列表去重、去空白，默认模型排第一。 */
export function normalizeModels(models: string[], primary = ""): string[] {
  const out: string[] = [];
  for (const raw of [primary, ...models]) {
    const model = raw.trim();
    if (model && !out.includes(model)) out.push(model);
  }
  return out;
}

export type Tone = "ok" | "warn" | "off" | "busy";

/** 顶部状态条：一句话 + 色调。 */
export function claudeState(status: ClaudeStatus | null, busy: string | null): { text: string; tone: Tone } {
  if (busy === "launch") return { text: "正在启动…", tone: "busy" };
  if (busy === "stop") return { text: "正在停止…", tone: "busy" };
  if (!status) return { text: "正在检测 Claude…", tone: "busy" };
  if (!status.supported) return { text: "没有找到 Claude Desktop", tone: "warn" };
  if (status.connected) return { text: "已连接", tone: "ok" };
  if (status.claudeRunning && status.localProcess) return { text: "运行中", tone: "ok" };
  if (status.claudeRunning) return { text: "Claude 已打开，但没有经 ClaudeMom 启动", tone: "warn" };
  if (status.running) return { text: "本地服务已就绪", tone: "ok" };
  return { text: "未启动", tone: "off" };
}

/** 启动前还差什么：按顺序列出，界面逐条给按钮。 */
export function setupSteps(status: ClaudeStatus | null, active: UpstreamProfile | null): Array<{ id: "upstream" | "node" | "trust"; text: string }> {
  if (!status || !status.supported) return [];
  const steps: Array<{ id: "upstream" | "node" | "trust"; text: string }> = [];
  if (!active || profileProblem(active)) steps.push({ id: "upstream", text: "先添加一组上游并选好默认模型" });
  if (!status.nodeAvailable) steps.push({ id: "node", text: "需要 Node.js（支持 fetch 与 WebSocket 的版本）" });
  if (!status.caTrusted) steps.push({ id: "trust", text: "首次使用需要信任本地证书" });
  return steps;
}

export const WINDOW_PRESETS: Array<{ value: number | null; label: string }> = [
  { value: null, label: "不声明（200k）" },
  { value: 200_000, label: "200k" },
  { value: 400_000, label: "400k" },
  { value: 1_000_000, label: "1M" },
];

export function formatWindow(value: number | null | undefined): string {
  if (!value) return "200k";
  if (value >= 1_000_000) return `${+(value / 1_000_000).toFixed(1)}M`;
  return `${Math.round(value / 1000)}k`;
}

export function isOk(result: { status: CommandStatus }): boolean {
  return result.status === "ok";
}
