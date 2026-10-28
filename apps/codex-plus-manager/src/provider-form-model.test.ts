import assert from "node:assert/strict";
import test from "node:test";
import type { ProviderDraft } from "./gateway-model.ts";
import { addModel, defaultModel, draftKeys, draftWithPrice, formatPrice, hasPrice, removeModel, withModels } from "./provider-form-model.ts";

const draft = (patch: Partial<ProviderDraft> = {}): ProviderDraft => ({
  name: "X", baseUrl: "https://x", keys: "", protocol: "chatCompletions", model: "", modelList: "",
  priceCurrency: "CNY", priceInput: "", priceCached: "", priceOutput: "", proxy: "", ...patch,
});

test("默认模型：选中的在列表里就用它，否则用第一个", () => {
  assert.equal(defaultModel(draft({ modelList: "a\nb", model: "b" })), "b");
  assert.equal(defaultModel(draft({ modelList: "a\nb", model: "zzz" })), "a");
  assert.equal(defaultModel(draft({ modelList: "", model: "solo" })), "solo");
});

test("增删模型去重；删掉默认模型后回到第一个", () => {
  let next = addModel(draft({ modelList: "a" }), "b\na");
  assert.equal(next.modelList, "a\nb");
  next = { ...next, model: "b" };
  next = removeModel(next, "b");
  assert.equal(next.model, "");
  assert.equal(defaultModel(next), "a");
  assert.equal(withModels(draft({ model: "keep" }), ["x", "keep"]).model, "keep");
});

test("Key 按行拆，至少一行", () => {
  assert.deepEqual(draftKeys(draft()), [""]);
  assert.deepEqual(draftKeys(draft({ keys: "k1\nk2" })), ["k1", "k2"]);
});

test("价格：按匹配结果填美元，数字最多四位小数", () => {
  const next = draftWithPrice(draft(), { providerId: "deepseek", providerName: "DeepSeek", modelId: "m", currency: "USD", input: 0.15, cachedInput: 0.0030001, output: 0.6 });
  assert.deepEqual([next.priceCurrency, next.priceInput, next.priceCached, next.priceOutput], ["USD", "0.15", "0.003", "0.6"]);
  assert.equal(hasPrice(next), true);
  assert.equal(hasPrice(draft({ priceInput: "0" })), false);
  assert.equal(formatPrice(3), "3");
});
