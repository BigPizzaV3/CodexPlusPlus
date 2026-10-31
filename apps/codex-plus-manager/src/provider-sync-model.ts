/** 模型「从上游同步 / 获取」的勾选口径（#2785）：上游新增的默认不勾，列表里已有的默认留着，
 * 勾选结果算出改完的列表和默认模型。供应商详情的同步预览和编辑弹窗的勾选列表共用。 */

/** 后端 `ModelSyncPreview`，随状态一起摊平在结果里。 */
export type ModelSyncPreview = {
  /** 上游现在的全部模型。 */
  upstream: string[];
  /** 现在的列表（默认模型在最前）。 */
  current: string[];
  defaultModel: string;
  /** 上游有、列表里没有的。 */
  added: string[];
  /** 列表里有、上游已经没有的。 */
  removed: string[];
};

/** 比较用的模型名：忽略大小写和 `[1M]` 这类窗口后缀（与后端一致）。 */
export function modelSlug(model: string): string {
  return model.trim().replace(/\[[^\]]*\]$/, "").trim().toLowerCase();
}

export function sameModel(left: string, right: string): boolean {
  return modelSlug(left) === modelSlug(right);
}

/** 按勾选算同步后的列表：现有的去掉勾选移除的、保持原顺序，勾选的新增追加在后面。
 * 默认模型还在就不变，被删了换成列表第一个（与后端 apply 的口径一致）。 */
export function syncOutcome(
  preview: ModelSyncPreview,
  remove: ReadonlySet<string>,
  add: ReadonlySet<string>,
): { models: string[]; defaultModel: string; defaultReplaced: boolean } {
  const models = preview.current.filter((model) => !remove.has(model));
  for (const model of preview.added) {
    if (add.has(model) && !models.some((item) => sameModel(item, model))) models.push(model);
  }
  const kept = models.find((model) => sameModel(model, preview.defaultModel));
  return {
    models,
    defaultModel: kept ?? models[0] ?? "",
    defaultReplaced: Boolean(preview.defaultModel) && !kept,
  };
}

/** 编辑弹窗「从上游获取」勾选列表的一行。 */
export type PickerRow = {
  model: string;
  /** 已在当前列表里（默认勾上，取消勾选表示移除）。 */
  inList: boolean;
  /** 上游有这个模型。 */
  upstream: boolean;
};

/** 勾选列表：先列现有的（按原顺序，标出上游有没有），再列上游新增的（默认不勾）。 */
export function pickerRows(existing: string[], fetched: string[]): PickerRow[] {
  const rows: PickerRow[] = existing.map((model) => ({ model, inList: true, upstream: fetched.some((item) => sameModel(item, model)) }));
  for (const model of fetched) {
    if (!rows.some((row) => sameModel(row.model, model))) rows.push({ model, inList: false, upstream: true });
  }
  return rows;
}

/** 勾选结果 → 新的模型列表（按勾选列表的顺序），以及新增、移除各几个。 */
export function pickerOutcome(rows: PickerRow[], selected: ReadonlySet<string>): { models: string[]; added: number; removed: number } {
  return {
    models: rows.filter((row) => selected.has(row.model)).map((row) => row.model),
    added: rows.filter((row) => !row.inList && selected.has(row.model)).length,
    removed: rows.filter((row) => row.inList && !selected.has(row.model)).length,
  };
}

/** 按搜索词筛：忽略大小写的子串匹配。 */
export function filterModels<T>(items: T[], query: string, name: (item: T) => string): T[] {
  const needle = query.trim().toLowerCase();
  return needle ? items.filter((item) => name(item).toLowerCase().includes(needle)) : items;
}
