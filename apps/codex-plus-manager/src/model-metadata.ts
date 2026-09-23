import { DEFAULT_AUTO_COMPACT_PERCENT, normalizeAutoCompactPercent } from "./auto-compact.ts";

export type ModelMetadata = Record<string, unknown>;
export type ModelMetadataMap = Record<string, ModelMetadata>;

export type ImportedModelMetadata = {
  slug: string;
  metadata: ModelMetadata;
  contextWindow: string | null;
  autoCompactPercent: string | null;
  autoCompactCalculationPercent?: string | null;
  ignoredFields: string[];
};

export type ModelMetadataImportResult =
  | { ok: true; value: ImportedModelMetadata }
  | { ok: false; error: string };

// slug 和三个由界面专门编辑的数值字段（context_window / max_context_window /
// auto_compact_token_limit）不进入 metadata map：窗口字段由「上下文窗口」列统一
// 管辖（catalog 生成时写为同值），压缩阈值换算成百分比。否则残留值会在生成后
// 反向覆盖界面编辑的窗口（issue #2191）。
// 其余字段属于供应商模型事实，导入时保留并在 catalog 中优先于生成默认值。
const MANAGED_MODEL_METADATA_FIELDS = new Set<string>();

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isImportedMetadataField(key: string): boolean {
  return key !== "slug"
    && key !== "context_window"
    && key !== "max_context_window"
    && key !== "auto_compact_token_limit"
    && !MANAGED_MODEL_METADATA_FIELDS.has(key);
}

function filteredMetadata(metadata: ModelMetadata): ModelMetadata {
  return Object.fromEntries(
    Object.entries(metadata).filter(([key]) => isImportedMetadataField(key)),
  );
}

export function parseModelMetadataMap(value: string): ModelMetadataMap {
  if (!value.trim()) return {};
  try {
    const parsed: unknown = JSON.parse(value);
    if (!isRecord(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed)
        .filter((entry): entry is [string, ModelMetadata] => isRecord(entry[1]))
        .map(([slug, metadata]) => [slug, filteredMetadata(metadata)] as [string, ModelMetadata])
        .filter(([, metadata]) => Object.keys(metadata).length > 0),
    );
  } catch {
    return {};
  }
}

export function serializeModelMetadataMap(map: ModelMetadataMap): string {
  return Object.keys(map).length > 0 ? JSON.stringify(map) : "";
}

const MAX_U64 = 18_446_744_073_709_551_615n;
const MAX_SAFE_INTEGER = BigInt(Number.MAX_SAFE_INTEGER);
const PERCENT_SCALE = 1_000_000n;
const SCALED_PERCENT_DENOMINATOR = 100n * PERCENT_SCALE;

function contextWindowToBigInt(value: string): bigint | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const match = trimmed.match(/^(\d+)([KkMm])?$/);
  if (!match) return null;
  const multiplier = match[2]?.toLowerCase() === "m"
    ? 1_000_000n
    : match[2]
      ? 1_000n
      : 1n;
  const tokens = BigInt(match[1]) * multiplier;
  return tokens > 0n && tokens <= MAX_U64 ? tokens : null;
}

function contextWindowToTokens(value: string): number | null {
  const tokens = contextWindowToBigInt(value);
  return tokens !== null && tokens <= MAX_SAFE_INTEGER ? Number(tokens) : null;
}

function autoCompactPercentToScaled(value: string): bigint | null {
  const normalized = value.trim().replace(/%$/, "").trim();
  if (!normalized) return null;
  const match = normalized.match(/^(\d+)(?:\.(\d{1,6}))?$/);
  if (!match) return null;
  const fraction = (match[2] ?? "").padEnd(6, "0");
  const scaled = BigInt(match[1]) * PERCENT_SCALE + BigInt(fraction || "0");
  return scaled > 0n && scaled <= SCALED_PERCENT_DENOMINATOR ? scaled : null;
}

function autoCompactPercentToTokenLimit(
  contextWindow: string,
  autoCompactPercent: string,
): number | null {
  const contextWindowTokens = contextWindowToBigInt(contextWindow);
  const scaledPercent = autoCompactPercentToScaled(autoCompactPercent);
  if (contextWindowTokens === null || scaledPercent === null) return null;
  const rounded = (contextWindowTokens * scaledPercent + SCALED_PERCENT_DENOMINATOR / 2n)
    / SCALED_PERCENT_DENOMINATOR;
  const compactTokens = rounded > 0n ? rounded : 1n;
  return compactTokens <= MAX_SAFE_INTEGER ? Number(compactTokens) : null;
}

