import assert from "node:assert/strict";
import test from "node:test";

import { persistRelayProfileDraft } from "./relay-profile-save.ts";

test("active relay profiles are applied through the transactional switch path", async () => {
  const calls: string[] = [];
  await persistRelayProfileDraft({
    next: { id: "deepseek" },
    shouldApplyActiveProfile: true,
    applyActiveProfile: async () => {
      calls.push("apply");
    },
    saveSettings: async () => {
      calls.push("save");
    },
  });
  assert.deepEqual(calls, ["apply"]);
});

test("inactive relay profiles only update saved settings", async () => {
  const calls: string[] = [];
  await persistRelayProfileDraft({
    next: { id: "standard" },
    shouldApplyActiveProfile: false,
    applyActiveProfile: async () => {
      calls.push("apply");
    },
    saveSettings: async () => {
      calls.push("save");
    },
  });
  assert.deepEqual(calls, ["save"]);
});
