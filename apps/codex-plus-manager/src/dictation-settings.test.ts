import assert from "node:assert/strict";
import { test } from "node:test";
import {
  applyDictationPreset,
  defaultDictationSettings,
  dictationPreset,
  dictationSettingsIssue,
  normalizeDictationSettings,
} from "./dictation-settings.ts";

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

test("presets select independent speech endpoints while custom preserves its configuration", () => {
  const configured = {
    ...defaultDictationSettings(), apiKey: "test-speech-key", apiKeyEnv: "SPEECH_API_KEY", language: "en", timeoutSeconds: 60,
  };
  for (const preset of ["groq", "openai", "local"] as const) {
    const next = applyDictationPreset(configured, preset);
    assert.equal(dictationPreset(next), preset);
    assert.equal(next.language, "en");
    assert.equal(next.timeoutSeconds, 60);
    assert.equal(next.enabled, false);
    assert.equal(next.apiKey, "test-speech-key");
    assert.equal(next.apiKeyEnv, "SPEECH_API_KEY");
  }
  const custom = { ...configured, baseUrl: "http://localhost:9090/asr/v1", model: "local-whisper" };
  assert.equal(dictationPreset(custom), "custom");
  assert.deepEqual(applyDictationPreset(custom, "custom"), custom);
});

test("enabled settings validate endpoints and environment names without requiring local credentials", () => {
  const local = { ...applyDictationPreset(defaultDictationSettings(), "local"), enabled: true };
  assert.equal(dictationSettingsIssue(local), null);
  for (const baseUrl of ["", "file:///tmp/asr", "https://user:pass@example.com/v1", "https://example.com/v1?key=x", "https://example.com/v1#fragment"]) {
    assert.equal(dictationSettingsIssue({ ...local, baseUrl }), "baseUrl");
  }
  assert.equal(dictationSettingsIssue({ ...local, model: " " }), "model");
  assert.equal(dictationSettingsIssue({ ...local, apiKeyEnv: "1INVALID" }), "apiKeyEnv");
  assert.equal(dictationSettingsIssue({ ...local, apiKeyEnv: "ASR_API_KEY" }), null);
  assert.equal(dictationSettingsIssue({ ...local, enabled: false, baseUrl: "" }), null);
});

test("timeout normalization remains finite and within the backend bounds", () => {
  assert.equal(normalizeDictationSettings({ timeoutSeconds: NaN }).timeoutSeconds, 120);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: Infinity }).timeoutSeconds, 120);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: 0 }).timeoutSeconds, 1);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: 700 }).timeoutSeconds, 600);
  assert.equal(normalizeDictationSettings({ timeoutSeconds: 90.6 }).timeoutSeconds, 91);
});
