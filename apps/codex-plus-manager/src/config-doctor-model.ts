/** 「AI 配置体检」的纯逻辑：结果排序分组、计数、预设选择、AI 三态文案与时间格式。
 *
 * 后端命令 `config_doctor_request(action, payload)` 的字段与 Rust 侧一一对应（都是 camelCase）。
 * 这里不直接翻译（便于 node 测试）：文案以「中文原文 + 参数」返回，界面用 `t` / `tf` 显示。 */

/** 一条待翻译的文案：中文原文（带 `{0}` 占位时用 `tf`）和参数。 */
export type DoctorText = { text: string; args: string[] };

const text = (value: string, args: string[] = []): DoctorText => ({ text: value, args });

export type FindingLevel = "error" | "warn" | "info";

/** 一条体检结论（本地规则或 AI 解释）。 */
export type Finding = { id: string; level: FindingLevel; title: string; detail: string; fix?: string; scope?: string };

export type Summary = { error: number; warn: number; info: number };

/** AI 解释的三种结果：跳过（未启用）、成功、失败。 */
export type AiReport = {
  status: "skipped" | "ok" | "failed";
  message: string;
  model: string;
  endpoint: string;
  elapsedMs: number;
  summary: string;
  items: Finding[];
};

/** 体检走哪一路模型：复用当前供应商（默认）/ 本机模型 / 自定义。 */
export type DoctorSource = "current" | "local" | "custom";

/** 已保存的 AI 接入配置；Key 原文不出后端，只给末几位掩码。 */
export type DoctorSettings = { enabled: boolean; source: DoctorSource; baseUrl: string; model: string; hasKey: boolean; apiKeyMasked: string };

/** Codex 当前在用的供应商与模型（后端读 config.toml 得到）；`label` 已经是拼好的整句。 */
export type CurrentSource = { available: boolean; model: string; profile: string; label: string };

/** 后端自动探测到的本机模型服务（Ollama / LM Studio）；`label` 也是整句。 */
export type LocalSource = { found: boolean; baseUrl: string; model: string; label: string };

/** 两种「不用填字段」的来源状态，界面据此显示说明与补白提示。 */
export type DoctorSources = { current: CurrentSource; local: LocalSource };

/** 免费 / 常见的 OpenAI 兼容服务预设，下拉一键填 baseUrl 与 model。 */
export type DoctorPreset = { id: string; label: string; baseUrl: string; model: string; needsKey: boolean; note: string; docsUrl: string };

/** `status` / `save` 的返回：设置、预设与来源探测结果。`status` 非 ok 时按失败处理。 */
export type DoctorState = { status?: string; message: string; settings: DoctorSettings; presets: DoctorPreset[]; sources: DoctorSources };

/** `run` 的返回：本地结论 + 汇总 + AI 报告。 */
export type DoctorReport = { findings: Finding[]; summary: Summary; ai: AiReport; checkedAt: string };

/** 表单里的接入配置草稿；`apiKey` 为空表示保存时不改动已存的 Key。 */
export type SettingsDraft = { enabled: boolean; source: DoctorSource; baseUrl: string; model: string; apiKey: string };

/** 分组顺序：先错误，再警告，最后提示。 */
export const FINDING_LEVELS: FindingLevel[] = ["error", "warn", "info"];

/** 三选一顺序：默认的「复用当前供应商」放第一位。 */
export const DOCTOR_SOURCES: DoctorSource[] = ["current", "local", "custom"];

/** 本机没探测到模型服务时的说明（Panel 直接显示这句，并给一个 Ollama 文档按钮）。 */
export const LOCAL_SOURCE_MISSING = "没检测到本机模型服务（Ollama 11434 / LM Studio 1234）";

/** 还没读到 Codex 默认模型时的补白提示。 */
export const CURRENT_SOURCE_MISSING = "Codex 还没设默认模型，先在供应商里选一个或启动一次 Codex。";

/** Ollama 文档的兜底地址（预设里带了 docsUrl 时优先用预设的）。 */
export const OLLAMA_DOCS_URL = "https://docs.ollama.com/api/openai-compatible";

const rank = (level: FindingLevel): number => FINDING_LEVELS.indexOf(level);