function autoCompactTokenLimitToPercent(contextWindow: string, tokenLimit: string): string | null {
  const contextWindowTokens = contextWindowToBigInt(contextWindow);
  const compactTokens = /^\d+$/.test(tokenLimit) ? BigInt(tokenLimit) : 0n;
  if (contextWindowTokens === null || compactTokens <= 0n || compactTokens > contextWindowTokens) return null;
  const scaled = (compactTokens * SCALED_PERCENT_DENOMINATOR + contextWindowTokens / 2n)
    / contextWindowTokens;
  if (scaled <= 0n || scaled > SCALED_PERCENT_DENOMINATOR) return null;
  const whole = scaled / PERCENT_SCALE;
  const fraction = (scaled % PERCENT_SCALE).toString().padStart(6, "0").replace(/0+$/, "");
  return `${whole}${fraction ? `.${fraction}` : ""}%`;
}

function displayAutoCompactPercent(value: string | null): string | null {
  if (!value) return value;
  const scaled = autoCompactPercentToScaled(value);
  if (scaled === null) return value;
  const rounded = (scaled + PERCENT_SCALE / 2n) / PERCENT_SCALE;
  return `${rounded}%`;
}

export function serializeModelMetadataDocument(
  slug: string,
  metadata: ModelMetadata,
  contextWindow: string,
  autoCompactPercent = "",
): string {
  const contextWindowTokens = contextWindowToTokens(contextWindow);
  const autoCompactTokenLimit = autoCompactPercentToTokenLimit(contextWindow, autoCompactPercent);
  return JSON.stringify({
    models: [{
      slug,
      ...(contextWindowTokens ? { context_window: contextWindowTokens } : {}),
      ...(autoCompactTokenLimit ? { auto_compact_token_limit: autoCompactTokenLimit } : {}),
      ...filteredMetadata(metadata),
    }],
  }, null, 2);
}

export type BuiltinModelMetadataEntry = {
  slug: string;
  display_name?: string;
  context_window?: number | null;
  max_context_window?: number | null;
  auto_compact_token_limit?: number | null;
  [key: string]: unknown;
};

// ── [1M] 后缀的检测与适配 ────────────────────────────────────────────────
// 模型行里用户原样输入的 `deepseek-v4-pro[1M]` 带窗口后缀，后缀的含义就是
// 「该模型的上下文窗口大小」（1M=1000000、256K=256000），由 Rust 侧
// parse_model_suffix 在生成 catalog 时剥离并换算。导入面板、标签、实时同步
// 都在 Slug 层面工作，必须先把后缀剥掉再做任何 slug 比较，否则后端返回的
// 规范 slug 与行名永远对不上（issue #2279 回归）。
// 这里集中放一处，四个入口（内置预填、parseModelMetadataDocument、
// synchronize*、metadataMatchesBuiltin）共用，避免各自实现再次分叉。
const MODEL_SUFFIX_PATTERN = /^(.*?)\[(\d+(?:[KkMm])?)\]$/;

/// 从模型行名拆出规范 slug；无后缀或后缀非法时返回去掉首尾空白的原串。
export function modelSlugFromRowName(rowName: string): string {
  const trimmed = rowName.trim();
  const match = MODEL_SUFFIX_PATTERN.exec(trimmed);
  if (!match) return trimmed;
  if (suffixWindowTokens(match[2]) === null) return trimmed;
  return match[1].trim();
}

/// 把后缀文字换算成 token 数：`[1M]`→1000000、`[256K]`→256000、`[123]`→123。
/// 仅识别纯数字 + 可选 K/M 单位（大小写均可），其余一律 null。
export function suffixWindowTokens(suffix: string): number | null {
  const match = /^(\d+)([KkMm])?$/.exec(suffix.trim());
  if (!match) return null;
  const multiplier = match[2]
    ? (match[2].toLowerCase() === "m" ? 1_000_000 : 1_000)
    : 1;
  const tokens = Number(match[1]) * multiplier;
  if (!Number.isSafeInteger(tokens) || tokens <= 0) return null;
  return tokens;
}

/// 后缀对应的窗口字符串（供「上下文窗口」列初值/写回用）；无有效后缀返回 null。
export function suffixWindowString(rowName: string): string | null {
  const match = MODEL_SUFFIX_PATTERN.exec(rowName.trim());
  if (!match) return null;
  const tokens = suffixWindowTokens(match[2]);
  return tokens === null ? null : String(tokens);
}

