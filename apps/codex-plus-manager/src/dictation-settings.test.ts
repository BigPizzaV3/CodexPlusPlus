import assert from "node:assert/strict";
import { test } from "node:test";
// 预设与校验在 Codex 设置页里，见 assets/inject/service-settings.test.cjs。
import { defaultDictationSettings, normalizeDictationSettings } from "./dictation-settings.ts";

test("old settings keep API dictation opt-in and each default is independent", () => {
  const first = normalizeDictationSettings(undefined);
  assert.equal(first.enabled, false);
  assert.equal(first.baseUrl, "https://api.groq.com/openai/v1");
  assert.equal(first.model, "whisper-large-v3-turbo");
  assert.equal(first.timeoutSeconds, 120);
  first.apiKey = "test-key";
  assert.equal(defaultDictationSettings().apiKey, "");
});

test("partial settings load with defaults and configured fields survive serialization", () => {
  assert.equal(normalizeDictationSettings({ language: "zh" }).model, "whisper-large-v3-turbo");
  const input = {
    enabled: true,
    baseUrl: " https://asr.example/v1/ ",
    apiKey: " test-key ",
    apiKeyEnv: " ASR_API_KEY ",
    model: " custom-whisper ",
    language: " zh ",
    timeoutSeconds: 90,
  };
  const normalized = normalizeDictationSettings(input);
  assert.equal(normalized.baseUrl, "https://asr.example/v1");
  assert.equal(normalized.apiKey, "test-key");
  assert.equal(normalized.apiKeyEnv, "ASR_API_KEY");
  assert.equal(normalized.model, "custom-whisper");
  assert.equal(normalized.language, "zh");
  assert.deepEqual(normalizeDictationSettings(JSON.parse(JSON.stringify(normalized))), normalized);
  assert.equal(input.baseUrl, " https://asr.example/v1/ ");
});

test("legacy SenseVoice marker restores the local provider", () => {
  const restored = normalizeDictationSettings({
    enabled: true,
    baseUrl: " LOCAL://SENSEVOICE/ ",
    model: "SenseVoiceSmall (INT8)",
  });
  assert.equal(restored.provider, "sensevoice");
  assert.equal(restored.baseUrl, "LOCAL://SENSEVOICE");
});

test("timeout normalization remains finite and within the backend bounds", () => {
  assert.equal(normalizeDictationSettings({ timeoutSeconds: NaN }).timeoutSeconds, 120);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: Infinity }).timeoutSeconds, 120);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: 0 }).timeoutSeconds, 1);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: 700 }).timeoutSeconds, 600);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: 90.6 }).timeoutSeconds, 91);
});
