import { DEFAULT_AUTO_COMPACT_PERCENT, normalizeAutoCompactPercent } from "./auto-compact.ts";

export type ModelMetadata = Record<string, unknown>;
export type ModelMetadataMap = Record<string, ModelMetadata>;

export type ImportedModelMetadata = {
  slug: string;
  /// 事实字段（过 filteredMetadata 白名单，写 metadata map 用）
  metadata: ModelMetadata;
  /// 文档匹配条目原值（含窗口/压缩托管字段）。「内容是否等于内置」的全字段
  /// 比较用这份，不能用 metadata——窗口/压缩偏离内置值同样是用户编辑。
  documentEntry: ModelMetadata;
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

export function modelMetadataKey(rowName: string): string {
  return parseModelRowName(rowName).key;
}

function canonicalizeModelMetadataMap(map: ModelMetadataMap): ModelMetadataMap {
  const canonical: ModelMetadataMap = {};
  for (const [slug, metadata] of Object.entries(map)) {
    const key = modelMetadataKey(slug);
    if (!key) continue;
    canonical[key] = metadata;
  }
  return canonical;
}

export function parseModelMetadataMap(value: string): ModelMetadataMap {
  if (!value.trim()) return {};
  try {
    const parsed: unknown = JSON.parse(value);
    if (!isRecord(parsed)) return {};
    // 不丢空条目：仅改窗口/压缩的「自定义」落盘后事实字段为空对象，读侧抹掉
    // 会让徽标与清除判定重新失明（写侧 Rust 已接受 {} 条目）。
    return canonicalizeModelMetadataMap(Object.fromEntries(
      Object.entries(parsed)
        .filter((entry): entry is [string, ModelMetadata] => isRecord(entry[1]))
        .map(([slug, metadata]) => [slug, filteredMetadata(metadata)] as [string, ModelMetadata]),
    ));
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

export type ModelRowIdentity = {
  rawName: string;
  canonicalSlug: string;
  suffixWindow: string | null;
  key: string;
};

/** 统一解析模型行名，所有 metadata map 和 suffix 入口都使用这个结果。 */
export function parseModelRowName(rowName: string): ModelRowIdentity {
  const rawName = rowName;
  const trimmed = rowName.trim();
  const match = MODEL_SUFFIX_PATTERN.exec(trimmed);
  const suffixWindow = match ? suffixWindowTokens(match[2]) : null;
  const canonicalSlug = match && suffixWindow !== null ? match[1].trim() : trimmed;
  return {
    rawName,
    canonicalSlug,
    suffixWindow: suffixWindow === null ? null : String(suffixWindow),
    key: canonicalSlug.toLowerCase(),
  };
}

/// 从模型行名拆出规范 slug；无后缀或后缀非法时返回去掉首尾空白的原串。
export function modelSlugFromRowName(rowName: string): string {
  return parseModelRowName(rowName).canonicalSlug;
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
  return parseModelRowName(rowName).suffixWindow;
}

/// 内置元数据命中时的行列回填裁决（窗口 + 压缩）：仅当对应列为空且无 [1M]
/// 后缀时回填，后缀与用户已填值都是显式意图，不得覆盖。
/// - 窗口：取内置 context_window（对内置命中的模型，留空的真实含义就是
///   「用内置窗口」，回填把它显式化，避免被误解为「Codex 默认长度」）。
/// - 压缩：优先取内置 auto_compact_token_limit 换算的百分比（当前内置资产
///   均为 null，预留厂商未来提供值的通路）；否则回落 Codex++ 默认 90%——
///   空列的真实含义就是「用默认 90%」。
/// 上游获取、手动提交行名、打开导入面板三个入口共用这一裁决。
export function builtinRowBackfillValue(
  slug: string,
  rowWindow: string,
  rowAutoCompact: string,
  builtin?: { context_window?: unknown; auto_compact_token_limit?: unknown } | null,
): { window: string | null; autoCompact: string | null } {
  if (!builtin || suffixWindowString(slug)) return { window: null, autoCompact: null };
  const window = rowWindow.trim() || typeof builtin.context_window !== "number"
    || !Number.isFinite(builtin.context_window) || builtin.context_window <= 0
    ? null
    : String(builtin.context_window);
  const limit = builtin.auto_compact_token_limit;
  const derived = typeof limit === "number" && Number.isFinite(limit) && limit > 0 && window
    ? displayAutoCompactPercent(autoCompactTokenLimitToPercent(window, String(limit)))
    : null;
  const autoCompact = rowAutoCompact.trim() ? null : (derived ?? DEFAULT_AUTO_COMPACT_PERCENT);
  return { window, autoCompact };
}

export type BuiltinModelMetadataMatch = {
  matched: boolean;
  source?: string;
  entry?: BuiltinModelMetadataEntry;
  fallback?: { slug: string; context_window: number };
};

/** 内置元数据查询的三态结果：命中、成功但未命中、命令失败。
 * `null` 不再同时承担“未命中”和“IPC 出错”两种语义。
 */
export type BuiltinMetadataQueryState =
  | { status: "matched"; value: BuiltinModelMetadataMatch & { matched: true; entry: BuiltinModelMetadataEntry } }
  | { status: "miss"; value: BuiltinModelMetadataMatch & { matched: false } }
  | { status: "error"; error: string };

export function builtinMetadataQueryState(
  value: BuiltinModelMetadataMatch | null | undefined,
  error?: unknown,
): BuiltinMetadataQueryState {
  if (error !== undefined && error !== null) {
    return { status: "error", error: error instanceof Error ? error.message : String(error) };
  }
  if (value?.matched && value.entry) {
    return { status: "matched", value: value as BuiltinModelMetadataMatch & { matched: true; entry: BuiltinModelMetadataEntry } };
  }
  return { status: "miss", value: (value ?? { matched: false }) as BuiltinModelMetadataMatch & { matched: false } };
}

export type ActiveImportDraft = {
  index: number;
  /// 行已不存在时的兜底身份（activeImportSlug 回退用）。名称不参与取消回滚：
  /// 改名在 blur 时已经 commitModelSlug 持久化（metadata remap 不可逆），
  /// 名称是行的身份而非面板内容，回滚会造成配置错乱。
  originalSlug: string;
  originalWindow: string;
  originalAutoCompact: string;
  document: string;
  preview: ImportedModelMetadata | null;
};

export function createActiveImportDraft(input: {
  index: number;
  rowName: string;
  window: string;
  autoCompact: string;
  document?: string;
  preview?: ImportedModelMetadata | null;
}): ActiveImportDraft {
  return {
    index: input.index,
    originalSlug: input.rowName.trim(),
    originalWindow: input.window,
    originalAutoCompact: input.autoCompact,
    document: input.document ?? "",
    preview: input.preview ?? null,
  };
}

export function updateActiveImportDraft(
  draft: ActiveImportDraft,
  patch: Partial<Pick<ActiveImportDraft, "document" | "preview">>,
): ActiveImportDraft {
  return { ...draft, ...patch };
}

export function cancelActiveImportDraft(
  draft: ActiveImportDraft,
): { draft: null; rowPatch: { window: string; autoCompact: string } } {
  return {
    draft: null,
    rowPatch: { window: draft.originalWindow, autoCompact: draft.originalAutoCompact },
  };
}

export function rematchActiveImportDraft(
  draft: ActiveImportDraft,
  document: string,
  preview: ImportedModelMetadata | null,
): ActiveImportDraft {
  // 只替换面板内容，不动 originalSlug：打开面板时的行名是「尚未迁移的旧 key」
  // 的兜底，被改名污染后 resolveModelMetadataRowKey 就找不到 pending rename
  // 下的配置了。
  return { ...draft, document, preview };
}

/// 内置条目 → 导入文档文本：剥掉窗口/压缩四个托管字段（serialize 会按
/// 窗口参数重写），保留供应商事实字段。窗口取 context_window 优先——它是
/// codex 的默认运行窗口（官方 gpt 系为 272000/872000，导入 872000 会把
/// 运行窗口改成上限，改变默认行为）；max 仅作 context 缺失时的回退。
export function builtinEntryToImportDocument(
  entry: BuiltinModelMetadataEntry,
  rowName?: string,
): string {
  const contextWindow = entry.context_window ?? entry.max_context_window;
  // 行名剥掉 [1M] 等后缀再写入文档，保留用户的拼写（大小写一致）。
  const rowSlug = rowName?.trim() ? modelSlugFromRowName(rowName) : "";
  return serializeModelMetadataDocument(
    rowSlug || entry.slug,
    entry as ModelMetadata,
    contextWindow ? String(contextWindow) : "",
  );
}

/// 标签文案以「中文 key + 插值参数」下发，由调用方过 t()/tf()：
/// 裸字符串会被 i18n-verify.mjs 漏掉（它只扫调用点），英文模式直接露中文。
/// key 全部登记在 i18n-en.ts 的 EN_TEMPLATE/EN_PLAIN 里。
export type MetadataSourceTag =
  | { kind: "match"; text: string; title: string; tone: "builtin" }
  | { kind: "fallback"; text: string; title: string; tone: "fallback" }
  | { kind: "custom"; text: string; title: string; tone: "custom" };

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
      text: `匹配：${matchedSource}`,
      title: `内置元数据：${matchedSource}`,
      tone: "builtin",
    });
  } else if (!imported) {
    tags.push({
      kind: "fallback",
      text: `回退：${fallbackSlug}`,
      title: `无内置元数据，生成时回退 ${fallbackSlug} 官方模板`,
      tone: "fallback",
    });
  }
  if (imported) {
    tags.push({
      kind: "custom",
      text: "自定义",
      title: matchedSource
        ? `已导入自定义元数据，生成时覆盖内置（${matchedSource}）`
        : "已导入自定义元数据，生成时以该配置为准",
      tone: "custom",
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
/// 口径是「全字段等价」（含窗口/压缩托管字段）：这里回答的是「内容是否就是
/// 内置的复刻」，窗口/压缩虽由「上下文窗口」列管辖、不进 metadata map，但它们
/// 是用户可编辑内容的一部分——偏离内置值同样是部分编辑，保存应落自定义。
/// 重新匹配填回的文档就是内置原值，全字段相等仍判内置，不会把内置复制成
/// 自定义（c00177e 的语义保持成立）。写 map 的白名单过滤不在此处
/// （见 filteredMetadata / replaceModelMetadataForSlug）。
export function metadataMatchesBuiltin(
  metadata: ModelMetadata | null | undefined,
  builtinMetadata: ModelMetadata | null | undefined,
): boolean {
  if (!metadata || !builtinMetadata) return false;
  // 空对象不是合法基线：否则「只有托管字段的内置条目」会与任意窗口-only 编辑判等。
  if (Object.keys(metadata).length === 0 || Object.keys(builtinMetadata).length === 0) return false;
  return stableMetadataKey(metadata) === stableMetadataKey(builtinMetadata);
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
      ? { needsSave: true, effect: "builtin", label: "保存此模型", title: "保存后清除该模型的自定义配置，生成时回退默认模板" }
      : { needsSave: false, effect: "none", label, title: "没有可保存的内容" };
  }
  // 内容与内置一致：目标态就是内置，本来就已经在用，不写覆盖
  if (options.matchesBuiltin) {
    return options.imported
      ? { needsSave: true, effect: "builtin", label: "保存此模型", title: "内容与内置元数据一致，保存后使用内置元数据" }
      : { needsSave: false, effect: "none", label, title: "内容与内置元数据一致，保存后继续使用内置元数据" };
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

/// 导入区四个按钮的判定来源。
/// 存在意义：把原先散在 JSX 里的四组显隐/置灰条件收成一處，使按钮「始终在同一
/// 位置、只是能不能点」——不会再出现点一个键就少一个键的情况。
export type ImportPanelControls = {
  rematch: { disabled: boolean; title: string };
  clear: { disabled: boolean; title: string };
  cancel: { disabled: boolean; title: string };
  save: { disabled: boolean; label: string; title: string };
};

export function importPanelControls(options: {
  slug: string;
  document: string;
  imported: boolean;
  parseOk: boolean;
  matched: boolean;
  /// 面板元数据与内置条目是否等价（由调用方用 metadataMatchesBuiltin 算出）
  matchesBuiltin: boolean;
}): ImportPanelControls {
  const slugBlank = !options.slug.trim();
  const documentBlank = !options.document.trim();
  const decision = importSaveDecision({
    parseOk: options.parseOk,
    documentBlank,
    imported: options.imported,
    matchesBuiltin: options.matchesBuiltin,
  });
  // 保存键只在 JSON 解析失败时置灰（对齐主面板行为）：内容与内置一致时
  // 点保存=确认用内置并关闭面板（不写自定义，见 applyModelMetadataImport），
  // 不再因为「没什么可写」而把保存键禁掉。
  const save = {
    disabled: !options.parseOk,
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
    // 「清除」=清空面板文档（draft 内容），面板保持打开；已保存的配置与窗口/
    // 压缩列不受影响，恒可点。摘除自定义配置走保存路径：清除 → 重新匹配 →
    // 保存（内容=内置 → 目标态内置），或 清除 → 保存（空文本 + 已有自定义）。
    clear: { disabled: false, title: "清空面板内容（不影响已保存的配置）" },
    cancel: { disabled: false, title: "放弃本次在面板里的改动，不写入任何配置" },
    save,
  };
}

export function replaceModelMetadataForSlug(
  value: string,
  slug: string,
  metadata: ModelMetadata,
): string {
  const map = parseModelMetadataMap(value);
  const key = modelMetadataKey(slug);
  if (!key) return serializeModelMetadataMap(map);
  const imported = filteredMetadata(metadata);
  const existing = map[key];
  if (typeof existing?.display_name === "string" && existing.display_name.trim()) {
    imported.display_name = existing.display_name;
  }
  // 保存语义是「写入该模型的自定义配置」：过滤后为空也落 {}，让仅改窗口/
  // 压缩的部分编辑仍被识别为自定义（窗口值本身由行列写 model_windows）。
  // 删除只归 clearModelMetadataForSlug，不得在这里静默降级成无操作。
  map[key] = imported;
  return serializeModelMetadataMap(map);
}

/// 这一行此刻的自定义配置落在哪个 key：现名命中用现名；改名尚未提交（map 还
/// 挂在原名下）时回退行的原始名。origin 必须与 resolvePendingModelSlugRenames
/// 的 previousSlug 同源（modelSlugOriginsRef），否则「面板认为有配置」和
/// 「blur 会迁移哪个 key」会分叉，清除按钮再次失明。
export function resolveModelMetadataRowKey(
  map: ModelMetadataMap,
  rowNames: { current: string; origin?: string },
): string | null {
  const current = modelMetadataKey(rowNames.current);
  if (current && map[current]) return current;
  const origin = modelMetadataKey(rowNames.origin ?? "");
  if (origin && map[origin]) return origin;
  return null;
}

export function clearModelMetadataForSlug(value: string, slug: string): string {
  const map = parseModelMetadataMap(value);
  const key = modelMetadataKey(slug);
  if (key) delete map[key];
  return serializeModelMetadataMap(map);
}

export function remapModelMetadataSlugs(
  value: string,
  mappings: Iterable<{ previousSlug: string; nextSlug: string }>,
): string {
  const map = parseModelMetadataMap(value);
  const normalized = Array.from(mappings, ({ previousSlug, nextSlug }) => ({
    previousSlug: modelMetadataKey(previousSlug),
    nextSlug: modelMetadataKey(nextSlug),
  }));
  const retainedSources = new Set(
    normalized
      .filter(({ previousSlug, nextSlug }) => previousSlug && previousSlug === nextSlug)
      .map(({ previousSlug }) => previousSlug),
  );
  const moves = normalized.filter(({ previousSlug, nextSlug }) => (
    previousSlug && nextSlug && previousSlug !== nextSlug && map[previousSlug]
  ));
  if (!moves.length) return serializeModelMetadataMap(map);

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
  const allowed = new Set(Array.from(slugs, modelMetadataKey).filter(Boolean));
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
      documentEntry: model,
      contextWindow,
      autoCompactPercent: displayAutoCompactPercent(autoCompactPercent),
      autoCompactCalculationPercent: autoCompactPercent,
      ignoredFields,
    },
  };
}
