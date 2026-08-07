import assert from "node:assert/strict";
import test from "node:test";

import {
  DEEPSEEK_MODEL_LIST,
  DEEPSEEK_OFFICIAL_BASE_URL,
  DEEPSEEK_PROFILE_ID,
  buildDeepSeekProfilePatch,
  createDeepSeekProfileBase,
  upsertDeepSeekProfile,
} from "./deepseek-onboarding.ts";

test("buildDeepSeekProfilePatch sets official responses fields", () => {
  const patch = buildDeepSeekProfilePatch("deepseek-v4-pro", "  sk-test  ");
  assert.equal(patch.protocol, "responses");
  assert.equal(patch.relayMode, "pureApi");
  assert.equal(patch.baseUrl, DEEPSEEK_OFFICIAL_BASE_URL);
  assert.equal(patch.upstreamBaseUrl, DEEPSEEK_OFFICIAL_BASE_URL);
  assert.equal(patch.model, "deepseek-v4-pro");
  assert.equal(patch.testModel, "deepseek-v4-pro");
  assert.equal(patch.modelList, DEEPSEEK_MODEL_LIST);
  // 窗口交给 catalog，避免顶层窗口键覆盖官方元数据。
  assert.equal(patch.contextWindow, "");
  assert.equal(patch.autoCompactLimit, "");
  assert.equal(patch.deepseekOfficialMetadata, true);
  assert.equal(patch.apiKey, "sk-test");
});

test("createDeepSeekProfileBase defaults to flash with official metadata", () => {
  const base = createDeepSeekProfileBase();
  assert.equal(base.id, DEEPSEEK_PROFILE_ID);
  assert.equal(base.model, "deepseek-v4-flash");
  assert.equal(base.protocol, "responses");
  assert.equal(base.relayMode, "pureApi");
  assert.equal(base.deepseekOfficialMetadata, true);
  assert.equal(base.contextWindow, "");
  assert.equal(base.autoCompactLimit, "");
});

test("upsertDeepSeekProfile replaces an existing deepseek profile and activates it", () => {
  const settings = {
    relayProfiles: [
      { id: "other", name: "Other" },
      { id: DEEPSEEK_PROFILE_ID, name: "Old DeepSeek" },
    ],
    activeRelayId: "other",
  };
  const profile = createDeepSeekProfileBase();
  const next = upsertDeepSeekProfile(settings, { ...profile, name: "DeepSeek" });
  assert.equal(next.relayProfiles.length, 2);
  assert.equal(next.relayProfiles[1].name, "DeepSeek");
  assert.equal(next.activeRelayId, DEEPSEEK_PROFILE_ID);
});

test("upsertDeepSeekProfile appends when the profile is missing", () => {
  const settings = {
    relayProfiles: [{ id: "other", name: "Other" }],
    activeRelayId: "other",
  };
  const profile = createDeepSeekProfileBase();
  const next = upsertDeepSeekProfile(settings, profile);
  assert.equal(next.relayProfiles.length, 2);
  assert.equal(next.relayProfiles[1].id, DEEPSEEK_PROFILE_ID);
  assert.equal(next.activeRelayId, DEEPSEEK_PROFILE_ID);
});
