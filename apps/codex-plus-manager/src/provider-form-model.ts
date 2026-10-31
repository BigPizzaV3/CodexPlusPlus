/** 「添加 / 编辑供应商」弹窗的草稿操作：模型列表与默认模型、多个 Key、自动价格。 */
import { splitModels, type ProviderDraft, type ProviderPrice } from "./gateway-model.ts";

/** 公开价格表的一次匹配（每百万 token，美元），与后端 PriceMatch 对应。 */
export type PriceMatch = {
  providerId: string;
  providerName: string;
  modelId: string;
  currency: string;
  input: number;
  cachedInput: number;
  output: number;
  context?: number | null;
};

export function draftModels(draft: ProviderDraft): string[] {
  return splitModels(draft.modelList);
}

/** 默认模型：显式选的那个在列表里就用它，否则用列表第一个。 */
export function defaultModel(draft: ProviderDraft): string {
  const models = draftModels(draft);
  const chosen = draft.model.trim();
  return chosen && models.includes(chosen) ? chosen : models[0] ?? chosen;
}

/** 换一份模型列表；原来的默认模型还在就保留，不在就清空（等于用第一个）。 */
export function withModels(draft: ProviderDraft, models: string[]): ProviderDraft {
  const unique = [...new Set(models.map((model) => model.trim()).filter(Boolean))];
  const keep = unique.includes(draft.model.trim()) ? draft.model.trim() : "";
  return { ...draft, modelList: unique.join("\n"), model: keep };
}

export function addModel(draft: ProviderDraft, model: string): ProviderDraft {
  return withModels(draft, [...draftModels(draft), ...splitModels(model)]);
}

export function removeModel(draft: ProviderDraft, model: string): ProviderDraft {
  return withModels(draft, draftModels(draft).filter((item) => item !== model));
}

export function draftKeys(draft: ProviderDraft): string[] {
  const keys = draft.keys.split(/\r?\n/);
  return keys.length ? keys : [""];
}

export function withKeys(draft: ProviderDraft, keys: string[]): ProviderDraft {
  return { ...draft, keys: keys.join("\n") };
}

/** 价格数字：最多 4 位小数，去掉末尾的 0。 */
export function formatPrice(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  return String(Number(value.toFixed(4)));
}

export function draftWithPrice(draft: ProviderDraft, match: PriceMatch): ProviderDraft {
  return {
    ...draft,
    priceCurrency: match.currency || "USD",
    priceInput: formatPrice(match.input),
    priceCached: formatPrice(match.cachedInput),
    priceOutput: formatPrice(match.output),
  };
}

export function hasPrice(draft: ProviderDraft): boolean {
  return [draft.priceInput, draft.priceCached, draft.priceOutput].some((value) => Number(value) > 0);
}

/** 价格表里一行的输入框文字（每百万 token）。 */
export type PriceRow = { input: string; cached: string; output: string };

export const EMPTY_PRICE_ROW: PriceRow = { input: "", cached: "", output: "" };

export function priceRowFrom(price: ProviderPrice | null | undefined): PriceRow {
  return price ? { input: String(price.input), cached: String(price.cachedInput), output: String(price.output) } : EMPTY_PRICE_ROW;
}

export function priceRowFromMatch(match: PriceMatch): PriceRow {
  return { input: formatPrice(match.input), cached: formatPrice(match.cachedInput), output: formatPrice(match.output) };
}

function priceValue(text: string): number {
  const value = Number(text.trim());
  return Number.isFinite(value) && value > 0 ? value : 0;
}

/** 一行都没填（或都是 0）时返回 null，表示这个模型不单独计价。 */
export function priceFromRow(row: PriceRow, currency: string): ProviderPrice | null {
  const price = { currency: currency || "USD", input: priceValue(row.input), cachedInput: priceValue(row.cached), output: priceValue(row.output) };
  return price.input || price.cachedInput || price.output ? price : null;
}

export function priceFromMatch(match: PriceMatch): ProviderPrice {
  return { currency: match.currency || "USD", input: match.input, cachedInput: match.cachedInput, output: match.output };
}

/** 各模型的价格表 → 提交给后端的 `modelPrices`（只留填了的）。 */
export function modelPricesFromRows(rows: Record<string, PriceRow>, currency: string): Record<string, ProviderPrice> {
  const prices: Record<string, ProviderPrice> = {};
  for (const [model, row] of Object.entries(rows)) {
    const price = priceFromRow(row, currency);
    if (price) prices[model] = price;
  }
  return prices;
}
