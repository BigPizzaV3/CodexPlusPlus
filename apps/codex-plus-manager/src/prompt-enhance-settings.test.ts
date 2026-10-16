import assert from "node:assert/strict";
import { test } from "node:test";
import {
  defaultPromptEnhanceSettings,
  normalizePromptEnhanceSettings,
  promptEnhanceSettingsIssue,
} from "./prompt-enhance-settings.ts";

test("prompt enhance is off by default and reuses the next-step connection", () => {
  const settings = normalizePromptEnhanceSettings(undefined);
  assert.deepEqual(settings, defaultPromptEnhanceSettings());
  assert.equal(settings.enabled, false);
  assert.equal(settings.useStepwiseConnection, true);
});

test("prompt enhance normalization trims fields and clamps values", () => {
  const settings = normalizePromptEnhanceSettings({
    enabled: true,
    useStepwiseConnection: false,
    baseUrl: " https://api.example.test/v1/ ",
    model: " mini ",
    protocol: "unknown",
    systemPrompt: "  只补充验收标准  ",
    timeoutSeconds: 900,
  });
  assert.equal(settings.baseUrl, "https://api.example.test/v1");
  assert.equal(settings.model, "mini");
  assert.equal(settings.protocol, "chat_completions");
  assert.equal(settings.systemPrompt, "只补充验收标准");
  assert.equal(settings.timeoutSeconds, 300);
  assert.equal(normalizePromptEnhanceSettings({ timeoutSeconds: 1 }).timeoutSeconds, 5);
  assert.equal(normalizePromptEnhanceSettings({ timeoutSeconds: NaN }).timeoutSeconds, 60);
  assert.deepEqual(normalizePromptEnhanceSettings(JSON.parse(JSON.stringify(settings))), settings);
});

test("separate connection requires base URL and model only when enabled", () => {
  const base = { ...defaultPromptEnhanceSettings(), enabled: true, useStepwiseConnection: false };
  assert.equal(promptEnhanceSettingsIssue(base), "baseUrl");
  assert.equal(promptEnhanceSettingsIssue({ ...base, baseUrl: "https://x.test/v1" }), "model");
  assert.equal(promptEnhanceSettingsIssue({ ...base, baseUrl: "https://x.test/v1", model: "m" }), null);
  assert.equal(promptEnhanceSettingsIssue({ ...base, useStepwiseConnection: true }), null);
  assert.equal(promptEnhanceSettingsIssue({ ...base, enabled: false }), null);
});
