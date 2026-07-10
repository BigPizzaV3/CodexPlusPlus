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

export type ModelWindowRow = {
  id: string;
  model: string;
  window: string;
};

let nextModelWindowRowId = 0;

export function createModelWindowRow(model = "", window = ""): ModelWindowRow {
  nextModelWindowRowId += 1;
  return { id: `model-window-${nextModelWindowRowId}`, model, window };
}

export function modelWindowTokenError(value: string): string | null {
  const token = value.trim();
  if (!token) return null;
  if (!/^[1-9]\d*(?:K|M)?$/i.test(token)) {
    return `上下文窗口“${value}”格式无效，请填写正整数或 K/M 后缀（例如 200K、1M）。`;
  }
  const suffix = token.slice(-1).toUpperCase();
  const multiplier = suffix === "K" ? 1_000n : suffix === "M" ? 1_000_000n : 1n;
  const numberText = suffix === "K" || suffix === "M" ? token.slice(0, -1) : token;
  if (BigInt(numberText) * multiplier > 9_223_372_036_854_775_807n) {
    return `上下文窗口“${value}”超出支持范围。`;
  }
  return null;
}

export function validateModelWindowRows(rows: ModelWindowRow[]): string | null {
  for (const row of rows) {
    const error = modelWindowTokenError(row.window);
    if (error) return row.model.trim() ? `模型 ${row.model.trim()}：${error}` : error;
    if (!row.model.trim() && row.window.trim()) return "填写上下文窗口前必须先填写模型名称。";
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
    rows.push({ ...row, model, window: row.window.trim() });
  };
  currentRows.forEach(append);
  incomingRows.forEach(append);
  return rows.length ? rows : [createModelWindowRow()];
}

export function modelWindowRowsFromProfile(modelList: string, modelWindows: string): ModelWindowRow[] {
  let map: Record<string, string> = {};
  try {
    map = JSON.parse(modelWindows || "{}") as Record<string, string>;
  } catch {
    map = {};
  }
  const rows = modelList
    .split("\n")
    .map((model) => model.trim())
    .filter(Boolean)
    .map((model) => createModelWindowRow(model, map[model] ?? ""));
  return rows.length ? rows : [createModelWindowRow()];
}

export function serializeModelWindowRows(rows: ModelWindowRow[]): { modelList: string; modelWindows: string } {
  const modelList: string[] = [];
  const modelWindows: Record<string, string> = {};
  mergeModelWindowRows(rows, []).forEach((row) => {
    const model = row.model.trim();
    if (!model) return;
    modelList.push(model);
    const window = row.window.trim();
    if (window) {
      modelWindows[model] = window;
    }
  });
  return {
    modelList: modelList.join("\n"),
    modelWindows: JSON.stringify(modelWindows),
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