export type BuiltinModelMetadataMatch = {
  matched: boolean;
  source?: string;
  entry?: BuiltinModelMetadataEntry;
  fallback?: { slug: string; context_window: number };
};

/// 内置条目 → 导入文档文本：剥掉窗口/压缩四个托管字段（serialize 会按
/// 窗口参数重写），保留供应商事实字段。窗口取 context_window 优先——它是
/// codex 的默认运行窗口（官方 gpt 系为 272000/872000，导入 872000 会把
/// 运行窗口改成上限，改变默认行为）；max 仅作 context 缺失时的回退。
export function builtinEntryToImportDocument(entry: BuiltinModelMetadataEntry): string {
  const contextWindow = entry.context_window ?? entry.max_context_window;
  return serializeModelMetadataDocument(
    entry.slug,
    entry as ModelMetadata,
    contextWindow ? String(contextWindow) : "",
  );
}

/// 标签文案以「中文 key + 插值参数」下发，由调用方过 t()/tf()：
/// 裸字符串会被 i18n-verify.mjs 漏掉（它只扫调用点），英文模式直接露中文。
/// key 全部登记在 i18n-en.ts 的 EN_TEMPLATE/EN_PLAIN 里。
export type MetadataSourceTag = {
  kind: "match" | "fallback" | "custom";
  tone: "builtin" | "fallback" | "custom";
  textKey: string;
  textArgs: Array<string | number>;
  titleKey: string;
  titleArgs: Array<string | number>;
};

/// 元数据来源标签（覆盖全部用户场景）：
/// - 自定义存在 → 内置命中与否都显示 [自定义]；同时命中内置时并列 [匹配：来源]
///   （内置仍是底层事实）；未命中内置时只显示 [自定义]（自定义已覆盖，无"回退"可言）
/// - 无自定义 → 命中内置 [匹配：来源]，否则 [回退：<fallbackSlug>]
export function metadataSourceTags(options: {
  slug: string;
  imported: boolean;
  builtinMatch: BuiltinModelMetadataMatch | null;
  builtinIndexSlug: { source: string } | undefined;
  /// 无内置时的回退模板名；由调用方从后端 fallback 字段实时取，不写死。
  fallbackSlug?: string;
}): MetadataSourceTag[] {
  const { imported, builtinMatch, builtinIndexSlug } = options;
  const fallbackSlug = options.fallbackSlug ?? options.builtinMatch?.fallback?.slug ?? "";
  const matchedSource = builtinMatch?.matched && builtinMatch.entry
    ? builtinMatch.source ?? ""
    : builtinIndexSlug?.source;
  const tags: MetadataSourceTag[] = [];
  if (matchedSource) {
    tags.push({
      kind: "match",
      tone: "builtin",
      textKey: "匹配：{0}",
      textArgs: [matchedSource],
      titleKey: "内置元数据：{0}",
      titleArgs: [matchedSource],
    });
  } else if (!imported) {
    tags.push({
      kind: "fallback",
      tone: "fallback",
      textKey: "回退：{0}",
      textArgs: [fallbackSlug],
      titleKey: "无内置元数据，生成时回退 {0} 官方模板",
      titleArgs: [fallbackSlug],
    });
  }
  if (imported) {
    tags.push({
      kind: "custom",
      tone: "custom",
      textKey: "自定义",
      textArgs: [],
      titleKey: matchedSource
        ? "已导入自定义元数据，生成时覆盖内置（{0}）"
        : "已导入自定义元数据，生成时以该配置为准",
      titleArgs: matchedSource ? [matchedSource] : [],
    });
  }
  return tags;
}

export type ModelRowSyncPatch = { window?: string; autoCompact?: string };

/// 导入文档解析结果 → 模型行补丁（JSON→行 的实时写回规则）：
/// - 窗口：解析出有效值且与行现值不同才写（相同不写，避免多余 state 更新）
/// - 压缩比：解析出显示值且与行现值不同才写；null（JSON 未声明）不动行，
///   避免粘贴别的模型 JSON 时清掉用户行里的值
export function importDocumentSyncPatch(
  row: { window: string; autoCompact: string },
  preview: { contextWindow: string | null; autoCompactPercent: string | null },
): ModelRowSyncPatch {
  const patch: ModelRowSyncPatch = {};
  if (preview.contextWindow && preview.contextWindow !== row.window) {
    patch.window = preview.contextWindow;
  }
  if (preview.autoCompactPercent && preview.autoCompactPercent !== row.autoCompact) {
    patch.autoCompact = preview.autoCompactPercent;
  }
  return patch;
}

