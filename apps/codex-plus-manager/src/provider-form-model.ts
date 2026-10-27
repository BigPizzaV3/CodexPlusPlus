/** 「添加 / 编辑供应商」弹窗的草稿操作：模型列表与默认模型、多个 Key、自动价格。 */
import { splitModels, type ProviderDraft } from "./gateway-model.ts";

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

/** 上游返回的模型太多（聚合站常有几百个）时让用户挑；少的直接替换。 */
export const FETCHED_DIRECT_LIMIT = 40;

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
