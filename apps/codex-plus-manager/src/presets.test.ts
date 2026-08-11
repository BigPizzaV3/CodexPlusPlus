import assert from "node:assert/strict";
import test from "node:test";

import { PRESETS } from "./presets.ts";

const deepseekPreset = () => {
  const preset = PRESETS.find((item) => item.id === "deepseek");
  assert.ok(preset, "DeepSeek 预设应存在");
  return preset;
};

test("DeepSeek preset uses native Responses protocol", () => {
  assert.equal(deepseekPreset().protocol, "responses");
});

test("DeepSeek preset prefills official 1M windows", () => {
  assert.deepEqual(deepseekPreset().modelList, [
    "deepseek-v4-flash[1M]",
    "deepseek-v4-pro[1M]",
  ]);
});

test("DeepSeek preset keeps flash as default model with official base URL", () => {
  assert.equal(deepseekPreset().model, "deepseek-v4-flash");
  assert.equal(deepseekPreset().baseUrl, "https://api.deepseek.com/");
});