export type ImportSaveDecision = {
  /// 是否需要写 profile.modelMetadata（false = 当前配置已是目标态）
  needsSave: boolean;
  /// 保存动作的语义：写自定义覆盖 / 清掉自定义改用内置 / 无需操作
  effect: "custom" | "builtin" | "none";
  /// 按钮文案
  label: string;
  /// hover 说明：为什么可点或为什么不可点
  title: string;
};

/// 元数据对象的稳定序列化（键排序）——用于两套元数据的相等比较，
/// 不受字段书写顺序影响，只比内容。
function stableMetadataKey(metadata: ModelMetadata): string {
  return JSON.stringify(metadata, (_key, value) => {
    if (value && typeof value === "object" && !Array.isArray(value)) {
      return Object.fromEntries(Object.entries(value as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : 1)));
    }
    return value;
  });
}

/// 面板里的元数据与内置条目是否等价（键排序后深比较）。
/// 只比供应商事实字段：窗口字段由「上下文窗口」列管辖，不参与判断——
/// 否则仅改窗口就会被误判成「要存成自定义」。这里显式剥掉被托管的字段，
/// 不依赖调用方恰好已经过滤：即便上游某一侧漏过滤，判定也不会被窗口值带偏
/// （否则 c00177e 修掉的「重新匹配后保存变成自定义」会静默回归）。
export function metadataMatchesBuiltin(
  metadata: ModelMetadata | null | undefined,
  builtinMetadata: ModelMetadata | null | undefined,
): boolean {
  if (!metadata || !builtinMetadata) return false;
  return stableMetadataKey(filteredMetadata(metadata))
    === stableMetadataKey(filteredMetadata(builtinMetadata));
}

/// 「保存此模型」按钮的判定。核心原则：**保存匹配到的内置数据不该产生自定义覆盖**。
/// 面板内容与内置条目等价时，目标态就是「用内置」——本来就已经在用，无需写入；
/// 只有当用户真的改了内容，才写成自定义覆盖。
export function importSaveDecision(options: {
  /// 当前文本是否可解析出有效模型（false = 解析失败）
  parseOk: boolean;
  /// 文本是否为空
  documentBlank: boolean;
  /// 当前是否已存有该模型的自定义配置
  imported: boolean;
  /// 面板元数据与内置条目是否等价（无内置匹配时为 false）
  matchesBuiltin: boolean;
}): ImportSaveDecision {
  const label = "保存此模型";

  if (!options.parseOk) {
    return { needsSave: false, effect: "none", label, title: "JSON 无法解析，修复后即可保存" };
  }
  // 空文本：没有内容可写成自定义；若已有自定义则等于「放弃自定义」
  if (options.documentBlank) {
    return options.imported
      ? { needsSave: true, effect: "builtin", label: "恢复内置", title: "保存后清除该模型的自定义配置，改用内置元数据" }
      : { needsSave: false, effect: "none", label, title: "没有可保存的内容" };
  }
  // 内容与内置一致：目标态就是内置，本来就已经在用，不写覆盖
  if (options.matchesBuiltin) {
    return options.imported
      ? { needsSave: true, effect: "builtin", label: "恢复内置", title: "内容与内置元数据一致，保存后改用内置元数据" }
      : { needsSave: false, effect: "none", label, title: "已在使用内置元数据，无需保存" };
  }
  // 内容与内置不同：写成自定义覆盖
  return {
    needsSave: true,
    effect: "custom",
    label: options.imported ? "更新此模型配置" : "保存为自定义配置",
    title: options.imported
      ? "保存当前内容为该模型的自定义配置"
      : "当前为内置元数据预览的修改版；保存后将成为该模型的自定义配置，生成时覆盖内置",
  };
}

/// 导入区四个按钮 + 状态行的唯一判定来源。
/// 存在意义：把原先散在 JSX 里的四组显隐/置灰条件收成一處，使按钮「始终在同一
/// 位置、只是能不能点」——不会再出现点一个键就少一个键的情况。
export type ImportPanelControls = {
  rematch: { disabled: boolean; title: string };
  clear: { disabled: boolean; title: string };
  cancel: { disabled: boolean; title: string };
  save: { disabled: boolean; label: string; title: string };
  status: ImportPanelStatus;
};

