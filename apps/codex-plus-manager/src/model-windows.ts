import { DEFAULT_AUTO_COMPACT_PERCENT, isValidAutoCompactPercent, normalizeAutoCompactPercent } from "./auto-compact.ts";
import { modelSlugFromRowName } from "./model-metadata.ts";

/// 把 model_windows JSON map 按 model_list 行顺序转成文本（每行一个窗口，空行表示默认）。
export function modelWindowsMapToText(modelList: string, modelWindows: string): string {
  try {
    const map = JSON.parse(modelWindows || "{}") as Record<string, string>;
    return modelList
      .split("\n")
      .map((line) => map[line.trim()] ?? "")
      .join("\n");
  } catch {
    return "";
  }
}

/// 把左右 textarea 文本组装成 model_windows JSON map。
export function modelWindowsTextToMap(modelList: string, modelWindowsText: string): string {
  const models = modelList.split("\n").map((s) => s.trim()).filter(Boolean);
  const windows = modelWindowsText.split("\n").map((s) => s.trim());
  const map: Record<string, string> = {};
  models.forEach((model, index) => {
    if (windows[index]) {
      map[model] = windows[index];
    }
  });
  return JSON.stringify(map);
}

/// 图片处理模式。
export type ImageHandling = "" | "send-as-is" | "strip" | "vlm";

export type ModelWindowRow = {
  model: string;
  window: string;
  /// 自动压缩百分比；空值在保存时使用 Codex++ 的明确默认值 90%。
  autoCompact: string;
  imageHandling: ImageHandling;
};

export type ModelWindowRowsValidationIssue = {
  code: "duplicateModel" | "invalidWindow" | "invalidAutoCompact";
  model: string;
};

/// 按拖动结果移动模型行，保留行内的窗口、压缩和图片处理配置。
/// 索引无效或指向同一行时返回原数组引用，避免无意义的状态更新。
export function reorderModelWindowRows(
  rows: ModelWindowRow[],
  activeIndex: number,
  overIndex: number,
): ModelWindowRow[] {
  if (
    activeIndex < 0
    || activeIndex >= rows.length
    || overIndex < 0
    || overIndex >= rows.length
    || activeIndex === overIndex
  ) {
    return rows;
  }
  const next = [...rows];
  const [moved] = next.splice(activeIndex, 1);
  next.splice(overIndex, 0, moved);
  return next;
}

function asStringMap(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, string>;
}

/// 归一化模型行名得到的 map key：剥掉 `[1M]` 窗口后缀，**保留原大小写**。
///
/// 运行期查表用的是请求体里的 model 字符串：`vision.rs` 的 `image_handling_mode`
/// 做全字匹配（大小写敏感），codex 侧 `collect_catalog_entries` 也按原样 slug
/// 查 `model_windows`。所以 key 必须等于剥掉后缀后的原始 slug——统一小写会把
/// `GLM-5.3` 这类驼峰 slug 写成 `glm-5.3`，上游请求仍发 `GLM-5.3`，查表全落空
/// 反而引入新故障。归一化只做「去后缀」这一件事（issue #2345）。
export function modelMapKeyFromRowName(rowName: string): string {
  return modelSlugFromRowName(rowName.trim());
}

/// 按模型查 map：先按归一化 key 查，再依次回退到原始行名、大小写不敏感匹配。
///
/// 回退分支专门吃历史数据——旧版本保存过的 key 可能是带后缀的行名原样，直接
/// 丢掉会让用户已配好的值看起来「被重置」。
export function lookupModelMapEntry<T>(
  map: Record<string, T>,
  modelName: string,
): T | undefined {
  const raw = modelName.trim();
  if (!raw) return undefined;
  const slug = modelSlugFromRowName(raw);
  const direct = map[slug] ?? map[raw];
  if (direct !== undefined) return direct;
  const lower = slug.toLowerCase();
  for (const [key, value] of Object.entries(map)) {
    if (key.toLowerCase() === lower) return value;
  }
  return undefined;
}

export function isValidModelWindow(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return true;
  const match = trimmed.match(/^(\d+)([KkMm])?$/);
  if (!match) return false;
  const multiplier = match[2]?.toLowerCase() === "m"
    ? 1_000_000n
    : match[2]
      ? 1_000n
      : 1n;
  const tokens = BigInt(match[1]) * multiplier;
  return tokens > 0n && tokens <= 18_446_744_073_709_551_615n;
}

export function modelWindowRowsValidationError(rows: ModelWindowRow[]): ModelWindowRowsValidationIssue | null {
  // 同样按规范 slug 判重：带后缀与不带后缀的同名行会写进同一组 map key，
  // 静默覆盖比报错更难排查（issue #2345）。
  const seen = new Set<string>();
  for (const row of rows) {
    const model = row.model.trim();
    if (!model) continue;
    const dedupeKey = modelMapKeyFromRowName(model) || model;
    if (seen.has(dedupeKey)) return { code: "duplicateModel", model };
    seen.add(dedupeKey);
    if (!isValidModelWindow(row.window)) return { code: "invalidWindow", model };
    if (!isValidAutoCompactPercent(row.autoCompact ?? "")) {
      return { code: "invalidAutoCompact", model };
    }
  }
  return null;
}