export function findingLevelLabel(level: FindingLevel): string {
  switch (level) {
    case "error":
      return "错误";
    case "warn":
      return "警告";
    default:
      return "提示";
  }
}

/** 排序：错误 → 警告 → 提示；同一级别保持后端给的顺序。 */
export function sortFindings(findings: Finding[]): Finding[] {
  return findings
    .map((finding, index) => ({ finding, index }))
    .sort((a, b) => rank(a.finding.level) - rank(b.finding.level) || a.index - b.index)
    .map((entry) => entry.finding);
}

/** 按级别分组（空组不出现），组内按原始顺序。 */
export function groupFindings(findings: Finding[]): Array<{ level: FindingLevel; label: string; items: Finding[] }> {
  return FINDING_LEVELS.map((level) => ({
    level,
    label: findingLevelLabel(level),
    items: sortFindings(findings.filter((finding) => finding.level === level)),
  })).filter((group) => group.items.length > 0);
}

/** 各档计数；后端也会给 summary，这里用于本地复核和展示兜底。 */
export function summarize(findings: Finding[]): Summary {
  const summary: Summary = { error: 0, warn: 0, info: 0 };
  for (const finding of findings) {
    if (finding.level === "error") summary.error += 1;
    else if (finding.level === "warn") summary.warn += 1;
    else if (finding.level === "info") summary.info += 1;
  }
  return summary;
}

export function summaryTotal(summary: Summary): number {
  return summary.error + summary.warn + summary.info;
}

/** 顶部一句话汇总；一项都没有时为空（界面显示「全部正常」）。 */
export function summaryLine(summary: Summary): DoctorText | null {
  const total = summaryTotal(summary);
  if (!total) return null;
  return text("共 {0} 项：{1} 个错误、{2} 个警告、{3} 条提示", [String(total), String(summary.error), String(summary.warn), String(summary.info)]);
}

export function presetFields(preset: DoctorPreset | null | undefined): { baseUrl: string; model: string } | null {
  if (!preset) return null;
  return { baseUrl: preset.baseUrl, model: preset.model };
}

const sameUrl = (a: string, b: string): boolean => a.trim().replace(/\/+$/, "").toLocaleLowerCase() === b.trim().replace(/\/+$/, "").toLocaleLowerCase();

/** 当前 baseUrl 对得上哪个预设：用于下拉回显；对不上（自定义地址）返回空串。 */
export function matchedPresetId(presets: DoctorPreset[], settings: Pick<DoctorSettings, "baseUrl">): string {
  return presets.find((preset) => sameUrl(preset.baseUrl, settings.baseUrl))?.id ?? "";
}

/** 后端历史上的配置可能没有 `source`，空串/未知值一律按「复用当前供应商」处理。 */
export function normalizeSource(value: string | null | undefined): DoctorSource {
  switch ((value ?? "").trim()) {
    case "local":
      return "local";
    case "custom":
      return "custom";
    default:
      return "current";
  }
}

/** 来源三选一的中文原文（界面用 `t` 显示）。 */
export function sourceLabel(source: DoctorSource): string {
  switch (source) {
    case "local":
      return "本机模型";
    case "custom":
      return "自定义";
    default:
      return "复用当前供应商（推荐）";
  }
}

/** 来源选项的说明行：复用后端的探测结果，没探测到时给安装提示。 */
export function sourceHint(source: DoctorSource, sources?: DoctorSources | null): string {
  switch (source) {
    case "local":
      return sources?.local?.found ? sources.local.label : LOCAL_SOURCE_MISSING;
    case "custom":
      return "自己填地址、模型和 Key（不占 Codex++ 的额度）。";
    default:
      return sources?.current?.label || "用你现在 Codex 用的供应商和模型，不用再填任何字段。";
  }
}

/** 三选一的选项列表（值 + 中文原文标签 + 一行说明）。 */
export function sourceOptions(sources?: DoctorSources | null): Array<{ value: DoctorSource; label: string; hint: string }> {
  return DOCTOR_SOURCES.map((value) => ({ value, label: sourceLabel(value), hint: sourceHint(value, sources) }));
}