export type ImportPanelStatus = {
  tone: "builtin" | "custom" | "fallback";
  text: string;
  title: string;
};

/// fallbackSlug：无内置元数据时生成所用的官方模板名，由调用方从后端
/// fallback 字段实时取（不写死，随 bundled 静态资产首条演进）。
export function importPanelControls(options: {
  slug: string;
  document: string;
  imported: boolean;
  parseOk: boolean;
  matched: boolean;
  /// 面板元数据与内置条目是否等价（由调用方用 metadataMatchesBuiltin 算出）
  matchesBuiltin: boolean;
  matchedSource?: string;
  /// 无内置时的回退模板名；由调用方从后端 fallback 字段实时取，不写死。
  fallbackSlug?: string;
}): ImportPanelControls {
  const fallbackSlug = options.fallbackSlug ?? "";
  const slugBlank = !options.slug.trim();
  const documentBlank = !options.document.trim();
  const decision = importSaveDecision({
    parseOk: options.parseOk,
    documentBlank,
    imported: options.imported,
    matchesBuiltin: options.matchesBuiltin,
  });
  const save = {
    disabled: !decision.needsSave,
    label: decision.label,
    title: decision.title,
  };

  return {
    rematch: {
      disabled: slugBlank || !options.matched,
      title: slugBlank
        ? "请先填写模型名称"
        : (options.matched
          ? "按当前模型名重新匹配内置元数据并重填下方内容"
          : "当前模型名没有内置元数据可匹配"),
    },
    clear: {
      disabled: !options.imported,
      title: options.imported
        ? "清除该模型的自定义元数据与未保存内容，生成时改用内置"
        : "该模型没有自定义元数据可清除",
    },
    cancel: { disabled: false, title: "放弃本次在面板里的改动，不写入任何配置" },
    save,
    status: importPanelStatus(options, fallbackSlug, decision),
  };
}

function importPanelStatus(
  options: {
    imported: boolean;
    matched: boolean;
    matchedSource?: string;
  },
  fallbackSlug: string,
  decision: ImportSaveDecision,
): ImportPanelStatus {
  // matched 为真但来源缺失时不编造供应商名：只声称「内置」，具体来源留空。
  const source = options.matchedSource || "内置";
  const hasSource = Boolean(options.matched);
  // 无内置时：命中不到就是回退，来源写清楚避免用户猜
  const effectiveSource = hasSource ? source : fallbackSlug;

  if (!hasSource && !options.imported) {
    return {
      tone: "fallback",
      text: `无内置元数据，生成时回退 ${fallbackSlug}`,
      title: `没有内置元数据可用，生成时回退 ${fallbackSlug} 官方模板；可粘贴供应商 JSON 或手动编辑`,
    };
  }

  const savedLabel = options.imported
    ? "当前使用自定义配置"
    : `当前使用内置元数据（${effectiveSource}）`;
  const liveHint = "窗口与压缩比随编辑实时生效，取消可撤销";

  // 清空文本 / 内容与内置一致 且已有自定义：预告保存后会恢复内置
  if (decision.effect === "builtin") {
    return {
      tone: "custom",
      text: `${savedLabel} · 保存后恢复内置`,
      title: `保存后清除该模型的自定义配置，改用${hasSource ? `内置元数据（${source}）` : `${fallbackSlug} 官方模板`}`,
    };
  }

  // 内容与内置不同：保存后才变成自定义
  if (decision.effect === "custom") {
    const overridden = options.imported ? "覆盖当前自定义配置" : `覆盖内置（${effectiveSource}）`;
    return {
      tone: "custom",
      text: `保存后：该模型改用这份自定义配置，${overridden}`,
      title: `窗口与压缩比已实时写回模型行；元数据保存后覆盖${options.imported ? "当前自定义配置" : "内置"}`,
    };
  }

  // 无可保存：如实说明当前就在用什么
  return {
    // 自定义优先：内置只是底层事实，用户看到的是「当前用它自己的配置」
    tone: options.imported ? "custom" : (hasSource ? "builtin" : "fallback"),
    text: `${savedLabel} · ${liveHint}`,
    title: hasSource
      ? `已匹配内置元数据（${source}）；不导入时生成也会自动使用内置数据`
      : `没有内置元数据，生成时回退 ${fallbackSlug} 官方模板；当前为自定义配置`,
  };
}

