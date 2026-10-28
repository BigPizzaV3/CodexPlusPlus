import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { providerInput, providerDraftFromPreset } from "./gateway-model.ts";
import { modelPricesFromRows, priceFromRow, priceRowFrom } from "./provider-form-model.ts";

test("价格按模型填：只提交填了的行，「其它模型」单独作为默认价", () => {
  const rows = {
    "claude-opus": { input: "15", cached: "1.5", output: "75" },
    "claude-haiku": { input: "", cached: "", output: "" },
    "claude-sonnet": { input: "3", cached: "abc", output: "15" },
  };
  assert.deepEqual(modelPricesFromRows(rows, "USD"), {
    "claude-opus": { currency: "USD", input: 15, cachedInput: 1.5, output: 75 },
    "claude-sonnet": { currency: "USD", input: 3, cachedInput: 0, output: 15 },
  });
  assert.equal(priceFromRow({ input: "0", cached: "", output: "" }, "USD"), null);
  assert.deepEqual(priceRowFrom({ currency: "CNY", input: 2, cachedInput: 0.5, output: 8 }), { input: "2", cached: "0.5", output: "8" });
});

test("编辑供应商不带 modelPrices（后端保留原来的）；新添加时带上自动查到的", () => {
  const draft = providerDraftFromPreset(null);
  assert.equal(providerInput(draft).modelPrices, null);
  const priced = { ...draft, modelPrices: { m: { currency: "USD", input: 1, cachedInput: 0, output: 2 } } };
  assert.deepEqual(providerInput(priced).modelPrices, priced.modelPrices);
});

test("价格弹窗按模型列出，提交 modelPrices", () => {
  const dialog = readFileSync(new URL("./ProviderExtrasDialog.tsx", import.meta.url), "utf8");
  assert.match(dialog, /entry\.models\.map\(\(model\) => row\(model/);
  assert.match(dialog, /lookup_model_prices", \{ baseUrl, models \}/);
  const providers = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  assert.match(providers, /"save_gateway_provider_extras", \{ id: entry\.id, price: nextPrice, modelPrices, proxy \}/);
});