/** `current` 来源的说明行：后端拼好的整句；没读到默认模型时补一句怎么办。 */
export function currentSourceNote(current?: CurrentSource | null): { label: string; available: boolean; hint: string | null } {
  const available = Boolean(current?.available);
  return {
    label: current?.label || "复用当前供应商（还没读到默认模型）",
    available,
    hint: available ? null : CURRENT_SOURCE_MISSING,
  };
}

/** `local` 来源的说明行：探测到就用后端整句，没探测到说明探测范围。 */
export function localSourceNote(local?: LocalSource | null): { label: string; found: boolean } {
  const found = Boolean(local?.found);
  return { label: found ? (local?.label || LOCAL_SOURCE_MISSING) : LOCAL_SOURCE_MISSING, found };
}

/** 「Ollama 文档」按钮的地址：优先用预设里带的 docsUrl，没有就退回官网地址。 */
export function ollamaDocsUrl(presets: DoctorPreset[]): string {
  const url = presets.find((preset) => preset.id === "ollama")?.docsUrl?.trim();
  return url || OLLAMA_DOCS_URL;
}

/** 从后端设置生成表单草稿；Key 只回显掩码，草稿里留空。 */
export function draftFromSettings(settings: DoctorSettings): SettingsDraft {
  return {
    enabled: settings.enabled,
    source: normalizeSource(settings.source),
    baseUrl: settings.baseUrl,
    model: settings.model,
    apiKey: "",
  };
}

/** 保存时提交的字段：带上来源；Key 留空就不提交，避免用空串清掉已存的 Key。 */
export function savedPayload(draft: SettingsDraft): { enabled: boolean; source: DoctorSource; baseUrl: string; model: string; apiKey?: string } {
  const payload: { enabled: boolean; source: DoctorSource; baseUrl: string; model: string; apiKey?: string } = {
    enabled: draft.enabled,
    source: normalizeSource(draft.source),
    baseUrl: draft.baseUrl.trim(),
    model: draft.model.trim(),
  };
  const apiKey = draft.apiKey.trim();
  if (apiKey) payload.apiKey = apiKey;
  return payload;
}

/** AI 深度体检能不能点：已启用；只有自定义来源要求 Base URL 与模型都填好（复用当前供应商 / 本机模型零配置）。 */
export function aiReady(settings: Pick<DoctorSettings, "enabled" | "source" | "baseUrl" | "model">): boolean {
  if (!settings.enabled) return false;
  if (normalizeSource(settings.source) !== "custom") return true;
  return Boolean(settings.baseUrl.trim() && settings.model.trim());
}

/** 不能点 AI 深度体检时的悬停说明；能点为 null。 */
export function aiBlockedHint(settings: Pick<DoctorSettings, "enabled" | "source" | "baseUrl" | "model">): DoctorText | null {
  if (!settings.enabled) return text("先启用 AI 解释并保存，再用 AI 深度体检。");
  if (normalizeSource(settings.source) === "custom" && !(settings.baseUrl.trim() && settings.model.trim())) {
    return text("先填好 Base URL 和模型并保存，再用 AI 深度体检。");
  }
  return null;
}

/** AI 报告在界面上的三态：图标配色、标签、说明与事实行（模型、地址、耗时）。 */
export function aiStatusView(ai: AiReport): { tone: "ok" | "off" | "danger"; label: string; detail: string; facts: string[] } {
  switch (ai.status) {
    case "ok":
      return {
        tone: "ok",
        label: "AI 解释完成",
        detail: ai.summary || ai.message,
        facts: [ai.model, ai.endpoint, formatElapsed(ai.elapsedMs)].filter(Boolean),
      };
    case "failed":
      return {
        tone: "danger",
        label: "AI 解释失败",
        detail: ai.message,
        facts: [ai.model, ai.endpoint].filter(Boolean),
      };
    default:
      return {
        tone: "off",
        label: "未用 AI 解释",
        detail: ai.message || "这次只跑了本地规则体检。",
        facts: [],
      };
  }
}

/** 毫秒数显示：1 秒以内按毫秒，以上按一位小数的秒；没有数值给「—」。 */
export function formatElapsed(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return "—";
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

/** 体检时间：本地时区的「YYYY-MM-DD HH:mm」；解析不了就空着。 */
export function formatCheckedAt(value: string | null | undefined): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