export function replaceModelMetadataForSlug(
  value: string,
  slug: string,
  metadata: ModelMetadata,
): string {
  const map = parseModelMetadataMap(value);
  const imported = filteredMetadata(metadata);
  const existing = map[slug];
  // Codex++ 中已经编辑过的显示名称是用户意图，导入供应商 metadata 时不要覆盖它。
  if (typeof existing?.display_name === "string" && existing.display_name.trim()) {
    imported.display_name = existing.display_name;
  }
  if (Object.keys(imported).length > 0) map[slug] = imported;
  else delete map[slug];
  return serializeModelMetadataMap(map);
}

export function clearModelMetadataForSlug(value: string, slug: string): string {
  const map = parseModelMetadataMap(value);
  delete map[slug];
  return serializeModelMetadataMap(map);
}

export function remapModelMetadataSlugs(
  value: string,
  mappings: Iterable<{ previousSlug: string; nextSlug: string }>,
): string {
  const map = parseModelMetadataMap(value);
  const normalized = Array.from(mappings, ({ previousSlug, nextSlug }) => ({
    previousSlug: previousSlug.trim(),
    nextSlug: nextSlug.trim(),
  }));
  const retainedSources = new Set(
    normalized
      .filter(({ previousSlug, nextSlug }) => previousSlug && previousSlug === nextSlug)
      .map(({ previousSlug }) => previousSlug),
  );
  const moves = normalized.filter(({ previousSlug, nextSlug }) => (
    previousSlug && nextSlug && previousSlug !== nextSlug && map[previousSlug]
  ));
  if (!moves.length) return value;

  const movedKeys = new Set(moves.map(({ nextSlug }) => nextSlug));
  for (const { previousSlug } of moves) {
    if (!retainedSources.has(previousSlug)) movedKeys.add(previousSlug);
  }
  const next: ModelMetadataMap = Object.fromEntries(
    Object.entries(map).filter(([key]) => !movedKeys.has(key)),
  );
  for (const { previousSlug, nextSlug } of moves) next[nextSlug] = map[previousSlug];
  return serializeModelMetadataMap(next);
}

export function retainModelMetadataForSlugs(value: string, slugs: Iterable<string>): string {
  const allowed = new Set(Array.from(slugs, (slug) => slug.trim()).filter(Boolean));
  const map = parseModelMetadataMap(value);
  return serializeModelMetadataMap(Object.fromEntries(
    Object.entries(map).filter(([slug]) => allowed.has(slug)),
  ));
}

function unwrapJsonCompatibleDocument(source: string): string {
  let text = source.trim().replace(/^\uFEFF/, "");
  const fenced = text.match(/^```(?:json|js|javascript)?\s*([\s\S]*?)\s*```$/i);
  if (fenced) text = fenced[1].trim();
  text = text
    .replace(/^export\s+default\s+/i, "")
    .replace(/^module\.exports\s*=\s*/i, "")
    .replace(/^(?:const|let|var)\s+[A-Za-z_$][\w$]*\s*=\s*/i, "")
    .trim();
  return text.replace(/;\s*$/, "").trim();
}

// 供应商 Model Key 大小写不统一（如智谱 GLM-5.3-FlashX），上游 API 对大小写宽容，
// 本地 slug 匹配若用严格相等会漏配元数据。
function slugMatchesIgnoreCase(candidateSlug: unknown, targetSlug: string): boolean {
  // targetSlug 允许是带 [1M] 后缀的模型行名：先剥成规范 slug 再比较，
  // 否则带后缀的行名永远匹配不到不带后缀的文档条目。
  return typeof candidateSlug === "string"
    && candidateSlug.toLowerCase() === modelSlugFromRowName(targetSlug).toLowerCase();
}

function documentCandidates(root: unknown): ModelMetadata[] | null {
  if (Array.isArray(root)) return root.filter(isRecord);
  if (isRecord(root) && Array.isArray(root.models)) return root.models.filter(isRecord);
  if (isRecord(root) && typeof root.slug === "string") return [root];
  return null;
}

// 强制管理字段顺序，避免保存后 context_window 跑到压缩字段之后。
function reorderManagedModelFields(model: ModelMetadata): void {
  const ordered: ModelMetadata = {};
  for (const key of ["slug", "context_window", "max_context_window", "auto_compact_token_limit"]) {
    if (Object.hasOwn(model, key)) ordered[key] = model[key];
  }
  for (const [key, value] of Object.entries(model)) {
    if (!Object.hasOwn(ordered, key)) ordered[key] = value;
  }
  for (const key of Object.keys(model)) delete model[key];
  Object.assign(model, ordered);
}

