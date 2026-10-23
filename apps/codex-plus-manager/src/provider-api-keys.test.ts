import assert from "node:assert/strict";
import { test } from "node:test";

import { apiKeyPreview, normalizeRelayApiKeys, relayProfileWithNormalizedApiKeys } from "./provider-api-keys.ts";
import type { RelayProfile } from "./App.tsx";

test("旧单 Key 自动迁移为默认命名 Key", () => {
  assert.deepEqual(normalizeRelayApiKeys({ apiKey: "sk-legacy" }), {
    apiKeys: [{ id: "default", name: "默认", apiKey: "sk-legacy" }],
    activeApiKeyId: "default",
  });
});

test("空配置也提供一个可直接编辑的默认 Key", () => {
  assert.deepEqual(normalizeRelayApiKeys({ apiKey: "" }), {
    apiKeys: [{ id: "default", name: "默认", apiKey: "" }],
    activeApiKeyId: "default",
  });
});

test("命名 Key 去重并回退到首个有效选项", () => {
  const normalized = normalizeRelayApiKeys({
    apiKey: "old",
    activeApiKeyId: "missing",
    apiKeys: [
      { id: "same", name: "主账号", apiKey: "sk-a" },
      { id: "same", name: "", apiKey: "sk-b" },
    ],
  });
  assert.deepEqual(normalized, {
    apiKeys: [
      { id: "same", name: "主账号", apiKey: "sk-a" },
      { id: "same-2", name: "Key 2", apiKey: "sk-b" },
    ],
    activeApiKeyId: "same",
  });
});

test("当前命名 Key 始终决定运行时 apiKey", () => {
  const profile = {
    apiKey: "stale",
    activeApiKeyId: "backup",
    apiKeys: [
      { id: "main", name: "主账号", apiKey: "sk-main" },
      { id: "backup", name: "备用", apiKey: "sk-backup" },
    ],
  } as RelayProfile;
  assert.equal(relayProfileWithNormalizedApiKeys(profile).apiKey, "sk-backup");
});

test("Key 预览只露头尾各 6 位，短 Key 不预览（#2656）", () => {
  assert.equal(apiKeyPreview("sk-proj-abcdefghijklmnop-123456"), "sk-pro…123456");
  assert.equal(apiKeyPreview("  sk-proj-abcdefghijklmnop-123456  "), "sk-pro…123456");
  assert.equal(apiKeyPreview("sk-short-key-0001"), "sk-sho…y-0001");
  assert.equal(apiKeyPreview("sk-1234567890ab"), "", "露头尾等于露全文的短 Key 不给预览");
  assert.equal(apiKeyPreview(""), "");
});
