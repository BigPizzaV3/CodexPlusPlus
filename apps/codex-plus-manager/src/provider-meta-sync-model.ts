/** 「从上游同步上下文窗口 / 思考等级」（#2807）的勾选口径：和「从上游同步模型列表」一样先看差异、
 * 勾选后才写入。现值是用户手动设的默认不勾（可勾选覆盖），其余默认勾上。 */
import { formatTokens } from "./gateway-model.ts";

/** 模型现在的值从哪来（后端 `CurrentSource`）。 */
export type MetaCurrentSource = "custom" | "synced" | "providerDefault" | "builtin" | "none";

/** 后端 `WindowChange`：一个模型的窗口差异。 */
export type WindowChange = {
  model: string;
  window: number;
  /** 上游字段，如 `context_length`。 */
  source: string;
  /** 上游 `context_windows` 的可选档位。 */
  options: number[];
  current: number | null;
  currentSource: MetaCurrentSource;
  manual: boolean;
};

/** 后端 `ReasoningChange`：一个模型的思考等级差异。 */
export type ReasoningChange = {
  model: string;
  levels: string[];
  defaultLevel: string | null;
  source: string;
  defaultSource: string;
  currentLevels: string[];
  currentDefault: string | null;
  currentSource: MetaCurrentSource;
  manual: boolean;
};

export type IgnoredReason = "invalidWindow" | "unknownLevel" | "defaultNotInLevels" | "invalidType";

/** 后端 `IgnoredNote`：被跳过的上游字段值。 */
export type IgnoredNote = { model: string; field: string; value: string; reason: IgnoredReason };

/** 后端 `MetaSyncPreview`，随状态一起摊平在结果里。 */
export type MetaSyncPreview = {
  upstreamModels: number;
  matched: number;
  withWindow: number;
  withReasoning: number;
  windows: WindowChange[];
  reasoning: ReasoningChange[];
  ignored: IgnoredNote[];
};

export type MetaSyncKind = "windows" | "reasoning";

/** 弹窗里一行差异（窗口和思考等级共用）。 */
export type MetaChangeRow = {
  model: string;
  next: string;
  current: string;
  currentSource: MetaCurrentSource;
  source: string;
  manual: boolean;
  /** 窗口的可选档位（展示用）。 */
  options: string;
};

/** 结果里缺的字段补默认值（旧版后端或失败时）。 */
export function normalizePreview(raw: Partial<MetaSyncPreview> | null | undefined): MetaSyncPreview {
  return {
    upstreamModels: raw?.upstreamModels ?? 0,
    matched: raw?.matched ?? 0,
    withWindow: raw?.withWindow ?? 0,
    withReasoning: raw?.withReasoning ?? 0,
    windows: raw?.windows ?? [],
    reasoning: raw?.reasoning ?? [],
    ignored: raw?.ignored ?? [],
  };
}

/** 思考等级的展示：`low, high · 默认 high`。 */
export function formatLevels(levels: string[], defaultLevel: string | null, defaultLabel = "默认"): string {
  if (!levels.length) return "";
  return defaultLevel ? `${levels.join(", ")} · ${defaultLabel} ${defaultLevel}` : levels.join(", ");
}

/** 这一类的差异行。 */
export function changeRows(preview: MetaSyncPreview, kind: MetaSyncKind, defaultLabel = "默认"): MetaChangeRow[] {
  if (kind === "windows") {
    return preview.windows.map((change) => ({
      model: change.model,
      next: formatTokens(change.window),
      current: change.current ? formatTokens(change.current) : "",
      currentSource: change.currentSource,
      source: change.source,
      manual: change.manual,
      options: change.options.length > 1 ? change.options.map(formatTokens).join(" / ") : "",
    }));
  }
  return preview.reasoning.map((change) => ({
    model: change.model,
    next: formatLevels(change.levels, change.defaultLevel, defaultLabel),
    current: formatLevels(change.currentLevels, change.currentDefault, defaultLabel),
    currentSource: change.currentSource,
    source: change.defaultSource ? `${change.source} · ${change.defaultSource}` : change.source,
    manual: change.manual,
    options: "",
  }));
}

/** 初始勾选：手动设过的不勾，其余勾上。 */
export function defaultPicked(preview: MetaSyncPreview, kind: MetaSyncKind): Set<string> {
  const changes: Array<{ model: string; manual: boolean }> = kind === "windows" ? preview.windows : preview.reasoning;
  return new Set(changes.filter((change) => !change.manual).map((change) => change.model));
}

/** 勾选结果 → 写入窗口的参数。 */
export function windowPicks(preview: MetaSyncPreview, picked: ReadonlySet<string>): Array<{ model: string; window: number }> {
  return preview.windows.filter((change) => picked.has(change.model)).map((change) => ({ model: change.model, window: change.window }));
}

/** 勾选结果 → 写入思考等级的参数。 */
export function reasoningPicks(
  preview: MetaSyncPreview,
  picked: ReadonlySet<string>,
): Array<{ model: string; levels: string[]; defaultLevel: string | null }> {
  return preview.reasoning
    .filter((change) => picked.has(change.model))
    .map((change) => ({ model: change.model, levels: change.levels, defaultLevel: change.defaultLevel }));
}

/** 这一类相关的被跳过字段：窗口看窗口类原因，思考等级看档位类原因；类型不对的按字段名分。 */
export function ignoredFor(preview: MetaSyncPreview, kind: MetaSyncKind): IgnoredNote[] {
  return preview.ignored.filter((note) => {
    const reasoning = note.reason === "unknownLevel" || note.reason === "defaultNotInLevels"
      || (note.reason === "invalidType" && /reason|effort/i.test(note.field));
    return kind === "reasoning" ? reasoning : !reasoning;
  });
}

/** 现值来源的文案（中文原文，界面再过 t()）。 */
export function currentSourceLabel(source: MetaCurrentSource): string {
  switch (source) {
    case "custom":
      return "手动设置";
    case "synced":
      return "上次同步";
    case "providerDefault":
      return "供应商默认";
    case "builtin":
      return "内置资料";
    default:
      return "未设置";
  }
}

/** 跳过原因的文案（中文原文，界面再过 t()）。 */
export function ignoredReasonLabel(reason: IgnoredReason): string {
  switch (reason) {
    case "invalidWindow":
      return "窗口数值不合理";
    case "unknownLevel":
      return "不是 Codex 支持的档位";
    case "defaultNotInLevels":
      return "默认档不在支持的档位里";
    default:
      return "字段类型不对";
  }
}