export function synchronizeModelMetadataDocumentContextWindow(
  source: string,
  targetSlug: string,
  contextWindow: string,
): string | null {
  let root: unknown;
  try {
    root = JSON.parse(unwrapJsonCompatibleDocument(source));
  } catch {
    return null;
  }
  const candidates = documentCandidates(root);
  if (!candidates) return null;
  const matches = candidates.filter((candidate) => slugMatchesIgnoreCase(candidate.slug, targetSlug));
  if (matches.length !== 1) return null;
  const trimmed = contextWindow.trim();
  const tokens = contextWindowToTokens(trimmed);
  if (trimmed && !tokens) return null;
  if (tokens) {
    matches[0].context_window = tokens;
    // max_context_window 是 codex 运行时的 clamp 权威（issue #2191）：
    // 文档条目若带该字段，必须与界面窗口同值，否则重新解析时它仍会赢。
    if (Object.hasOwn(matches[0], "max_context_window")) {
      matches[0].max_context_window = tokens;
    }
  } else if (Object.hasOwn(matches[0], "context_window")) {
    // 保留供应商字段位置；null 表示界面清空，重新填写时不会把键移到末尾。
    matches[0].context_window = null;
    if (Object.hasOwn(matches[0], "max_context_window")) {
      matches[0].max_context_window = null;
    }
  }
  reorderManagedModelFields(matches[0]);
  return JSON.stringify(root, null, 2);
}

export function synchronizeModelMetadataDocumentLimits(
  source: string,
  targetSlug: string,
  contextWindow: string,
  autoCompactPercent: string,
): string | null {
  const synchronized = synchronizeModelMetadataDocumentContextWindow(source, targetSlug, contextWindow);
  if (synchronized === null) return null;
  let root: unknown;
  try {
    root = JSON.parse(synchronized);
  } catch {
    return null;
  }
  const candidates = documentCandidates(root);
  if (!candidates) return null;
  const matches = candidates.filter((candidate) => slugMatchesIgnoreCase(candidate.slug, targetSlug));
  if (matches.length !== 1) return null;
  const compactTokenLimit = autoCompactPercentToTokenLimit(contextWindow, autoCompactPercent);
  if (compactTokenLimit) matches[0].auto_compact_token_limit = compactTokenLimit;
  else if (Object.hasOwn(matches[0], "auto_compact_token_limit")) {
    // 保留供应商 JSON 的字段位置，清空只写 null；再次输入时不会把字段移到末尾。
    matches[0].auto_compact_token_limit = null;
  }
  reorderManagedModelFields(matches[0]);
  return JSON.stringify(root, null, 2);
}

export function synchronizeModelMetadataDocumentLimitsPreview(
  source: string,
  targetSlug: string,
  contextWindow: string,
  autoCompactPercent: string,
): { document: string; preview: ImportedModelMetadata } | null {
  const document = synchronizeModelMetadataDocumentLimits(source, targetSlug, contextWindow, autoCompactPercent);
  if (document === null) return null;
  const parsed = parseModelMetadataDocument(document, targetSlug);
  if (!parsed.ok) return null;
  return {
    document,
    preview: {
      ...parsed.value,
      autoCompactPercent: autoCompactPercent.trim()
        ? displayAutoCompactPercent(parsed.value.autoCompactPercent)
        : "",
      autoCompactCalculationPercent: autoCompactPercent.trim()
        ? normalizeAutoCompactPercent(autoCompactPercent)
        : "",
    },
  };
}

function positiveIntegerString(value: unknown): string | null {
  if (typeof value === "number" && Number.isSafeInteger(value) && value > 0) return String(value);
  if (typeof value === "string" && /^\d+$/.test(value.trim())) {
    const parsed = Number(value.trim());
    return Number.isSafeInteger(parsed) && parsed > 0 ? String(parsed) : null;
  }
  return null;
}

