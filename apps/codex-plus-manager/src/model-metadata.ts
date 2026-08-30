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

// 只有 slug 和两个由界面专门编辑的数值字段不进入 metadata map。
// 其余字段属于供应商模型事实，导入时保留并在 catalog 中优先于生成默认值。
const MANAGED_MODEL_METADATA_FIELDS = new Set<string>();

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isImportedMetadataField(key: string): boolean {
  return key !== "slug"
    && key !== "context_window"
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

function documentCandidates(root: unknown): ModelMetadata[] | null {
  if (Array.isArray(root)) return root.filter(isRecord);
  if (isRecord(root) && Array.isArray(root.models)) return root.models.filter(isRecord);
  if (isRecord(root) && typeof root.slug === "string") return [root];
  return null;
}

// 强制管理字段顺序，避免保存后 context_window 跑到压缩字段之后。
function reorderManagedModelFields(model: ModelMetadata): void {
  const ordered: ModelMetadata = {};
  for (const key of ["slug", "context_window", "auto_compact_token_limit"]) {
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
  const matches = candidates.filter((candidate) => candidate.slug === targetSlug);
  if (matches.length !== 1) return null;
  const trimmed = contextWindow.trim();
  const tokens = contextWindowToTokens(trimmed);
  if (trimmed && !tokens) return null;
  if (tokens) matches[0].context_window = tokens;
  else if (Object.hasOwn(matches[0], "context_window")) {
    // 保留供应商字段位置；null 表示界面清空，重新填写时不会把键移到末尾。
    matches[0].context_window = null;
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
  const matches = candidates.filter((candidate) => candidate.slug === targetSlug);
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
  const matches = candidates.filter((model) => model.slug === targetSlug);
  if (matches.length === 0) {
    const available = candidates
      .map((model) => model.slug)
      .filter((slug): slug is string => typeof slug === "string" && slug.length > 0);
    const suffix = available.length > 0 ? ` 文档包含：${available.join("、")}。` : "";
    return { ok: false, error: `文档中没有找到当前模型 slug：${targetSlug}。${suffix}` };
  }
  if (matches.length > 1) return { ok: false, error: `文档中存在多个 slug 为 ${targetSlug} 的模型，无法确定要导入哪一个。` };

  const model = matches[0];
  let contextWindow: string | null = null;
  if (Object.hasOwn(model, "context_window") && model.context_window !== null) {
    contextWindow = positiveIntegerString(model.context_window);
    if (!contextWindow) return { ok: false, error: "context_window 必须是正整数。" };
  }
  let autoCompactPercent: string | null = null;
  if (Object.hasOwn(model, "auto_compact_token_limit") && model.auto_compact_token_limit !== null) {
    const limit = positiveIntegerString(model.auto_compact_token_limit);
    if (!limit) return { ok: false, error: "auto_compact_token_limit 必须是正整数或 null。" };
    if (!contextWindow) return { ok: false, error: "存在 auto_compact_token_limit 时必须同时提供 context_window。" };
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
