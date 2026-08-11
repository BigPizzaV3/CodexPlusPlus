import { DEFAULT_AUTO_COMPACT_PERCENT } from "./auto-compact.ts";

export type ModelMetadata = Record<string, unknown>;
export type ModelMetadataMap = Record<string, ModelMetadata>;

export type ImportedModelMetadata = {
  slug: string;
  metadata: ModelMetadata;
  contextWindow: string | null;
  autoCompactPercent: string | null;
  ignoredFields: string[];
};

export type ModelMetadataImportResult =
  | { ok: true; value: ImportedModelMetadata }
  | { ok: false; error: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

const MANAGED_MODEL_METADATA_FIELDS = new Set([
  "max_context_window",
  "effective_context_window_percent",
  "priority",
  "visibility",
  "supported_in_api",
]);

function isImportedMetadataField(key: string): boolean {
  return key !== "slug"
    && key !== "context_window"
    && key !== "auto_compact_token_limit"
    && !MANAGED_MODEL_METADATA_FIELDS.has(key);
}

function entriesToMap(entries: Array<[string, ModelMetadata]>): ModelMetadataMap {
  return Object.fromEntries(entries);
}

export function parseModelMetadataMap(value: string): ModelMetadataMap {
  if (!value.trim()) return {};
  try {
    const parsed: unknown = JSON.parse(value);
    if (!isRecord(parsed)) return {};
    return entriesToMap(
      Object.entries(parsed)
        .filter((entry): entry is [string, ModelMetadata] => isRecord(entry[1]))
        .map(([slug, metadata]) => [
          slug,
          Object.fromEntries(Object.entries(metadata).filter(([key]) => isImportedMetadataField(key))),
        ] as [string, ModelMetadata])
        .filter(([, metadata]) => Object.keys(metadata).length > 0),
    );
  } catch {
    return {};
  }
}

export function serializeModelMetadataMap(map: ModelMetadataMap): string {
  return Object.keys(map).length > 0 ? JSON.stringify(map) : "";
}

function contextWindowToTokens(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (/^\d+$/.test(trimmed)) {
    const tokens = Number(trimmed);
    return Number.isSafeInteger(tokens) && tokens > 0 ? tokens : null;
  }

  const compact = trimmed.match(/^(\d+)([KkMm])$/);
  if (!compact) return null;
  const multiplier = compact[2].toLowerCase() === "m" ? 1_000_000 : 1_000;
  const tokens = Number(compact[1]) * multiplier;
  return Number.isSafeInteger(tokens) && tokens > 0 ? tokens : null;
}

function autoCompactPercentToTokenLimit(contextWindow: string, autoCompactPercent: string): number | null {
  const contextWindowTokens = contextWindowToTokens(contextWindow);
  if (!contextWindowTokens) return null;
  const normalized = (autoCompactPercent.trim() || DEFAULT_AUTO_COMPACT_PERCENT).replace(/%$/, "").trim();
  const percent = Number(normalized);
  if (!Number.isFinite(percent) || percent <= 0 || percent > 100) return null;
  return Math.round(contextWindowTokens * percent / 100);
}

function autoCompactTokenLimitToPercent(contextWindow: string, tokenLimit: string): string | null {
  const contextWindowTokens = contextWindowToTokens(contextWindow);
  const compactTokens = Number(tokenLimit);
  if (!contextWindowTokens || !Number.isSafeInteger(compactTokens) || compactTokens <= 0) return null;
  const percent = compactTokens / contextWindowTokens * 100;
  if (percent > 100) return null;
  return `${Math.round(percent * 1_000_000) / 1_000_000}%`;
}

export function serializeModelMetadataDocument(
  slug: string,
  metadata: ModelMetadata,
  contextWindow: string,
  autoCompactPercent = DEFAULT_AUTO_COMPACT_PERCENT,
): string {
  const model = Object.fromEntries(
    Object.entries(metadata).filter(([key]) => isImportedMetadataField(key)),
  );
  const contextWindowTokens = contextWindowToTokens(contextWindow);
  const autoCompactTokenLimit = autoCompactPercentToTokenLimit(contextWindow, autoCompactPercent);
  return JSON.stringify({
    models: [{
      slug,
      ...(contextWindowTokens ? { context_window: contextWindowTokens } : {}),
      ...(autoCompactTokenLimit ? { auto_compact_token_limit: autoCompactTokenLimit } : {}),
      ...model,
    }],
  }, null, 2);
}

export function replaceModelMetadataForSlug(
  value: string,
  slug: string,
  metadata: ModelMetadata,
): string {
  const entries = Object.entries(parseModelMetadataMap(value)).filter(([key]) => key !== slug);
  const importedMetadata = Object.fromEntries(
    Object.entries(metadata).filter(([key]) => isImportedMetadataField(key)),
  );
  if (Object.keys(importedMetadata).length > 0) entries.push([slug, importedMetadata]);
  return serializeModelMetadataMap(entriesToMap(entries));
}

export function clearModelMetadataForSlug(value: string, slug: string): string {
  const entries = Object.entries(parseModelMetadataMap(value)).filter(([key]) => key !== slug);
  return serializeModelMetadataMap(entriesToMap(entries));
}

export function remapModelMetadataSlugs(
  value: string,
  mappings: Iterable<{ previousSlug: string; nextSlug: string }>,
): string {
  const map = parseModelMetadataMap(value);
  const normalizedMappings = Array.from(mappings, ({ previousSlug, nextSlug }) => ({
    previousSlug: previousSlug.trim(),
    nextSlug: nextSlug.trim(),
  }));
  const retainedSources = new Set(
    normalizedMappings
      .filter(({ previousSlug, nextSlug }) => previousSlug && previousSlug === nextSlug)
      .map(({ previousSlug }) => previousSlug),
  );
  const moves = normalizedMappings.filter(({ previousSlug, nextSlug }) => (
    previousSlug && nextSlug && previousSlug !== nextSlug && map[previousSlug]
  ));
  if (!moves.length) return value;

  const movedKeys = new Set(moves.map(({ nextSlug }) => nextSlug));
  for (const { previousSlug } of moves) {
    if (!retainedSources.has(previousSlug)) movedKeys.add(previousSlug);
  }
  const entries = Object.entries(map).filter(([key]) => !movedKeys.has(key));
  for (const { previousSlug, nextSlug } of moves) {
    entries.push([nextSlug, map[previousSlug]]);
  }
  return serializeModelMetadataMap(entriesToMap(entries));
}

export function retainModelMetadataForSlugs(value: string, slugs: Iterable<string>): string {
  const allowed = new Set(Array.from(slugs, (slug) => slug.trim()).filter(Boolean));
  const entries = Object.entries(parseModelMetadataMap(value)).filter(([slug]) => allowed.has(slug));
  return serializeModelMetadataMap(entriesToMap(entries));
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

  let candidates: unknown[];
  if (Array.isArray(root)) {
    candidates = root;
  } else if (isRecord(root) && Array.isArray(root.models)) {
    candidates = root.models;
  } else if (isRecord(root) && typeof root.slug === "string") {
    candidates = [root];
  } else {
    return null;
  }

  const matches = candidates.filter(
    (candidate): candidate is ModelMetadata => isRecord(candidate) && candidate.slug === targetSlug,
  );
  if (matches.length !== 1) return null;

  const trimmed = contextWindow.trim();
  const tokens = contextWindowToTokens(trimmed);
  if (trimmed && !tokens) return null;
  if (tokens) matches[0].context_window = tokens;
  else delete matches[0].context_window;
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

  let candidates: unknown[];
  if (Array.isArray(root)) {
    candidates = root;
  } else if (isRecord(root) && Array.isArray(root.models)) {
    candidates = root.models;
  } else if (isRecord(root) && typeof root.slug === "string") {
    candidates = [root];
  } else {
    return null;
  }

  const matches = candidates.filter(
    (candidate): candidate is ModelMetadata => isRecord(candidate) && candidate.slug === targetSlug,
  );
  if (matches.length !== 1) return null;

  const compactTokenLimit = autoCompactPercentToTokenLimit(contextWindow, autoCompactPercent);
  if (!compactTokenLimit) return null;
  matches[0].auto_compact_token_limit = compactTokenLimit;
  return JSON.stringify(root, null, 2);
}

function positiveIntegerString(value: unknown): string | null {
  if (typeof value === "number" && Number.isSafeInteger(value) && value > 0) {
    return String(value);
  }
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
      if (
        !isRecord(item)
        || typeof item.effort !== "string"
        || !item.effort.trim()
        || typeof item.description !== "string"
      ) {
        return "supported_reasoning_levels 的每一项都必须包含 effort 和 description 字符串。";
      }
    }
  }

  if (Object.hasOwn(metadata, "default_reasoning_level") && metadata.default_reasoning_level !== null) {
    if (typeof metadata.default_reasoning_level !== "string" || !metadata.default_reasoning_level.trim()) {
      return "default_reasoning_level 必须是非空字符串。";
    }
  }

  if (Object.hasOwn(metadata, "support_verbosity") && typeof metadata.support_verbosity !== "boolean") {
    return "support_verbosity 必须是 true 或 false。";
  }
  if (Object.hasOwn(metadata, "default_verbosity") && metadata.default_verbosity !== null) {
    if (typeof metadata.default_verbosity !== "string" || !metadata.default_verbosity.trim()) {
      return "default_verbosity 必须是非空字符串。";
    }
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

  let candidates: unknown[];
  if (Array.isArray(root)) {
    candidates = root;
  } else if (isRecord(root) && Array.isArray(root.models)) {
    candidates = root.models;
  } else if (isRecord(root) && typeof root.slug === "string") {
    candidates = [root];
  } else {
    return { ok: false, error: "配置中没有找到 models 数组或带 slug 的模型对象。" };
  }

  const models = candidates.filter(isRecord);
  const matches = models.filter((model) => model.slug === targetSlug);
  if (matches.length === 0) {
    const available = models
      .map((model) => model.slug)
      .filter((slug): slug is string => typeof slug === "string" && slug.length > 0);
    const suffix = available.length > 0 ? ` 文档包含：${available.join("、")}。` : "";
    return { ok: false, error: `文档中没有找到当前模型 slug：${targetSlug}。${suffix}` };
  }
  if (matches.length > 1) {
    return { ok: false, error: `文档中存在多个 slug 为 ${targetSlug} 的模型，无法确定要导入哪一个。` };
  }

  const model = matches[0];
  let contextWindow: string | null = null;
  if (Object.hasOwn(model, "context_window")) {
    contextWindow = positiveIntegerString(model.context_window);
    if (!contextWindow) {
      return { ok: false, error: "context_window 必须是正整数。" };
    }
  }

  let autoCompactPercent: string | null = null;
  if (Object.hasOwn(model, "auto_compact_token_limit") && model.auto_compact_token_limit !== null) {
    const autoCompactTokenLimit = positiveIntegerString(model.auto_compact_token_limit);
    if (!autoCompactTokenLimit) {
      return { ok: false, error: "auto_compact_token_limit 必须是正整数或 null。" };
    }
    if (!contextWindow) {
      return { ok: false, error: "存在 auto_compact_token_limit 时必须同时提供 context_window。" };
    }
    const derivedPercent = autoCompactTokenLimitToPercent(contextWindow, autoCompactTokenLimit);
    if (!derivedPercent) {
      return { ok: false, error: "auto_compact_token_limit 必须小于或等于 context_window。" };
    }
    autoCompactPercent = derivedPercent;
  }

  const metadata = Object.fromEntries(Object.entries(model).filter(([key]) => isImportedMetadataField(key)));
  const ignoredFields = Object.keys(model).filter((key) => MANAGED_MODEL_METADATA_FIELDS.has(key));

  if (
    typeof metadata.supports_reasoning_summaries === "boolean"
    && !Object.hasOwn(metadata, "supports_reasoning_summary_parameter")
  ) {
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
      autoCompactPercent,
      ignoredFields,
    },
  };
}
