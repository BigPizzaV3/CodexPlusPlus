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
  imageHandling: ImageHandling;
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
    rows.push({ model, window: row.window.trim(), imageHandling: row.imageHandling ?? "send-as-is" });
  };
  currentRows.forEach(append);
  incomingRows.forEach(append);
  return rows.length ? rows : [{ model: "", window: "", imageHandling: "send-as-is" }];
}

export function modelWindowRowsFromProfile(modelList: string, modelWindows: string, modelVlm?: string): ModelWindowRow[] {
  let map: Record<string, string> = {};
  try {
    map = JSON.parse(modelWindows || "{}") as Record<string, string>;
  } catch {
    map = {};
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
    .map((model) => ({ model, window: map[model] ?? "", imageHandling: vlmMap[model] ?? "send-as-is" }));
  return rows.length ? rows : [{ model: "", window: "", imageHandling: "send-as-is" }];
}

export function serializeModelWindowRows(rows: ModelWindowRow[]): { modelList: string; modelWindows: string; modelVlm: string } {
  const modelList: string[] = [];
  const modelWindows: Record<string, string> = {};
  const modelVlm: Record<string, string> = {};
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
  });
  return {
    modelList: modelList.join("\n"),
    modelWindows: JSON.stringify(modelWindows),
    modelVlm: JSON.stringify(modelVlm),
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
