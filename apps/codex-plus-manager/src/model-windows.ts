import { DEFAULT_AUTO_COMPACT_PERCENT, isValidAutoCompactPercent, normalizeAutoCompactPercent } from "./auto-compact.ts";

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

function asStringMap(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, string>;
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
  const seen = new Set<string>();
  for (const row of rows) {
    const model = row.model.trim();
    if (!model) continue;
    if (seen.has(model)) return { code: "duplicateModel", model };
    seen.add(model);
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
  const seen = new Set<string>();
  const append = (row: ModelWindowRow) => {
    const model = row.model.trim();
    if (!model || seen.has(model)) return;
    seen.add(model);
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

/// 按模型名称排序的比较器：忽略大小写，数字段按数值比较（gpt-4 排在 gpt-10 前）；未命名的空行始终排到最后。
export function compareModelWindowRowsByName(left: ModelWindowRow, right: ModelWindowRow): number {
  const leftModel = left.model.trim();
  const rightModel = right.model.trim();
  if (!leftModel && !rightModel) return 0;
  if (!leftModel) return 1;
  if (!rightModel) return -1;
  return leftModel.localeCompare(rightModel, "en", { numeric: true, sensitivity: "base" });
}

export type ModelWindowSortDirection = "asc" | "desc";

/// 返回按模型名称排序后的新数组，不修改入参；等值行保持原有相对顺序。
export function sortModelWindowRowsByName(
  rows: ModelWindowRow[],
  direction: ModelWindowSortDirection = "asc",
): ModelWindowRow[] {
  return sortModelWindowRowsWithOrigins(rows, [], direction).rows;
}

/// 按模型名同步重排行和对应的原始模型名（“原始名”用于把重命名正确回写 modelMetadata）。
export function sortModelWindowRowsWithOrigins(
  rows: ModelWindowRow[],
  origins: string[],
  direction: ModelWindowSortDirection = "asc",
): { rows: ModelWindowRow[]; origins: string[] } {
  const entries = rows.map((row, index) => ({
    row,
    origin: origins[index] || row.model.trim(),
  }));
  entries.sort((left, right) => {
    const result = compareModelWindowRowsByName(left.row, right.row);
    // 未命名的空行始终排在最后，不随升降序翻转。
    if (!left.row.model.trim() || !right.row.model.trim()) return result;
    return direction === "desc" ? -result : result;
  });
  return {
    rows: entries.map((entry) => entry.row),
    origins: entries.map((entry) => entry.origin),
  };
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
  // 解析 modelVlm JSON：`{"model": "vlm"/"strip"}`
  let vlmMap: Record<string, ImageHandling> = {};
  try {
    const raw = JSON.parse(modelVlm || "{}") as Record<string, unknown>;
    for (const [model, value] of Object.entries(raw)) {
      if (value === "vlm") {
        vlmMap[model] = "vlm";
      } else if (value === "strip") {
        vlmMap[model] = "strip";
      }
      // 其他值 → 不记录
    }
  } catch {
    vlmMap = {};
  }
  const rows = modelList
    .split("\n")
    .map((model) => model.trim())
    .filter(Boolean)
    .map((model) => ({
      model,
      window: typeof map[model] === "string" ? map[model] : "",
      autoCompact: normalizeAutoCompactPercent(
        typeof autoCompactMap[model] === "string"
          ? autoCompactMap[model]
          : DEFAULT_AUTO_COMPACT_PERCENT,
      ),
      imageHandling: vlmMap[model] ?? "send-as-is",
    }));
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
    modelList.push(model);
    const window = row.window.trim();
    if (window) {
      modelWindows[model] = window;
    }
    // 只持久化非默认值
    if (row.imageHandling === "vlm" || row.imageHandling === "strip") {
      modelVlm[model] = row.imageHandling;
    }
    const autoCompact = normalizeAutoCompactPercent(
      row.autoCompact?.trim() || DEFAULT_AUTO_COMPACT_PERCENT,
    );
    modelAutoCompact[model] = autoCompact;
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
