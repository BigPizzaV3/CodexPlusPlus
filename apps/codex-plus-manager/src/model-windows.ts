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

/// 图片处理模式（供后端 map 值用，前端 ImageHandling 已移除）。
export type ImageHandling = "" | "send-as-is" | "strip" | "vlm";

export type ModelWindowRow = {
  model: string;
  window: string;
  textOnly: boolean;     // 只支持文本 -> 派生 modelVlm
  noReasoning: boolean;  // 不支持推理 -> modelReasoningSupport
};

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
      textOnly: row.textOnly ?? false,
      noReasoning: row.noReasoning ?? false,
    });
  };
  currentRows.forEach(append);
  incomingRows.forEach(append);
  return rows.length ? rows : [{ model: "", window: "", textOnly: false, noReasoning: false }];
}

export function modelWindowRowsFromProfile(
  modelList: string,
  modelWindows: string,
  modelVlm?: string,
  modelReasoningSupport?: string,
): ModelWindowRow[] {
  let winMap: Record<string, string> = {};
  try { winMap = JSON.parse(modelWindows || "{}"); } catch { winMap = {}; }
  // 旧 modelVlm 迁移：vlm/strip -> textOnly=true
  const textOnlySet = new Set<string>();
  try {
    const raw = JSON.parse(modelVlm || "{}") as Record<string, unknown>;
    for (const [model, value] of Object.entries(raw)) {
      if (value === "vlm" || value === "strip") textOnlySet.add(model);
    }
  } catch { /* ignore */ }
  const noReasoningSet = new Set<string>();
  try {
    const raw = JSON.parse(modelReasoningSupport || "{}") as Record<string, unknown>;
    for (const [model, value] of Object.entries(raw)) {
      if (value === false) noReasoningSet.add(model);
    }
  } catch { /* ignore */ }
  const rows = modelList
    .split("\n").map((s) => s.trim()).filter(Boolean)
    .map((model) => ({
      model,
      window: winMap[model] ?? "",
      textOnly: textOnlySet.has(model),
      noReasoning: noReasoningSet.has(model),
    }));
  return rows.length ? rows : [{ model: "", window: "", textOnly: false, noReasoning: false }];
}

export function serializeModelWindowRows(
  rows: ModelWindowRow[],
  vlConfigured: boolean,
): { modelList: string; modelWindows: string; modelVlm: string; modelReasoningSupport: string } {
  const modelList: string[] = [];
  const modelWindows: Record<string, string> = {};
  const modelVlm: Record<string, string> = {};
  const modelReasoningSupport: Record<string, boolean> = {};
  mergeModelWindowRows(rows, []).forEach((row) => {
    const model = row.model.trim();
    if (!model) return;
    modelList.push(model);
    const window = row.window.trim();
    if (window) modelWindows[model] = window;
    if (row.textOnly) modelVlm[model] = vlConfigured ? "vlm" : "strip";
    if (row.noReasoning) modelReasoningSupport[model] = false;
  });
  return {
    modelList: modelList.join("\n"),
    modelWindows: JSON.stringify(modelWindows),
    modelVlm: JSON.stringify(modelVlm),
    modelReasoningSupport: JSON.stringify(modelReasoningSupport),
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