export function validateModelCapabilities(metadata: ModelMetadata): string | null {
  if (Object.hasOwn(metadata, "supported_reasoning_levels")) {
    if (!Array.isArray(metadata.supported_reasoning_levels)) {
      return "supported_reasoning_levels 必须是数组。";
    }
    for (const item of metadata.supported_reasoning_levels) {
      if (!isRecord(item) || typeof item.effort !== "string" || !item.effort.trim() || typeof item.description !== "string") {
        return "supported_reasoning_levels 的每一项都必须包含 effort 和 description 字符串。";
      }
    }
  }
  if (Object.hasOwn(metadata, "default_reasoning_level") && metadata.default_reasoning_level !== null
    && (typeof metadata.default_reasoning_level !== "string" || !metadata.default_reasoning_level.trim())) {
    return "default_reasoning_level 必须是非空字符串。";
  }
  if (Object.hasOwn(metadata, "support_verbosity") && typeof metadata.support_verbosity !== "boolean") {
    return "support_verbosity 必须是 true 或 false。";
  }
  if (Object.hasOwn(metadata, "default_verbosity") && metadata.default_verbosity !== null
    && (typeof metadata.default_verbosity !== "string" || !metadata.default_verbosity.trim())) {
    return "default_verbosity 必须是非空字符串。";
  }
  return null;
}

export function parseModelMetadataDocument(source: string, targetSlug: string): ModelMetadataImportResult {
  if (!source.trim()) return { ok: false, error: "请先粘贴 model.js 或 JSON 配置。" };
  if (!targetSlug.trim()) return { ok: false, error: "当前模型名称为空，无法匹配 slug。" };

  let root: unknown;
  try {
    root = JSON.parse(unwrapJsonCompatibleDocument(source));
  } catch {
    return {
      ok: false,
      error: "无法解析配置。仅支持 JSON，或 export default / module.exports 包裹的 JSON；不会执行 JavaScript。",
    };
  }
  const candidates = documentCandidates(root);
  if (!candidates) return { ok: false, error: "配置中没有找到 models 数组或带 slug 的模型对象。" };
  const matches = candidates.filter((model) => slugMatchesIgnoreCase(model.slug, targetSlug));
  if (matches.length === 0) {
    const available = candidates
      .map((model) => model.slug)
      .filter((slug): slug is string => typeof slug === "string" && slug.length > 0);
    const suffix = available.length > 0 ? ` 文档包含：${available.join("、")}。` : "";
    return { ok: false, error: `文档中没有找到当前模型 slug：${targetSlug}。${suffix}` };
  }
  if (matches.length > 1) return { ok: false, error: `文档中存在多个 slug 为 ${targetSlug} 的模型，无法确定要导入哪一个。` };

  const model = matches[0];
  // 窗口提取 max_context_window 优先：它是 codex 运行时的 clamp 权威
  // （openai/codex#19185），取 context_window 会把模型真实能力上限写低。
  // 两者同值与仅其一的常见形态不受影响。
  const windowField = Object.hasOwn(model, "max_context_window") && model.max_context_window !== null
    ? "max_context_window"
    : "context_window";
  let contextWindow: string | null = null;
  if (Object.hasOwn(model, windowField) && model[windowField] !== null) {
    contextWindow = positiveIntegerString(model[windowField]);
    if (!contextWindow) return { ok: false, error: `${windowField} 必须是正整数。` };
  }
  let autoCompactPercent: string | null = null;
  if (Object.hasOwn(model, "auto_compact_token_limit") && model.auto_compact_token_limit !== null) {
    const limit = positiveIntegerString(model.auto_compact_token_limit);
    if (!limit) return { ok: false, error: "auto_compact_token_limit 必须是正整数或 null。" };
    if (!contextWindow) return { ok: false, error: "存在 auto_compact_token_limit 时必须同时提供 context_window 或 max_context_window。" };
    autoCompactPercent = autoCompactTokenLimitToPercent(contextWindow, limit);
    if (!autoCompactPercent) return { ok: false, error: "auto_compact_token_limit 必须小于或等于 context_window。" };
  }

  const metadata = filteredMetadata(model);
  const ignoredFields = Object.keys(model).filter((key) => MANAGED_MODEL_METADATA_FIELDS.has(key));
  if (typeof metadata.supports_reasoning_summaries === "boolean"
    && !Object.hasOwn(metadata, "supports_reasoning_summary_parameter")) {
    metadata.supports_reasoning_summary_parameter = metadata.supports_reasoning_summaries;
  }
  const capabilityError = validateModelCapabilities(metadata);
  if (capabilityError) return { ok: false, error: capabilityError };
  return {
    ok: true,
    value: {
      slug: targetSlug,
      metadata,
      contextWindow,
      autoCompactPercent: displayAutoCompactPercent(autoCompactPercent),
      autoCompactCalculationPercent: autoCompactPercent,
      ignoredFields,
    },
  };
}