export function mergeModelWindowRows(
  currentRows: ModelWindowRow[],
  incomingRows: ModelWindowRow[],
): ModelWindowRow[] {
  const rows: ModelWindowRow[] = [];
  // 去重按规范 slug：`deepseek-v4-pro[1M]` 与 `deepseek-v4-pro` 是同一个模型的
  // 两种写法，同时存在会在序列化时互相覆盖同一张 map 的同一个 key（issue #2345）。
  const seen = new Set<string>();
  const append = (row: ModelWindowRow) => {
    const model = row.model.trim();
    if (!model) return;
    const dedupeKey = modelMapKeyFromRowName(model) || model;
    if (seen.has(dedupeKey)) return;
    seen.add(dedupeKey);
    rows.push({
      model,
      window: row.window.trim(),
      autoCompact: normalizeAutoCompactPercent(row.autoCompact ?? ""),
      imageHandling: row.imageHandling ?? "send-as-is",
    });
  };
  currentRows.forEach(append);
  incomingRows.forEach(append);
  return rows.length ? rows : [{ model: "", window: "", autoCompact: "", imageHandling: "send-as-is" }];
}

export function modelWindowRowsFromProfile(
  modelList: string,
  modelWindows: string,
  modelVlm?: string,
  modelAutoCompact?: string,
): ModelWindowRow[] {
  let map: Record<string, string> = {};
  try {
    map = asStringMap(JSON.parse(modelWindows || "{}"));
  } catch {
    map = {};
  }
  let autoCompactMap: Record<string, string> = {};
  try {
    autoCompactMap = asStringMap(JSON.parse(modelAutoCompact || "{}"));
  } catch {
    autoCompactMap = {};
  }
  // 解析 modelVlm JSON：`{"model": "vlm"/"strip"}`。
  // key 按规范 slug 归一后再存：历史数据里的 key 可能是带 `[1M]` 后缀的行名，
  // 而归一化让读写两侧始终落在同一个 key 上（issue #2345）。
  const vlmMap: Record<string, ImageHandling> = {};
  try {
    const raw = JSON.parse(modelVlm || "{}") as Record<string, unknown>;
    for (const [model, value] of Object.entries(raw)) {
      const key = modelMapKeyFromRowName(model);
      if (!key) continue;
      if (value === "vlm") {
        vlmMap[key] = "vlm";
      } else if (value === "strip") {
        vlmMap[key] = "strip";
      }
      // 其他值 → 不记录（未知值不静默变 send-as-is 改写用户数据，读侧忽略即可）
    }
  } catch {
    // vlmMap 保持空对象
  }
  const rows = modelList
    .split("\n")
    .map((model) => model.trim())
    .filter(Boolean)
    .map((model) => {
      const window = lookupModelMapEntry(map, model);
      const autoCompact = lookupModelMapEntry(autoCompactMap, model);
      return {
        model,
        window: typeof window === "string" ? window : "",
        autoCompact: normalizeAutoCompactPercent(
          typeof autoCompact === "string" ? autoCompact : DEFAULT_AUTO_COMPACT_PERCENT,
        ),
        imageHandling: lookupModelMapEntry(vlmMap, model) ?? "send-as-is",
      };
    });
  return rows.length ? rows : [{ model: "", window: "", autoCompact: "", imageHandling: "send-as-is" }];
}

export function serializeModelWindowRows(rows: ModelWindowRow[]): {
  modelList: string;
  modelWindows: string;
  modelVlm: string;
  modelAutoCompact: string;
} {
  const modelList: string[] = [];
  const modelWindows: Record<string, string> = {};
  const modelVlm: Record<string, string> = {};
  const modelAutoCompact: Record<string, string> = {};
  mergeModelWindowRows(rows, []).forEach((row) => {
    const model = row.model.trim();
    if (!model) return;
    // model_list 保留用户原样行名（含 `[1M]` 后缀，codex 侧按后缀识别窗口）；
    // 三张配置 map 的 key 统一用规范 slug——运行期查表用的是剥掉后缀的 model
    // 字符串，带后缀的 key 永远查不中（issue #2345）。
    const key = modelMapKeyFromRowName(model) || model;
    modelList.push(model);
    const window = row.window.trim();
    if (window) {
      modelWindows[key] = window;
    }
    // 只持久化非默认值
    if (row.imageHandling === "vlm" || row.imageHandling === "strip") {
      modelVlm[key] = row.imageHandling;
    }
    const autoCompact = normalizeAutoCompactPercent(
      row.autoCompact?.trim() || DEFAULT_AUTO_COMPACT_PERCENT,
    );
    modelAutoCompact[key] = autoCompact;
  });
  return {
    modelList: modelList.join("\n"),
    modelWindows: JSON.stringify(modelWindows),
    modelVlm: JSON.stringify(modelVlm),
    modelAutoCompact: JSON.stringify(modelAutoCompact),
  };
}

export type BuildModelWindowsResult =
  | { ok: true; modelWindows: string }
  | { ok: false; error: string };

/// 校验模型列表与窗口文本行数一致，并组装成 model_windows JSON。
export function buildModelWindows(modelList: string, modelWindowsText: string): BuildModelWindowsResult {
  const models = modelList.split("\n").map((s) => s.trim()).filter(Boolean);
  const windows = modelWindowsText.split("\n").map((s) => s.trim());
  if (models.length !== windows.length) {
    return {
      ok: false,
      error: `模型名称有 ${models.length} 行，上下文窗口有 ${windows.length} 行，请保持行数一致。`,
    };
  }
  return { ok: true, modelWindows: modelWindowsTextToMap(modelList, modelWindowsText) };
}
