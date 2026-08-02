import assert from "node:assert/strict";
import test from "node:test";

import { PRESETS } from "./presets.ts";

test("Atlas Cloud preset uses the OpenAI-compatible chat endpoint", () => {
  const presets = PRESETS.filter((preset) => preset.id === "atlascloud");

  assert.equal(presets.length, 1);
  assert.deepEqual(presets[0], {
    id: "atlascloud",
    name: "Atlas Cloud",
    websiteUrl: "https://www.atlascloud.ai",
    apiKeyUrl: "https://www.atlascloud.ai/console/api-keys",
    category: "aggregator",
    baseUrl: "https://api.atlascloud.ai/v1",
    protocol: "chatCompletions",
    model: "deepseek-ai/deepseek-v4-pro",
    modelList: ["deepseek-ai/deepseek-v4-pro"],
  });
});
