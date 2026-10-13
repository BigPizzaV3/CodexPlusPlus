import {
  modelMetadataKey,
  parseModelMetadataMap,
  serializeModelMetadataMap,
  type ModelMetadata,
} from "./model-metadata.ts";

// 「思考等级」列的纯逻辑：解析、过滤、标签折叠与写回 metadata map。
// effort 取值与描述文本对齐内置元数据资产（assets/*-model-metadata*.json），
// 写进 catalog 的 supported_reasoning_levels 后由 Codex 直接展示。
export const REASONING_LEVELS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;

export type ReasoningLevel = (typeof REASONING_LEVELS)[number];

export const REASONING_LEVEL_DESCRIPTIONS: Record<ReasoningLevel, string> = {
  none: "Fast responses without reasoning",
  minimal: "Minimal reasoning, fastest responses",
  low: "Fast responses with lighter reasoning",
  medium: "Balances speed and reasoning depth for everyday tasks",
  high: "Greater reasoning depth for complex problems",
  xhigh: "Extra high reasoning depth for complex problems",
  max: "Maximum reasoning depth for the hardest problems",
  ultra: "Maximum reasoning with automatic task delegation",
};

function isReasoningLevel(value: unknown): value is ReasoningLevel {
  return typeof value === "string" && Object.hasOwn(REASONING_LEVEL_DESCRIPTIONS, value);
}

/** 归一化任意声明（字符串或 {effort} 对象）：只保留已知档位，按官方升序去重。 */
export function normalizeReasoningLevels(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const picked = new Set<string>();
  for (const item of value) {
    const effort = typeof item === "string"
      ? item
      : item && typeof item === "object"
        ? (item as { effort?: unknown }).effort
        : null;
    if (isReasoningLevel(effort)) picked.add(effort);
  }
  return REASONING_LEVELS.filter((level) => picked.has(level));
}

/** metadata 里的自定义声明；字段缺失返回 null（区别于显式空数组）。 */
export function reasoningLevelsFromMetadata(metadata: ModelMetadata | null | undefined): string[] | null {
  if (!metadata || !Object.hasOwn(metadata, "supported_reasoning_levels")) return null;
  return normalizeReasoningLevels(metadata.supported_reasoning_levels);
}

export function defaultReasoningLevelFromMetadata(metadata: ModelMetadata | null | undefined): string | null {
  const value = metadata?.default_reasoning_level;
  return isReasoningLevel(value) ? value : null;
}

/**
 * 触发器标签：连续三档及以上折叠成 "low → max"（minimal 未选时区间不会跨过它，
 * 所以 "none, low → max" 不会暗示 minimal 被选中）；空选择返回空串。
 */
export function formatReasoningLevelsLabel(levels: string[]): string {
  const positions = levels.map((level) => REASONING_LEVELS.indexOf(level as ReasoningLevel));
  const ordered = positions.every(
    (position, index) => position >= 0 && (index === 0 || position > positions[index - 1]),
  );
  if (!ordered) return levels.join(", ");
  const parts: string[] = [];
  let runStart = 0;
  for (let index = 1; index <= levels.length; index++) {
    if (index < levels.length && positions[index] === positions[index - 1] + 1) continue;
    const run = levels.slice(runStart, index);
    if (run.length >= 3) parts.push(`${run[0]} → ${run[run.length - 1]}`);
    else parts.push(...run);
    runStart = index;
  }
  return parts.join(", ");
}

export type ModelReasoningLevelsState = {
  /// 当前生效的档位（自定义优先，其次内置声明）。
  levels: string[];
  /// true 表示来自该模型的自定义配置，false 表示内置/未声明。
  overridden: boolean;
  /// 自定义默认档；内置默认不回填，避免开启面板就伪造出覆盖。
  defaultLevel: string | null;
};

export function resolveModelReasoningLevels(
  override: ModelMetadata | null | undefined,
  builtinLevels: unknown,
): ModelReasoningLevelsState {
  const declared = reasoningLevelsFromMetadata(override);
  if (declared !== null) {
    return {
      levels: declared,
      overridden: true,
      defaultLevel: defaultReasoningLevelFromMetadata(override),
    };
  }
  return {
    levels: normalizeReasoningLevels(builtinLevels),
    overridden: false,
    defaultLevel: null,
  };
}

/**
 * 写回该模型的思考等级声明：整组替换 supported_reasoning_levels，并按需维护
 * default_reasoning_level；清空选择等价于移除这两个键（回退内置默认），不残留
 * 空条目。其他字段与其它模型条目原样保留。
 */
export function setModelReasoningLevelsForSlug(
  value: string,
  slug: string,
  levels: unknown,
  defaultLevel?: string | null,
): string {
  const map = parseModelMetadataMap(value);
  const key = modelMetadataKey(slug);
  if (!key) return serializeModelMetadataMap(map);
  const normalized = normalizeReasoningLevels(levels);
  const entry: ModelMetadata = { ...(map[key] ?? {}) };
  if (normalized.length) {
    entry.supported_reasoning_levels = normalized.map((effort) => ({
      effort,
      description: REASONING_LEVEL_DESCRIPTIONS[effort as ReasoningLevel],
    }));
  } else {
    delete entry.supported_reasoning_levels;
  }
  if (defaultLevel && normalized.includes(defaultLevel)) entry.default_reasoning_level = defaultLevel;
  else delete entry.default_reasoning_level;
  if (Object.keys(entry).length) map[key] = entry;
  else delete map[key];
  return serializeModelMetadataMap(map);
}
