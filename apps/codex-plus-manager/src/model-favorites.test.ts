import assert from "node:assert/strict";
import test from "node:test";
import { isFavoriteModel, toggleFavoriteModel, type GatewayPayload } from "./gateway-model.ts";

function payload(): GatewayPayload {
  return {
    gateway: { version: 2, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: true },
    port: 1,
    helperRunning: true,
    pool: [
      { id: "chatgpt", name: "ChatGPT Pro", baseUrl: "", protocol: "responses", official: true, models: ["gpt-6.1-sol", "gpt-6-sol"], keyCount: 1, enabled: true },
      { id: "jojo", name: "jojocodemax", baseUrl: "https://jojo.test/v1", protocol: "responses", official: false, models: ["gpt-6.1-sol", "gpt-image-2"], keyCount: 1, enabled: true },
    ],
    groups: [],
    agents: [],
    health: { keys: [], providers: [] },
    codexProviders: [],
    claudeProviders: [],
    grokProfiles: [],
  };
}

test("旧版收藏的裸名：多家都有的模型在各家名下的那几份和「自动」那份都显示已收藏", () => {
  const data = payload();
  const legacy = ["gpt-6.1-sol"];
  for (const model of ["chatgpt/gpt-6.1-sol", "jojo/gpt-6.1-sol", "gpt-6.1-sol"]) assert.equal(isFavoriteModel(legacy, model, data), true, model);
  assert.equal(isFavoriteModel(legacy, "gpt-6-sol", data), false);
  assert.equal(isFavoriteModel(["gpt-image-2"], "gpt-image-2", data), true, "只有一家的模型照旧");
});

test("取消一份只取消那一份；新收藏按各自那份记", () => {
  const data = payload();
  // 取消 jojo 那份：ChatGPT 那份和「自动」那份还在。
  const afterPinned = toggleFavoriteModel(["gpt-6.1-sol"], "jojo/gpt-6.1-sol", data);
  assert.equal(isFavoriteModel(afterPinned, "jojo/gpt-6.1-sol", data), false);
  assert.equal(isFavoriteModel(afterPinned, "chatgpt/gpt-6.1-sol", data), true);
  assert.equal(isFavoriteModel(afterPinned, "gpt-6.1-sol", data), true);
  // 取消「自动」那份：各家那两份还在。
  const afterAuto = toggleFavoriteModel(["gpt-6.1-sol"], "gpt-6.1-sol", data);
  assert.equal(isFavoriteModel(afterAuto, "gpt-6.1-sol", data), false);
  assert.equal(isFavoriteModel(afterAuto, "chatgpt/gpt-6.1-sol", data), true);
  assert.equal(isFavoriteModel(afterAuto, "jojo/gpt-6.1-sol", data), true);
  // 新收藏某一家的那份，只点亮那一份。
  const fresh = toggleFavoriteModel([], "jojo/gpt-6.1-sol", data);
  assert.deepEqual(fresh, ["jojo/gpt-6.1-sol"]);
  assert.equal(isFavoriteModel(fresh, "chatgpt/gpt-6.1-sol", data), false);
});
